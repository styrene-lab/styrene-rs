use crate::error::{code, ErrorCategory, SdkError};
use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;
use std::collections::BTreeMap;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
#[non_exhaustive]
pub enum Profile {
    DesktopFull,
    DesktopLocalRuntime,
    EmbeddedAlloc,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum BindMode {
    LocalOnly,
    Remote,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AuthMode {
    LocalTrusted,
    Token,
    Mtls,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum OverflowPolicy {
    Reject,
    DropOldest,
    Block,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct EventStreamConfig {
    pub max_poll_events: usize,
    pub max_event_bytes: usize,
    pub max_batch_bytes: usize,
    pub max_extension_keys: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum RedactionTransform {
    Hash,
    Truncate,
    Redact,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RedactionConfig {
    pub enabled: bool,
    pub sensitive_transform: RedactionTransform,
    pub break_glass_allowed: bool,
    pub break_glass_ttl_ms: Option<u64>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct TokenAuthConfig {
    pub issuer: String,
    pub audience: String,
    pub jti_cache_ttl_ms: u64,
    pub clock_skew_ms: u64,
    pub shared_secret: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct MtlsAuthConfig {
    pub ca_bundle_path: String,
    pub require_client_cert: bool,
    pub allowed_san: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct RpcBackendConfig {
    pub listen_addr: String,
    pub read_timeout_ms: u64,
    pub write_timeout_ms: u64,
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
    pub token_auth: Option<TokenAuthConfig>,
    pub mtls_auth: Option<MtlsAuthConfig>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[non_exhaustive]
pub struct SdkConfig {
    pub profile: Profile,
    pub bind_mode: BindMode,
    pub auth_mode: AuthMode,
    pub overflow_policy: OverflowPolicy,
    pub block_timeout_ms: Option<u64>,
    pub event_stream: EventStreamConfig,
    pub idempotency_ttl_ms: u64,
    pub redaction: RedactionConfig,
    pub rpc_backend: Option<RpcBackendConfig>,
    #[serde(default)]
    pub extensions: BTreeMap<String, JsonValue>,
}

impl SdkConfig {
    pub fn validate(&self) -> Result<(), SdkError> {
        if self.overflow_policy == OverflowPolicy::Block && self.block_timeout_ms.is_none() {
            return Err(SdkError::new(
                code::VALIDATION_INVALID_ARGUMENT,
                ErrorCategory::Validation,
                "overflow_policy=block requires block_timeout_ms",
            )
            .with_user_actionable(true)
            .with_detail("field", JsonValue::String("block_timeout_ms".to_owned())));
        }

        if self.event_stream.max_extension_keys > 32 {
            return Err(SdkError::new(
                code::VALIDATION_MAX_EXTENSION_KEYS_EXCEEDED,
                ErrorCategory::Validation,
                "event stream extension key limit exceeds contract maximum",
            )
            .with_user_actionable(true)
            .with_detail("limit_name", JsonValue::String("max_extension_keys".to_owned()))
            .with_detail("limit_value", JsonValue::from(self.event_stream.max_extension_keys)));
        }

        match self.bind_mode {
            BindMode::LocalOnly => {
                if self.auth_mode != AuthMode::LocalTrusted {
                    return Err(SdkError::new(
                        code::SECURITY_AUTH_REQUIRED,
                        ErrorCategory::Security,
                        "local_only bind mode requires local_trusted auth mode",
                    )
                    .with_user_actionable(true));
                }
            }
            BindMode::Remote => {
                if !matches!(self.auth_mode, AuthMode::Token | AuthMode::Mtls) {
                    return Err(SdkError::new(
                        code::SECURITY_REMOTE_BIND_DISALLOWED,
                        ErrorCategory::Security,
                        "remote bind mode requires token or mtls auth mode",
                    )
                    .with_user_actionable(true));
                }
            }
        }

        match self.auth_mode {
            AuthMode::LocalTrusted => {}
            AuthMode::Token => {
                let token_auth =
                    self.rpc_backend.as_ref().and_then(|backend| backend.token_auth.as_ref());
                let Some(token_auth) = token_auth else {
                    return Err(SdkError::new(
                        code::SECURITY_AUTH_REQUIRED,
                        ErrorCategory::Security,
                        "token auth mode requires rpc_backend.token_auth configuration",
                    )
                    .with_user_actionable(true));
                };
                if token_auth.issuer.trim().is_empty() || token_auth.audience.trim().is_empty() {
                    return Err(SdkError::new(
                        code::VALIDATION_INVALID_ARGUMENT,
                        ErrorCategory::Validation,
                        "token auth configuration requires issuer and audience",
                    )
                    .with_user_actionable(true));
                }
                if token_auth.jti_cache_ttl_ms == 0 {
                    return Err(SdkError::new(
                        code::VALIDATION_INVALID_ARGUMENT,
                        ErrorCategory::Validation,
                        "token auth jti_cache_ttl_ms must be greater than zero",
                    )
                    .with_user_actionable(true));
                }
                if token_auth.shared_secret.trim().is_empty() {
                    return Err(SdkError::new(
                        code::SECURITY_AUTH_REQUIRED,
                        ErrorCategory::Security,
                        "token auth shared_secret must be configured",
                    )
                    .with_user_actionable(true));
                }
            }
            AuthMode::Mtls => {
                let mtls_auth =
                    self.rpc_backend.as_ref().and_then(|backend| backend.mtls_auth.as_ref());
                if mtls_auth.is_none() {
                    return Err(SdkError::new(
                        code::SECURITY_AUTH_REQUIRED,
                        ErrorCategory::Security,
                        "mtls auth mode requires rpc_backend.mtls_auth configuration",
                    )
                    .with_user_actionable(true));
                }
            }
        }

        Ok(())
    }
}
