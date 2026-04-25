//! AutoReplyService — automatic reply with per-peer cooldown.
//!
//! Owns: 13.4 auto-reply with per-peer cooldown tracking, reply composition.
//! Sends through MessagingService, reads from ConfigService.
//! Package: E
//!
//! Standalone service — has behavior beyond config (cooldown tracking,
//! reply composition). Clean dependency: reads config, sends via messaging.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Auto-reply operating mode.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum AutoReplyMode {
    /// Auto-reply is disabled.
    #[default]
    Disabled,
    /// Reply to all incoming messages.
    All,
    /// Reply only to first message from each peer (within cooldown).
    FirstOnly,
}

/// Auto-reply configuration.
#[derive(Debug, Clone)]
pub struct AutoReplyConfig {
    pub mode: AutoReplyMode,
    pub message: String,
    pub cooldown: Duration,
}

impl Default for AutoReplyConfig {
    fn default() -> Self {
        Self {
            mode: AutoReplyMode::Disabled,
            message: String::new(),
            cooldown: Duration::from_secs(300), // 5 minutes
        }
    }
}

/// Service managing auto-reply behavior with per-peer cooldown tracking.
pub struct AutoReplyService {
    config: Mutex<AutoReplyConfig>,
    /// Per-peer cooldown tracking: identity_hash → last reply time.
    cooldowns: Mutex<HashMap<String, Instant>>,
}

impl AutoReplyService {
    pub fn new() -> Self {
        Self {
            config: Mutex::new(AutoReplyConfig::default()),
            cooldowns: Mutex::new(HashMap::new()),
        }
    }

    /// Update the auto-reply configuration.
    pub fn set_config(&self, config: AutoReplyConfig) {
        *self.config.lock().unwrap() = config;
    }

    /// Get the current auto-reply configuration.
    pub fn config(&self) -> AutoReplyConfig {
        self.config.lock().unwrap().clone()
    }

    /// Check if auto-reply is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.lock().unwrap().mode != AutoReplyMode::Disabled
    }

    /// Determine whether we should reply to a message from the given peer.
    ///
    /// Returns `Some(reply_text)` if a reply should be sent, `None` otherwise.
    /// Automatically updates the cooldown tracker on positive decisions.
    pub fn should_reply(&self, peer_identity_hash: &str) -> Option<String> {
        let config = self.config.lock().unwrap().clone();

        match config.mode {
            AutoReplyMode::Disabled => None,
            AutoReplyMode::All => {
                if self.check_and_update_cooldown(peer_identity_hash, config.cooldown) {
                    Some(config.message.clone())
                } else {
                    None
                }
            }
            AutoReplyMode::FirstOnly => {
                if self.check_and_update_cooldown(peer_identity_hash, config.cooldown) {
                    Some(config.message.clone())
                } else {
                    None
                }
            }
        }
    }

    /// Clear all cooldown tracking (e.g., on config change).
    pub fn clear_cooldowns(&self) {
        self.cooldowns.lock().unwrap().clear();
    }

    /// Check cooldown for a peer and update if allowed.
    /// Returns `true` if the cooldown has expired (or no prior entry).
    fn check_and_update_cooldown(&self, peer: &str, cooldown: Duration) -> bool {
        let mut cooldowns = self.cooldowns.lock().unwrap();
        let now = Instant::now();

        if let Some(last) = cooldowns.get(peer) {
            if now.duration_since(*last) < cooldown {
                return false; // still in cooldown
            }
        }

        cooldowns.insert(peer.to_string(), now);
        true
    }
}

impl Default for AutoReplyService {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        let svc = AutoReplyService::new();
        assert!(!svc.is_enabled());
        assert!(svc.should_reply("peer1").is_none());
    }

    #[test]
    fn enabled_replies_with_message() {
        let svc = AutoReplyService::new();
        svc.set_config(AutoReplyConfig {
            mode: AutoReplyMode::All,
            message: "I'm away".into(),
            cooldown: Duration::from_secs(60),
        });
        assert!(svc.is_enabled());
        let reply = svc.should_reply("peer1");
        assert_eq!(reply, Some("I'm away".into()));
    }

    #[test]
    fn cooldown_blocks_rapid_replies() {
        let svc = AutoReplyService::new();
        svc.set_config(AutoReplyConfig {
            mode: AutoReplyMode::All,
            message: "away".into(),
            cooldown: Duration::from_secs(3600), // 1 hour
        });

        // First reply goes through
        assert!(svc.should_reply("peer1").is_some());

        // Second reply blocked by cooldown
        assert!(svc.should_reply("peer1").is_none());

        // Different peer is fine
        assert!(svc.should_reply("peer2").is_some());
    }

    #[test]
    fn clear_cooldowns_allows_re_reply() {
        let svc = AutoReplyService::new();
        svc.set_config(AutoReplyConfig {
            mode: AutoReplyMode::All,
            message: "away".into(),
            cooldown: Duration::from_secs(3600),
        });

        assert!(svc.should_reply("peer1").is_some());
        assert!(svc.should_reply("peer1").is_none());

        svc.clear_cooldowns();
        assert!(svc.should_reply("peer1").is_some());
    }

    #[test]
    fn config_snapshot() {
        let svc = AutoReplyService::new();
        let config = svc.config();
        assert_eq!(config.mode, AutoReplyMode::Disabled);
        assert_eq!(config.cooldown, Duration::from_secs(300));
    }
}
