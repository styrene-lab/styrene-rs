use rusqlite::{OptionalExtension, params};

use super::messages::{
    AttachmentBlobInput, CanonicalInboundRecord, LxmfTicketRecord, MessageRecord, MessagesStore,
    stage_attachment_blobs,
};

pub const STAMP_COST_EXPIRY_SECS: i64 = 45 * 24 * 60 * 60;
pub const TICKET_INTERVAL_SECS: i64 = 24 * 60 * 60;
pub const TICKET_RESERVATION_TTL_SECS: i64 = 5 * 60;
type InboundPropagationCorrelation = ([u8; 32], [u8; 16], [u8; 16], i64);

impl MessagesStore {
    pub fn insert_canonical_with_verified_ticket(
        &self,
        projection: &MessageRecord,
        canonical: &CanonicalInboundRecord,
        received_ticket: Option<&LxmfTicketRecord>,
    ) -> rusqlite::Result<bool> {
        self.insert_canonical_with_attachments_and_ticket(
            projection,
            canonical,
            received_ticket,
            &[],
            None,
        )
    }

    pub fn insert_canonical_with_attachments_and_ticket(
        &self,
        projection: &MessageRecord,
        canonical: &CanonicalInboundRecord,
        received_ticket: Option<&LxmfTicketRecord>,
        attachments: &[AttachmentBlobInput],
        attachment_issue: Option<&str>,
    ) -> rusqlite::Result<bool> {
        self.insert_canonical_with_attachments_ticket_and_transfer(
            projection,
            canonical,
            received_ticket,
            attachments,
            attachment_issue,
            None,
        )
    }

    pub fn insert_canonical_with_attachments_ticket_and_transfer(
        &self,
        projection: &MessageRecord,
        canonical: &CanonicalInboundRecord,
        received_ticket: Option<&LxmfTicketRecord>,
        attachments: &[AttachmentBlobInput],
        attachment_issue: Option<&str>,
        transfer: Option<super::messages::InboundAttachmentTransferEvidence>,
    ) -> rusqlite::Result<bool> {
        self.insert_canonical_with_attachments_ticket_transfer_and_propagation(
            projection,
            canonical,
            received_ticket,
            attachments,
            attachment_issue,
            transfer,
            None,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn insert_canonical_with_attachments_ticket_transfer_and_propagation(
        &self,
        projection: &MessageRecord,
        canonical: &CanonicalInboundRecord,
        received_ticket: Option<&LxmfTicketRecord>,
        attachments: &[AttachmentBlobInput],
        attachment_issue: Option<&str>,
        transfer: Option<super::messages::InboundAttachmentTransferEvidence>,
        propagation: Option<InboundPropagationCorrelation>,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(
            "INSERT OR IGNORE INTO messages
             (id, source, destination, title, content, timestamp, direction, fields,
              receipt_status, read)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'in', ?7, NULL, 0)",
            params![
                &projection.id,
                &projection.source,
                &projection.destination,
                &projection.title,
                &projection.content,
                projection.timestamp,
                projection
                    .fields
                    .as_ref()
                    .map(|value| serde_json::to_string(value).unwrap_or_default()),
            ],
        )?;
        if changed == 0 {
            if let Some((transient_id, attempt_id, peer, now)) = propagation {
                super::standard_propagation::link_inbound_in_transaction(
                    &transaction,
                    &projection.id,
                    transient_id,
                    attempt_id,
                    peer,
                    now,
                )?;
                transaction.commit()?;
            }
            return Ok(false);
        }
        transaction.execute(
            "INSERT INTO canonical_inbound_messages
             (message_id, source, destination, title, content, timestamp, fields_msgpack,
               signature, stamp, wire, authentication_state, stamp_state, stamp_value)
               VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)",
            params![
                &canonical.message_id,
                canonical.source.as_slice(),
                canonical.destination.as_slice(),
                &canonical.title,
                &canonical.content,
                canonical.timestamp,
                &canonical.fields_msgpack,
                &canonical.signature,
                &canonical.stamp,
                &canonical.wire,
                &canonical.authentication_state,
                &canonical.stamp_state,
                canonical.stamp_value,
            ],
        )?;
        transaction.execute(
            "INSERT INTO canonical_inbound_inspection (message_id, stamp_target) VALUES (?1, ?2)",
            params![&canonical.message_id, canonical.stamp_target],
        )?;
        if canonical.authentication_state == "verified"
            && let Some(ticket) = received_ticket
        {
            validate_ticket(ticket)?;
            transaction.execute(
                "INSERT INTO lxmf_tickets (peer, ticket, expires_at, direction)
                     VALUES (?1, ?2, ?3, 'received')
                     ON CONFLICT(peer, direction, ticket)
                     DO UPDATE SET expires_at = excluded.expires_at",
                params![&ticket.peer, &ticket.ticket, ticket.expires_at],
            )?;
        }
        if let Some(issue) = attachment_issue {
            transaction.execute(
                "INSERT INTO attachment_issues (message_id, reason, created_at)
                 VALUES (?1, ?2, CAST(strftime('%s','now') AS INTEGER))",
                params![&projection.id, issue],
            )?;
        } else {
            stage_attachment_blobs(
                &transaction,
                &projection.id,
                attachments,
                projection.timestamp.max(0),
            )?;
            if !attachments.is_empty() {
                let total = transfer.map_or_else(
                    || i64::try_from(canonical.wire.len()).unwrap_or(i64::MAX),
                    |value| i64::try_from(value.total).unwrap_or(i64::MAX),
                );
                let transferred = transfer.map_or(total, |value| {
                    i64::try_from(value.transferred).unwrap_or(i64::MAX).min(total)
                });
                let representation = if transfer.is_some() {
                    "resource"
                } else if canonical.wire.len() <= rns_core::transport::resource::LINK_PACKET_MDU {
                    "packet"
                } else {
                    "resource"
                };
                let resource_hash = transfer.map(|value| value.resource_hash);
                let checksum_verified =
                    i64::from(transfer.is_none_or(|value| value.checksum_verified));
                transaction.execute(
                    "INSERT INTO attachment_transfers
                     (message_id, transfer_id, resource_hash, representation, direction, state, transferred,
                      total, checksum_verified, error, updated_at)
                     VALUES (?1, ?1, ?2, ?3, 'inbound', 'completed', ?4, ?5, ?6, NULL, ?7)",
                    params![
                        &projection.id,
                        resource_hash.as_ref().map(<[u8; 32]>::as_slice),
                        representation,
                        transferred,
                        total,
                        checksum_verified,
                        projection.timestamp.max(0)
                    ],
                )?;
                if let Some(resource_hash) = resource_hash {
                    transaction.execute(
                        "UPDATE message_attachments SET resource_hash = ?2, transfer_id = ?1
                         WHERE message_id = ?1",
                        params![&projection.id, resource_hash.as_slice()],
                    )?;
                }
            }
        }
        if let Some((transient_id, attempt_id, peer, now)) = propagation {
            super::standard_propagation::link_inbound_in_transaction(
                &transaction,
                &projection.id,
                transient_id,
                attempt_id,
                peer,
                now,
            )?;
        }
        transaction.commit()?;
        Ok(true)
    }

