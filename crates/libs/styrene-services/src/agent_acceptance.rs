//! Durable acceptance ledger for canonical A2A envelopes.
//!
//! Bearers provide at-least-once delivery. This store provides idempotent daemon
//! acceptance by reserving each message ID before effects execute and rejecting a
//! conflicting body that reuses an accepted ID.

use std::sync::Mutex;

use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use styrene_a2a::AgentEnvelope;

use crate::ServiceError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AcceptanceOutcome {
    Accepted { message_id: [u8; 16] },
    Duplicate { message_id: [u8; 16] },
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
              ON agent_acceptance_ledger(source_runtime_id, stream_id, sequence);",
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
