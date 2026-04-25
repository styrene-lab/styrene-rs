//! SSH agent backed by Styrene Identity.
//!
//! Implements the SSH agent protocol via [`ssh_agent_lib::agent::Session`].
//! Keys are derived in memory from the root secret via HKDF — private key
//! material is never written to disk.
//!
//! ## Architecture
//!
//! ```text
//! SSH client → SSH_AUTH_SOCK → StyreneAgent (this module)
//!   request_identities() → derive public keys for configured labels
//!   sign(pubkey, data)   → match pubkey to label → derive private key → sign → zeroize
//! ```
//!
//! ## Usage
//!
//! ```ignore
//! use styrene_identity::ssh_agent::StyreneAgent;
//! use tokio::net::UnixListener;
//!
//! let agent = StyreneAgent::new(signer, &["github", "work"]);
//! let listener = UnixListener::bind("/tmp/styrene-ssh-agent.sock")?;
//! ssh_agent_lib::agent::listen(listener, agent).await?;
//! ```

use std::collections::HashMap;
use std::sync::Arc;

use ssh_agent_lib::agent::Session;
use ssh_agent_lib::error::AgentError;
use ssh_agent_lib::proto::{Identity, SignRequest};
use ssh_key::public::{Ed25519PublicKey, KeyData};
use ssh_key::{Algorithm, Signature};
use tokio::sync::Mutex;
use zeroize::Zeroize;

use crate::derive::{KeyDeriver, KeyPurpose};
use crate::signer::{IdentitySigner, SignerError};

/// SSH agent session backed by Styrene Identity HKDF derivation.
///
/// Serves multiple key families, all derived from the same root secret:
/// - **SSH user keys** — per-label (e.g., "github", "work") for SSH auth
/// - **Git signing key** — user's personal commit signing key
/// - **Agent signing keys** — per-agent (e.g., "omegon-primary") for agent commits
/// - **SSH host key** — optional, for the machine's SSH server
///
/// Git uses `gpg.format = ssh` to sign commits with these keys. Agent keys
/// allow cryptographic distinction between user-authored and agent-authored
/// commits while all tracing back to the same StyreneID root.
#[derive(Clone)]
pub struct StyreneAgent {
    /// The identity signer providing the root secret.
    signer: Arc<Mutex<Box<dyn IdentitySigner>>>,
    /// Labels for SSH user keys (e.g., "github", "work").
    labels: Vec<String>,
    /// Agent names for agent signing keys (e.g., "omegon-primary", "omegon-cleave-0").
    agent_names: Vec<String>,
    /// Whether to serve the git commit signing key.
    serve_git_signing: bool,
    /// Whether to serve the SSH host key.
    serve_host_key: bool,
}

impl StyreneAgent {
    /// Create an agent serving SSH user keys for the given labels.
    pub fn new(signer: Box<dyn IdentitySigner>, labels: &[&str]) -> Self {
        Self {
            signer: Arc::new(Mutex::new(signer)),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            agent_names: Vec::new(),
            serve_git_signing: false,
            serve_host_key: false,
        }
    }

    /// Also serve the SSH host key.
    pub fn with_host_key(mut self) -> Self {
        self.serve_host_key = true;
        self
    }

    /// Also serve the user's git commit signing key.
    pub fn with_git_signing(mut self) -> Self {
        self.serve_git_signing = true;
        self
    }

    /// Also serve agent-specific signing keys for git commits.
    ///
    /// Each agent name produces a distinct Ed25519 key that can be
    /// registered on GitHub for "Verified" badges on agent commits.
    pub fn with_agent_keys(mut self, agent_names: &[&str]) -> Self {
        self.agent_names = agent_names.iter().map(|s| s.to_string()).collect();
        self
    }