    pub fn update_unknown_auth_with_verified_ticket(
        &self,
        message_id: &str,
        state: &str,
        ticket: Option<&LxmfTicketRecord>,
    ) -> rusqlite::Result<bool> {
        if !matches!(state, "verified" | "invalid") {
            return Ok(false);
        }
        let transaction = self.conn.unchecked_transaction()?;
        let changed = transaction.execute(
            "UPDATE canonical_inbound_messages SET authentication_state = ?2
             WHERE message_id = ?1 AND authentication_state = 'unknown_identity'",
            params![message_id, state],
        )?;
        if changed > 0
            && state == "verified"
            && let Some(ticket) = ticket
        {
            validate_ticket(ticket)?;
            transaction.execute(
                "INSERT INTO lxmf_tickets (peer, ticket, expires_at, direction)
                     VALUES (?1, ?2, ?3, 'received')
                     ON CONFLICT(peer, direction, ticket)
                     DO UPDATE SET expires_at = excluded.expires_at",
                params![&ticket.peer, &ticket.ticket, ticket.expires_at],
            )?;
        }
        transaction.commit()?;
        Ok(changed > 0)
    }

    pub fn peer_stamp_cost_at(&self, peer: &str, now: i64) -> rusqlite::Result<Option<u32>> {
        self.conn
            .query_row(
                "SELECT stamp_cost FROM lxmf_peer_costs
                 WHERE peer = ?1 AND observed_at + ?2 >= ?3",
                params![peer, STAMP_COST_EXPIRY_SECS, now],
                |row| row.get(0),
            )
            .optional()
    }

