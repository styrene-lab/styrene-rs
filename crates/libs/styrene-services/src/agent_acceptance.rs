//! Durable acceptance ledger for canonical A2A envelopes.
//!
//! Bearers provide at-least-once delivery. This store provides idempotent daemon
//! acceptance by reserving each message ID before effects execute and rejecting a
//! conflicting body that reuses an accepted ID.

use std::sync::Mutex;

use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use styrene_a2a::{AgentEnvelope, GraphEdgeRelationship};

use crate::ServiceError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceptanceOutcome {
    Accepted { message_id: [u8; 16] },
    Duplicate { message_id: [u8; 16] },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectAcceptanceOutcome {
    Accepted { message_id: [u8; 16], event_sequence: u64 },
    Duplicate { message_id: [u8; 16], event_sequence: u64 },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentDomainEffect {
    pub task_id: String,
    pub task_state: String,
    pub agent_id: String,
    pub effective_grant_reference: Option<String>,
    pub parent_task_id: Option<String>,
    pub relationship: Option<GraphEdgeRelationship>,
    pub event_kind: String,
    pub event_payload: Vec<u8>,
}

pub struct AgentAcceptanceStore {
    conn: Mutex<Connection>,
}

impl AgentAcceptanceStore {
    pub fn open(path: &str) -> Result<Self, ServiceError> {
        let store = Self { conn: Mutex::new(Connection::open(path)?) };
        store.migrate()?;
        Ok(store)
    }

    pub fn in_memory() -> Result<Self, ServiceError> {
        let store = Self { conn: Mutex::new(Connection::open_in_memory()?) };
        store.migrate()?;
        Ok(store)
    }

    fn migrate(&self) -> Result<(), ServiceError> {
        let conn = self.conn.lock().map_err(|error| ServiceError::Storage(error.to_string()))?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS agent_acceptance_ledger (
                message_id BLOB PRIMARY KEY CHECK(length(message_id) = 16),
                envelope_digest BLOB NOT NULL CHECK(length(envelope_digest) = 32),
                source_agent_id TEXT NOT NULL,
                source_runtime_id BLOB NOT NULL CHECK(length(source_runtime_id) = 16),
                root_operation_id TEXT NOT NULL,
                stream_id TEXT NOT NULL,
                sequence INTEGER NOT NULL,
                accepted_at_ms INTEGER NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_agent_acceptance_source_stream
              ON agent_acceptance_ledger(source_runtime_id, stream_id, sequence);
            CREATE TABLE IF NOT EXISTS agent_tasks (
                task_id TEXT PRIMARY KEY,
                root_operation_id TEXT NOT NULL,
                agent_id TEXT NOT NULL,
                task_state TEXT NOT NULL,
                effective_grant_reference TEXT,
                updated_at_ms INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS agent_graph_edges (
                parent_task_id TEXT NOT NULL,
                child_task_id TEXT NOT NULL UNIQUE,
                relationship TEXT NOT NULL,
                root_operation_id TEXT NOT NULL,
                PRIMARY KEY(parent_task_id, child_task_id)
            );
            CREATE TABLE IF NOT EXISTS agent_sequence_watermarks (
                source_runtime_id BLOB NOT NULL CHECK(length(source_runtime_id) = 16),
                stream_id TEXT NOT NULL,
                contiguous_through INTEGER NOT NULL,
                highest_observed INTEGER NOT NULL,
                PRIMARY KEY(source_runtime_id, stream_id)
            );
            CREATE TABLE IF NOT EXISTS agent_outbound_events (
                event_sequence INTEGER PRIMARY KEY AUTOINCREMENT,
                message_id BLOB NOT NULL UNIQUE CHECK(length(message_id) = 16),
                root_operation_id TEXT NOT NULL,
                task_id TEXT NOT NULL,
                event_kind TEXT NOT NULL,
                event_payload BLOB NOT NULL,
                created_at_ms INTEGER NOT NULL
            );",
        )?;
        Ok(())
    }

    /// Atomically reserve an envelope's message ID before applying domain effects.
    ///
    /// The caller MUST execute effects only for [`AcceptanceOutcome::Accepted`].
    /// A duplicate with identical canonical bytes is harmless. Reuse of the same
    /// message ID for different bytes is rejected as a protocol conflict.
    pub fn accept(
        &self,
        envelope: &AgentEnvelope,
        now_ms: u64,
    ) -> Result<AcceptanceOutcome, ServiceError> {
        envelope
            .validate(now_ms)
            .map_err(|error| ServiceError::InvalidArgument(error.to_string()))?;
        let encoded = envelope
            .encode_cbor()
            .map_err(|error| ServiceError::InvalidArgument(error.to_string()))?;
        let digest = sha256(&encoded);
        let mut conn =
            self.conn.lock().map_err(|error| ServiceError::Storage(error.to_string()))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<Vec<u8>> = tx
            .query_row(
                "SELECT envelope_digest FROM agent_acceptance_ledger WHERE message_id = ?1",
                params![envelope.message_id.as_slice()],
                |row| row.get(0),
            )
            .optional()?;
        if let Some(existing) = existing {
            if existing.as_slice() == digest {
                tx.commit()?;
                return Ok(AcceptanceOutcome::Duplicate { message_id: envelope.message_id });
            }
            return Err(ServiceError::InvalidArgument(
                "agent message id was reused with conflicting envelope bytes".into(),
            ));
        }
        tx.execute(
            "INSERT INTO agent_acceptance_ledger
             (message_id, envelope_digest, source_agent_id, source_runtime_id,
              root_operation_id, stream_id, sequence, accepted_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![
                envelope.message_id.as_slice(),
                digest.as_slice(),
                envelope.source_agent_id,
                envelope.source_runtime_id.as_slice(),
                envelope.root_operation_id,
                envelope.stream_id,
                envelope.sequence,
                now_ms,
            ],
        )?;
        tx.commit()?;
        Ok(AcceptanceOutcome::Accepted { message_id: envelope.message_id })
    }

    pub fn accept_with_effect(
        &self,
        envelope: &AgentEnvelope,
        effect: &AgentDomainEffect,
        now_ms: u64,
    ) -> Result<EffectAcceptanceOutcome, ServiceError> {
        envelope
            .validate(now_ms)
            .map_err(|error| ServiceError::InvalidArgument(error.to_string()))?;
        if envelope.task_id.as_deref() != Some(effect.task_id.as_str())
            || effect.task_id.trim().is_empty()
            || effect.task_state.trim().is_empty()
            || effect.agent_id.trim().is_empty()
            || effect.event_kind.trim().is_empty()
        {
            return Err(ServiceError::InvalidArgument(
                "agent effect does not match envelope task".into(),
            ));
        }
        if effect.parent_task_id.is_some() != effect.relationship.is_some() {
            return Err(ServiceError::InvalidArgument(
                "graph parent and relationship must be supplied together".into(),
            ));
        }
        let encoded = envelope
            .encode_cbor()
            .map_err(|error| ServiceError::InvalidArgument(error.to_string()))?;
        let digest = sha256(&encoded);
        let mut conn =
            self.conn.lock().map_err(|error| ServiceError::Storage(error.to_string()))?;
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<(Vec<u8>, i64)> = tx.query_row(
            "SELECT l.envelope_digest, e.event_sequence FROM agent_acceptance_ledger l JOIN agent_outbound_events e USING(message_id) WHERE l.message_id = ?1",
            params![envelope.message_id.as_slice()],
            |row| Ok((row.get(0)?, row.get(1)?)),
        ).optional()?;
        if let Some((existing_digest, event_sequence)) = existing {
            if existing_digest.as_slice() != digest {
                return Err(ServiceError::InvalidArgument(
                    "agent message id was reused with conflicting envelope bytes".into(),
                ));
            }
            tx.commit()?;
            return Ok(EffectAcceptanceOutcome::Duplicate {
                message_id: envelope.message_id,
                event_sequence: event_sequence as u64,
            });
        }
        insert_acceptance(&tx, envelope, &digest, now_ms)?;
        tx.execute(
            "INSERT INTO agent_tasks(task_id, root_operation_id, agent_id, task_state, effective_grant_reference, updated_at_ms)
             VALUES(?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(task_id) DO UPDATE SET agent_id=excluded.agent_id, task_state=excluded.task_state,
             effective_grant_reference=excluded.effective_grant_reference, updated_at_ms=excluded.updated_at_ms",
            params![effect.task_id, envelope.root_operation_id, effect.agent_id, effect.task_state, effect.effective_grant_reference, now_ms],
        )?;
        if let (Some(parent), Some(relationship)) = (&effect.parent_task_id, effect.relationship) {
            tx.execute(
                "INSERT INTO agent_graph_edges(parent_task_id, child_task_id, relationship, root_operation_id) VALUES(?1, ?2, ?3, ?4)
                 ON CONFLICT(child_task_id) DO UPDATE SET parent_task_id=excluded.parent_task_id, relationship=excluded.relationship, root_operation_id=excluded.root_operation_id",
                params![parent, effect.task_id, relationship_name(relationship), envelope.root_operation_id],
            )?;
        }
        tx.execute(
            "INSERT INTO agent_sequence_watermarks(source_runtime_id, stream_id, contiguous_through, highest_observed)
             VALUES(?1, ?2, ?3, ?3)
             ON CONFLICT(source_runtime_id, stream_id) DO UPDATE SET
             contiguous_through = CASE WHEN excluded.contiguous_through = highest_observed + 1 THEN excluded.contiguous_through ELSE contiguous_through END,
             highest_observed = MAX(highest_observed, excluded.highest_observed)",
            params![envelope.source_runtime_id.as_slice(), envelope.stream_id, envelope.sequence],
        )?;
        tx.execute(
            "INSERT INTO agent_outbound_events(message_id, root_operation_id, task_id, event_kind, event_payload, created_at_ms) VALUES(?1, ?2, ?3, ?4, ?5, ?6)",
            params![envelope.message_id.as_slice(), envelope.root_operation_id, effect.task_id, effect.event_kind, effect.event_payload, now_ms],
        )?;
        let event_sequence = tx.last_insert_rowid() as u64;
        tx.commit()?;
        Ok(EffectAcceptanceOutcome::Accepted { message_id: envelope.message_id, event_sequence })
    }

    pub fn contains(&self, message_id: &[u8; 16]) -> Result<bool, ServiceError> {
        let conn = self.conn.lock().map_err(|error| ServiceError::Storage(error.to_string()))?;
        Ok(conn
            .query_row(
                "SELECT 1 FROM agent_acceptance_ledger WHERE message_id = ?1",
                params![message_id.as_slice()],
                |_| Ok(()),
            )
            .optional()?
            .is_some())
    }
}

fn insert_acceptance(
    tx: &rusqlite::Transaction<'_>,
    envelope: &AgentEnvelope,
    digest: &[u8; 32],
    now_ms: u64,
) -> rusqlite::Result<()> {
    tx.execute(
        "INSERT INTO agent_acceptance_ledger
         (message_id, envelope_digest, source_agent_id, source_runtime_id, root_operation_id, stream_id, sequence, accepted_at_ms)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![envelope.message_id.as_slice(), digest.as_slice(), envelope.source_agent_id,
            envelope.source_runtime_id.as_slice(), envelope.root_operation_id, envelope.stream_id,
            envelope.sequence, now_ms],
    )?;
    Ok(())
}

fn relationship_name(relationship: GraphEdgeRelationship) -> &'static str {
    match relationship {
        GraphEdgeRelationship::Delegate => "delegate",
        GraphEdgeRelationship::CleaveChild => "cleave_child",
        GraphEdgeRelationship::Handoff => "handoff",
        GraphEdgeRelationship::OperatorAttach => "operator_attach",
    }
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use styrene_a2a::{AgentEnvelopeKind, AgentId, RootOperationId, RuntimeId};

    fn envelope() -> AgentEnvelope {
        let mut envelope = AgentEnvelope::new(
            AgentEnvelopeKind::Command,
            &AgentId::new("source").unwrap(),
            RuntimeId::new(),
            &AgentId::new("target").unwrap(),
            &RootOperationId::new("root").unwrap(),
            Some("task".into()),
            "task",
            1,
            10,
            "a2a.message.v1",
            br#"{"role":"user"}"#.to_vec(),
        );
        envelope.expires_at_ms = Some(1000);
        envelope
    }

    #[test]
    fn duplicate_delivery_has_one_accepted_effect() {
        let store = AgentAcceptanceStore::in_memory().unwrap();
        let envelope = envelope();
        assert!(matches!(store.accept(&envelope, 20).unwrap(), AcceptanceOutcome::Accepted { .. }));
        assert!(matches!(
            store.accept(&envelope, 20).unwrap(),
            AcceptanceOutcome::Duplicate { .. }
        ));
        assert!(store.contains(&envelope.message_id).unwrap());
    }

    #[test]
    fn acceptance_and_domain_effect_commit_once() {
        let store = AgentAcceptanceStore::in_memory().unwrap();
        let envelope = envelope();
        let effect = AgentDomainEffect {
            task_id: "task".into(),
            task_state: "working".into(),
            agent_id: "source".into(),
            effective_grant_reference: None,
            parent_task_id: None,
            relationship: None,
            event_kind: "task_updated".into(),
            event_payload: b"event".to_vec(),
        };
        let accepted = store.accept_with_effect(&envelope, &effect, 20).unwrap();
        let duplicate = store.accept_with_effect(&envelope, &effect, 20).unwrap();
        let sequence = match accepted {
            EffectAcceptanceOutcome::Accepted { event_sequence, .. } => event_sequence,
            _ => panic!(),
        };
        assert_eq!(
            duplicate,
            EffectAcceptanceOutcome::Duplicate {
                message_id: envelope.message_id,
                event_sequence: sequence
            }
        );
        let conn = store.conn.lock().unwrap();
        assert_eq!(
            conn.query_row("SELECT count(*) FROM agent_tasks", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT count(*) FROM agent_outbound_events", [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            1
        );
        assert_eq!(
            conn.query_row("SELECT contiguous_through FROM agent_sequence_watermarks", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap(),
            1
        );
    }

    #[test]
    fn invalid_effect_rolls_back_acceptance() {
        let store = AgentAcceptanceStore::in_memory().unwrap();
        let envelope = envelope();
        let effect = AgentDomainEffect {
            task_id: "other".into(),
            task_state: "working".into(),
            agent_id: "source".into(),
            effective_grant_reference: None,
            parent_task_id: None,
            relationship: None,
            event_kind: "task_updated".into(),
            event_payload: Vec::new(),
        };
        assert!(store.accept_with_effect(&envelope, &effect, 20).is_err());
        assert!(!store.contains(&envelope.message_id).unwrap());
    }

    #[test]
    fn conflicting_message_id_is_rejected() {
        let store = AgentAcceptanceStore::in_memory().unwrap();
        let original = envelope();
        store.accept(&original, 20).unwrap();
        let mut conflict = original.clone();
        conflict.a2a_payload = br#"{"role":"user","changed":true}"#.to_vec();
        conflict.payload_digest = conflict.computed_payload_digest();
        assert!(matches!(store.accept(&conflict, 20), Err(ServiceError::InvalidArgument(_))));
    }
}