    /// Derive public keys and build a pubkey→key-specifier map.
    /// No private key material is stored — only the information needed
    /// to re-derive the correct seed on a subsequent `sign()` call.
    async fn derive_public_map(
        &self,
    ) -> Result<(Vec<Identity>, HashMap<[u8; 32], KeySpec>), AgentError> {
        let signer = self.signer.lock().await;
        let root = signer
            .root_secret()
            .await
            .map_err(|e| AgentError::Other(Box::new(AgentErrorWrap(e))))?;

        let deriver = KeyDeriver::new(root.as_bytes());
        let mut identities = Vec::new();
        let mut key_map = HashMap::new();

        // SSH user keys (two-level HKDF)
        for label in &self.labels {
            let mut seed =
                deriver.derive_ssh_user_key(label).expect("label validated at config time");
            let vk = crate::pubkey::ed25519_verifying_key(&seed);
            let pubkey_bytes: [u8; 32] = vk.to_bytes();
            seed.zeroize();

            identities.push(Identity {
                pubkey: KeyData::Ed25519(Ed25519PublicKey(pubkey_bytes)),
                comment: format!("styrene-ssh-user-{label}"),
            });
            key_map.insert(pubkey_bytes, KeySpec::SshUser(label.clone()));
        }

        // Git commit signing key (flat purpose)
        if self.serve_git_signing {
            let mut seed = deriver.git_signing_seed();
            let vk = crate::pubkey::ed25519_verifying_key(&seed);
            let pubkey_bytes: [u8; 32] = vk.to_bytes();
            seed.zeroize();

            identities.push(Identity {
                pubkey: KeyData::Ed25519(Ed25519PublicKey(pubkey_bytes)),
                comment: "styrene-git-signing".to_string(),
            });
            key_map.insert(pubkey_bytes, KeySpec::GitSigning);
        }

        // Agent signing keys (two-level HKDF)
        for name in &self.agent_names {
            let mut seed =
                deriver.derive_agent_key(name).expect("agent name validated at config time");
            let vk = crate::pubkey::ed25519_verifying_key(&seed);
            let pubkey_bytes: [u8; 32] = vk.to_bytes();
            seed.zeroize();

            identities.push(Identity {
                pubkey: KeyData::Ed25519(Ed25519PublicKey(pubkey_bytes)),
                comment: format!("styrene-agent:{name}"),
            });
            key_map.insert(pubkey_bytes, KeySpec::Agent(name.clone()));
        }

        // SSH host key (flat purpose)
        if self.serve_host_key {
            let mut seed = deriver.derive(KeyPurpose::SshHost);
            let vk = crate::pubkey::ed25519_verifying_key(&seed);
            let pubkey_bytes: [u8; 32] = vk.to_bytes();
            seed.zeroize();

            identities.push(Identity {
                pubkey: KeyData::Ed25519(Ed25519PublicKey(pubkey_bytes)),
                comment: "styrene-ssh-host".to_string(),
            });
            key_map.insert(pubkey_bytes, KeySpec::Host);
        }

        Ok((identities, key_map))
    }

    /// Derive only the private seed for a specific key spec.
    /// The seed is held only for the duration of signing.
    async fn derive_seed(&self, spec: &KeySpec) -> Result<[u8; 32], AgentError> {
        let signer = self.signer.lock().await;
        let root = signer
            .root_secret()
            .await
            .map_err(|e| AgentError::Other(Box::new(AgentErrorWrap(e))))?;

        let deriver = KeyDeriver::new(root.as_bytes());
        let seed = match spec {
            KeySpec::SshUser(label) => {
                deriver.derive_ssh_user_key(label).expect("label validated at config time")
            }
            KeySpec::GitSigning => deriver.git_signing_seed(),
            KeySpec::Agent(name) => {
                deriver.derive_agent_key(name).expect("agent name validated at config time")
            }
            KeySpec::Host => deriver.derive(KeyPurpose::SshHost),
        };
        Ok(seed)
    }
}

/// Specifies which key to derive — stored in the pubkey→spec map.
/// Contains no private key material.
#[derive(Clone)]
enum KeySpec {
    SshUser(String),
    GitSigning,
    Agent(String),
    Host,
}

/// Wrapper to make SignerError implement std::error::Error for AgentError::Other.
#[derive(Debug)]
struct AgentErrorWrap(SignerError);

impl std::fmt::Display for AgentErrorWrap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for AgentErrorWrap {}

#[async_trait::async_trait]
impl Session for StyreneAgent {
    async fn request_identities(&mut self) -> Result<Vec<Identity>, AgentError> {
        let (identities, _key_map) = self.derive_public_map().await?;
        Ok(identities)
    }