    pub fn ticket_offer_due_at(&self, peer: &str, now: i64) -> rusqlite::Result<bool> {
        self.conn.query_row(
            "SELECT NOT EXISTS (
                 SELECT 1 FROM lxmf_ticket_deliveries
                 WHERE peer = ?1 AND last_delivered_at > ?2 - ?3
             ) AND NOT EXISTS (
                 SELECT 1 FROM lxmf_ticket_offer_reservations
                 WHERE peer = ?1 AND reserved_at > ?2 - ?4
             ) AND NOT EXISTS (
                 SELECT 1 FROM outbound_ticket_offers o
                 JOIN outbound_routes r ON r.message_id = o.message_id
                 WHERE o.peer = ?1 AND o.delivered_at IS NULL
                   AND r.state NOT IN ('delivered','failed','cancelled','expired','rejected')
             )",
            params![peer, now, TICKET_INTERVAL_SECS, TICKET_RESERVATION_TTL_SECS],
            |row| row.get(0),
        )
    }

    pub fn reserve_ticket_offer(
        &self,
        peer: &str,
        reservation_id: &str,
        now: i64,
    ) -> rusqlite::Result<bool> {
        if peer.len() > 128 || reservation_id.len() > 128 {
            return Err(rusqlite::Error::InvalidParameterName(
                "invalid LXMF ticket offer reservation".into(),
            ));
        }
        let transaction = self.conn.unchecked_transaction()?;
        transaction.execute(
            "DELETE FROM lxmf_ticket_offer_reservations WHERE reserved_at <= ?1 - ?2",
            params![now, TICKET_RESERVATION_TTL_SECS],
        )?;
        let changed = transaction.execute(
            "INSERT OR IGNORE INTO lxmf_ticket_offer_reservations
                 (peer, reservation_id, reserved_at)
             SELECT ?1, ?2, ?3
             WHERE NOT EXISTS (
                 SELECT 1 FROM lxmf_ticket_deliveries
                 WHERE peer = ?1 AND last_delivered_at > ?3 - ?4
             ) AND NOT EXISTS (
                 SELECT 1 FROM outbound_ticket_offers o
                 JOIN outbound_routes r ON r.message_id = o.message_id
                 WHERE o.peer = ?1 AND o.delivered_at IS NULL
                   AND r.state NOT IN ('delivered','failed','cancelled','expired','rejected')
             )",
            params![peer, reservation_id, now, TICKET_INTERVAL_SECS],
        )?;
        transaction.commit()?;
        Ok(changed == 1)
    }

    pub fn release_ticket_offer_reservation(
        &self,
        peer: &str,
        reservation_id: &str,
    ) -> rusqlite::Result<bool> {
        Ok(self.conn.execute(
            "DELETE FROM lxmf_ticket_offer_reservations
             WHERE peer = ?1 AND reservation_id = ?2",
            params![peer, reservation_id],
        )? == 1)
    }

    pub fn track_ticket_offer(
        &self,
        message_id: &str,
        ticket: &LxmfTicketRecord,
    ) -> rusqlite::Result<()> {
        validate_ticket(ticket)?;
        self.conn.execute(
            "INSERT OR IGNORE INTO outbound_ticket_offers
             (message_id, peer, ticket, expires_at, delivered_at)
             VALUES (?1, ?2, ?3, ?4, NULL)",
            params![message_id, &ticket.peer, &ticket.ticket, ticket.expires_at],
        )?;
        Ok(())
    }

    pub fn mark_ticket_offer_delivered(
        &self,
        message_id: &str,
        delivered_at: i64,
    ) -> rusqlite::Result<bool> {
        let transaction = self.conn.unchecked_transaction()?;
        let peer: Option<String> = transaction
            .query_row(
                "SELECT peer FROM outbound_ticket_offers
                 WHERE message_id = ?1 AND delivered_at IS NULL",
                params![message_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(peer) = peer else {
            return Ok(false);
        };
        transaction.execute(
            "UPDATE outbound_ticket_offers SET delivered_at = ?2
             WHERE message_id = ?1 AND delivered_at IS NULL",
            params![message_id, delivered_at],
        )?;
        transaction.execute(
            "INSERT INTO lxmf_ticket_deliveries (peer, last_delivered_at) VALUES (?1, ?2)
             ON CONFLICT(peer) DO UPDATE SET
                 last_delivered_at = MAX(last_delivered_at, excluded.last_delivered_at)",
            params![peer, delivered_at],
        )?;
        transaction.commit()?;
        Ok(true)
    }

    pub fn repair_ticket_offer_deliveries(&self, repaired_at: i64) -> rusqlite::Result<usize> {
        let mut repaired = 0usize;
        loop {
            let ids: Vec<String> = {
                let mut statement = self.conn.prepare(
                    "SELECT o.message_id FROM outbound_ticket_offers o
                     JOIN outbound_routes r ON r.message_id = o.message_id
                     WHERE o.delivered_at IS NULL AND r.state = 'delivered'
                     ORDER BY o.message_id LIMIT 1024",
                )?;

                statement.query_map([], |row| row.get(0))?.collect::<rusqlite::Result<_>>()?
            };
            if ids.is_empty() {
                break;
            }
            for id in &ids {
                self.mark_ticket_offer_delivered(id, repaired_at)?;
            }
            repaired = repaired.saturating_add(ids.len());
        }
        Ok(repaired)
    }

    pub fn reconcile_ticket_offer_startup(&self, repaired_at: i64) -> rusqlite::Result<usize> {
        let repaired = self.repair_ticket_offer_deliveries(repaired_at)?;
        self.conn.execute(
            "DELETE FROM lxmf_ticket_offer_reservations WHERE reserved_at <= ?1 - ?2",
            params![repaired_at, TICKET_RESERVATION_TTL_SECS],
        )?;
        Ok(repaired)
    }
}

fn validate_ticket(record: &LxmfTicketRecord) -> rusqlite::Result<()> {
    if record.peer.len() > 128
        || record.ticket.len() != lxmf::stamps::TICKET_LENGTH
        || !matches!(record.direction.as_str(), "issued" | "received")
    {
        return Err(rusqlite::Error::InvalidParameterName("invalid LXMF ticket".into()));
    }
    Ok(())
}