    async fn sign(&mut self, request: SignRequest) -> Result<Signature, AgentError> {
        // Step 1: derive only public keys to find which key spec matches.
        let (_identities, key_map) = self.derive_public_map().await?;

        // Extract the public key bytes from the request.
        let requested_pubkey = match &request.pubkey {
            KeyData::Ed25519(pk) => pk.0,
            _ => {
                return Err(AgentError::other(AgentErrorWrap(SignerError::Unavailable(
                    "only Ed25519 keys are supported".into(),
                ))));
            }
        };

        // Find the matching key spec.
        let spec = key_map.get(&requested_pubkey).ok_or_else(|| {
            AgentError::other(AgentErrorWrap(SignerError::KeyNotFound(
                "no matching key found for this public key".into(),
            )))
        })?;

        // Step 2: derive ONLY the matching private seed, sign, then zeroize.
        let mut seed = self.derive_seed(spec).await?;
        let sig_bytes = crate::pubkey::sign_with_seed(&seed, &request.data);
        seed.zeroize();

        Signature::new(Algorithm::Ed25519, sig_bytes.to_vec()).map_err(AgentError::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_signer::FileSigner;

    fn test_signer() -> (Box<dyn IdentitySigner>, tempfile::TempDir) {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("identity.key");
        let signer = FileSigner::with_static_passphrase(&path, b"test");
        signer.generate(b"test").expect("generate");
        (Box::new(signer), dir) // caller holds dir to keep files alive
    }

    #[tokio::test]
    async fn request_identities_returns_configured_labels() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github", "work"]);

        let identities = agent.request_identities().await.expect("identities");
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].comment, "styrene-ssh-user-github");
        assert_eq!(identities[1].comment, "styrene-ssh-user-work");
    }

    #[tokio::test]
    async fn request_identities_with_host_key() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github"]).with_host_key();

        let identities = agent.request_identities().await.expect("identities");
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].comment, "styrene-ssh-user-github");
        assert_eq!(identities[1].comment, "styrene-ssh-host");
    }

    #[tokio::test]
    async fn sign_with_known_key() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github"]);

        let identities = agent.request_identities().await.expect("identities");
        let pubkey = identities[0].pubkey.clone();

        let request = SignRequest { pubkey, data: b"hello world".to_vec(), flags: 0 };

        let sig = agent.sign(request).await.expect("sign");
        assert_eq!(sig.algorithm(), Algorithm::Ed25519);
        assert_eq!(sig.as_bytes().len(), 64);
    }

    #[tokio::test]
    async fn sign_with_unknown_key_fails() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github"]);

        let request = SignRequest {
            pubkey: KeyData::Ed25519(Ed25519PublicKey([0u8; 32])),
            data: b"hello".to_vec(),
            flags: 0,
        };

        let result = agent.sign(request).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn signatures_are_deterministic() {
        // Set env var here too, in case another test's cleanup races
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github"]);

        let identities = agent.request_identities().await.expect("identities");
        let pubkey = identities[0].pubkey.clone();

        let request1 =
            SignRequest { pubkey: pubkey.clone(), data: b"deterministic".to_vec(), flags: 0 };
        let request2 = SignRequest { pubkey, data: b"deterministic".to_vec(), flags: 0 };

        let sig1 = agent.sign(request1).await.expect("sign1");
        let sig2 = agent.sign(request2).await.expect("sign2");
        assert_eq!(sig1.as_bytes(), sig2.as_bytes());
    }

    #[tokio::test]
    async fn different_labels_produce_different_keys() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github", "work"]);

        let identities = agent.request_identities().await.expect("identities");
        assert_ne!(identities[0].pubkey, identities[1].pubkey);
    }

    #[tokio::test]
    async fn git_signing_key_served() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github"]).with_git_signing();

        let identities = agent.request_identities().await.expect("identities");
        assert_eq!(identities.len(), 2);
        assert_eq!(identities[0].comment, "styrene-ssh-user-github");
        assert_eq!(identities[1].comment, "styrene-git-signing");
    }

    #[tokio::test]
    async fn agent_keys_served() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github"])
            .with_agent_keys(&["omegon-primary", "omegon-cleave-0"]);

        let identities = agent.request_identities().await.expect("identities");
        assert_eq!(identities.len(), 3);
        assert_eq!(identities[0].comment, "styrene-ssh-user-github");
        assert_eq!(identities[1].comment, "styrene-agent:omegon-primary");
        assert_eq!(identities[2].comment, "styrene-agent:omegon-cleave-0");
    }

    #[tokio::test]
    async fn all_key_families_distinct() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &["github"])
            .with_git_signing()
            .with_agent_keys(&["omegon-primary"])
            .with_host_key();

        let identities = agent.request_identities().await.expect("identities");
        assert_eq!(identities.len(), 4);

        // All public keys must be unique
        let pubkeys: Vec<_> = identities.iter().map(|i| &i.pubkey).collect();
        for i in 0..pubkeys.len() {
            for j in (i + 1)..pubkeys.len() {
                assert_ne!(
                    pubkeys[i], pubkeys[j],
                    "collision between {} and {}",
                    identities[i].comment, identities[j].comment
                );
            }
        }
    }

    #[tokio::test]
    async fn sign_with_agent_key() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &[]).with_agent_keys(&["omegon-primary"]);

        let identities = agent.request_identities().await.expect("identities");
        assert_eq!(identities[0].comment, "styrene-agent:omegon-primary");

        let request = SignRequest {
            pubkey: identities[0].pubkey.clone(),
            data: b"agent commit data".to_vec(),
            flags: 0,
        };

        let sig = agent.sign(request).await.expect("sign");
        assert_eq!(sig.algorithm(), Algorithm::Ed25519);
        assert_eq!(sig.as_bytes().len(), 64);
    }

    #[tokio::test]
    async fn sign_with_git_signing_key() {
        let (signer, _dir) = test_signer();
        let mut agent = StyreneAgent::new(signer, &[]).with_git_signing();

        let identities = agent.request_identities().await.expect("identities");
        assert_eq!(identities[0].comment, "styrene-git-signing");

        let request = SignRequest {
            pubkey: identities[0].pubkey.clone(),
            data: b"git commit signature payload".to_vec(),
            flags: 0,
        };

        let sig = agent.sign(request).await.expect("sign");
        assert_eq!(sig.algorithm(), Algorithm::Ed25519);
    }
}
