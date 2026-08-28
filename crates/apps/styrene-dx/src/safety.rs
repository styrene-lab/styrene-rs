use crate::backend::RuntimeProfile;
use styrene_ipc::types::{ACTIVE_CAPABILITIES_VERSION, ActiveCapabilitiesInfo};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ControlPlane {
    Operate,
    Lab,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafetyAction {
    plane: ControlPlane,
    required_capability: Option<String>,
    requires_daemon_session: bool,
    external_network: bool,
    destructive: bool,
}

impl SafetyAction {
    pub fn operate(capability: impl Into<String>, destructive: bool) -> Self {
        Self {
            plane: ControlPlane::Operate,
            required_capability: Some(capability.into()),
            requires_daemon_session: true,
            external_network: false,
            destructive,
        }
    }

    pub fn fixture_scenario() -> Self {
        Self {
            plane: ControlPlane::Lab,
            required_capability: Some("fixture.scenarios".into()),
            requires_daemon_session: true,
            external_network: false,
            destructive: false,
        }
    }

    pub fn live_scenario() -> Self {
        Self {
            plane: ControlPlane::Lab,
            required_capability: None,
            requires_daemon_session: false,
            external_network: true,
            destructive: true,
        }
    }

    pub fn lab_evidence_export() -> Self {
        Self {
            plane: ControlPlane::Lab,
            required_capability: None,
            requires_daemon_session: false,
            external_network: false,
            destructive: false,
        }
    }

    pub fn is_destructive(&self) -> bool {
        self.destructive
    }

    pub fn required_capability(&self) -> Option<&str> {
        self.required_capability.as_deref()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmationToken {
    client_generation: u64,
    server_generation: Option<u64>,
}

#[cfg(test)]
impl ConfirmationToken {
    pub(crate) const fn fixture(client_generation: u64, server_generation: Option<u64>) -> Self {
        Self { client_generation, server_generation }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RuntimeKind {
    Live,
    Embedded,
    Fixture,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SafetyContext {
    runtime: Option<RuntimeKind>,
    connected: bool,
    client_generation: u64,
    server_generation: Option<u64>,
    capability_version: Option<u16>,
    authorized_operations: Vec<String>,
    runtime_capabilities: Vec<String>,
    degraded: Vec<(String, String)>,
}

impl SafetyContext {
    pub fn new(
        profile: Option<&RuntimeProfile>,
        connected: bool,
        client_generation: u64,
        server_generation: Option<u64>,
        capabilities: Option<&ActiveCapabilitiesInfo>,
    ) -> Self {
        let runtime = profile.map(|profile| match profile {
            RuntimeProfile::Live { .. } => RuntimeKind::Live,
            RuntimeProfile::Embedded { .. } => RuntimeKind::Embedded,
            RuntimeProfile::Fixture { .. } => RuntimeKind::Fixture,
        });
        Self {
            runtime,
            connected,
            client_generation,
            server_generation,
            capability_version: capabilities.map(|value| value.version),
            authorized_operations: capabilities
                .map(|value| value.authorized_operations.clone())
                .unwrap_or_default(),
            runtime_capabilities: capabilities
                .map(|value| value.runtime.clone())
                .unwrap_or_default(),
            degraded: capabilities
                .map(|value| {
                    value
                        .degraded
                        .iter()
                        .map(|item| (item.id.clone(), "daemon reported degraded".into()))
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    pub fn authorize(&self, plane: ControlPlane, action: &SafetyAction) -> Result<(), String> {
        if action.plane != plane {
            return Err(format!(
                "{:?} action is unavailable from the {:?} control plane",
                action.plane, plane
            ));
        }
        let runtime = self.runtime.ok_or_else(|| "runtime profile is invalid".to_string())?;
        if action.external_network && runtime == RuntimeKind::Fixture {
            return Err("Fixture profile cannot open external network interfaces".into());
        }
        if !action.requires_daemon_session {
            return Ok(());
        }
        if !self.connected {
            return Err("daemon is disconnected".into());
        }
        if self.server_generation.is_none() {
            return Err("daemon generation is unknown".into());
        }
        match self.capability_version {
            Some(ACTIVE_CAPABILITIES_VERSION) => {}
            Some(version) => return Err(format!("capabilities are stale (version {version})")),
            None => return Err("capabilities are unknown".into()),
        }
        let Some(capability) = action.required_capability.as_deref() else {
            return Ok(());
        };
        if let Some((_, reason)) = self.degraded.iter().find(|(id, _)| id == capability) {
            return Err(format!("{capability} unavailable: {reason}"));
        }
        let authorized = match action.plane {
            ControlPlane::Operate => &self.authorized_operations,
            ControlPlane::Lab => &self.runtime_capabilities,
        };
        if authorized.iter().any(|item| item == capability) {
            Ok(())
        } else {
            Err(format!("current daemon session does not authorize {capability}"))
        }
    }

    pub fn operate_session_availability(&self) -> Result<(), String> {
        self.authorize(
            ControlPlane::Operate,
            &SafetyAction {
                plane: ControlPlane::Operate,
                required_capability: None,
                requires_daemon_session: true,
                external_network: false,
                destructive: false,
            },
        )
    }

    pub fn begin_confirmation(
        &self,
        plane: ControlPlane,
        action: &SafetyAction,
    ) -> Result<ConfirmationToken, String> {
        if !action.is_destructive() {
            return Err("action does not require destructive confirmation".into());
        }
        self.authorize(plane, action)?;
        Ok(ConfirmationToken {
            client_generation: self.client_generation,
            server_generation: self.server_generation,
        })
    }

    pub fn confirm(
        &self,
        plane: ControlPlane,
        action: &SafetyAction,
        token: ConfirmationToken,
    ) -> Result<(), String> {
        if token.client_generation != self.client_generation
            || token.server_generation != self.server_generation
        {
            return Err("confirmation expired because the runtime generation changed".into());
        }
        self.authorize(plane, action)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn capabilities(authorized: &[&str], runtime: &[&str]) -> ActiveCapabilitiesInfo {
        let mut value = ActiveCapabilitiesInfo::default();
        value.version = ACTIVE_CAPABILITIES_VERSION;
        value.authorized_operations = authorized.iter().map(|item| (*item).into()).collect();
        value.runtime = runtime.iter().map(|item| (*item).into()).collect();
        value
    }

    fn live_context(generation: u64) -> SafetyContext {
        SafetyContext::new(
            Some(&RuntimeProfile::Live { socket_path: "/tmp/styrene.sock".into() }),
            true,
            generation,
            Some(generation),
            Some(&capabilities(&["rpc.reboot"], &["fixture.scenarios"])),
        )
    }

    #[test]
    fn control_planes_reject_cross_plane_actions() {
        let context = live_context(7);
        assert!(
            context
                .authorize(ControlPlane::Lab, &SafetyAction::operate("rpc.reboot", true))
                .is_err()
        );
        assert!(context.authorize(ControlPlane::Operate, &SafetyAction::live_scenario()).is_err());
    }

    #[test]
    fn operate_actions_require_current_capability() {
        let context = live_context(7);
        assert!(
            context
                .authorize(ControlPlane::Operate, &SafetyAction::operate("rpc.reboot", true))
                .is_ok()
        );
        assert!(
            context
                .authorize(ControlPlane::Operate, &SafetyAction::operate("policy.update", true))
                .is_err()
        );
    }

    #[test]
    fn confirmation_expires_across_runtime_generation() {
        let action = SafetyAction::operate("rpc.reboot", true);
        let token = live_context(7)
            .begin_confirmation(ControlPlane::Operate, &action)
            .expect("authorized destructive action");
        let error = live_context(8)
            .confirm(ControlPlane::Operate, &action, token)
            .expect_err("stale confirmation must fail closed");
        assert!(error.contains("generation changed"));
    }

    #[test]
    fn confirmation_rechecks_capability_before_submission() {
        let action = SafetyAction::operate("rpc.reboot", true);
        let token = live_context(7)
            .begin_confirmation(ControlPlane::Operate, &action)
            .expect("authorized destructive action");
        let revoked = SafetyContext::new(
            Some(&RuntimeProfile::Live { socket_path: "/tmp/styrene.sock".into() }),
            true,
            7,
            Some(7),
            Some(&capabilities(&[], &["fixture.scenarios"])),
        );
        let error = revoked
            .confirm(ControlPlane::Operate, &action, token)
            .expect_err("revoked capability must fail closed");
        assert!(error.contains("does not authorize rpc.reboot"));
    }

    #[test]
    fn stale_capability_schema_disables_operate_actions() {
        let mut stale = capabilities(&["rpc.reboot"], &[]);
        stale.version = ACTIVE_CAPABILITIES_VERSION.saturating_add(1);
        let context = SafetyContext::new(
            Some(&RuntimeProfile::Live { socket_path: "/tmp/styrene.sock".into() }),
            true,
            7,
            Some(7),
            Some(&stale),
        );
        let error = context
            .authorize(ControlPlane::Operate, &SafetyAction::operate("rpc.reboot", true))
            .expect_err("stale capabilities must fail closed");
        assert!(error.contains("capabilities are stale"));
    }

    #[test]
    fn fixture_profile_rejects_live_runner_before_networking() {
        let profile = RuntimeProfile::Fixture { fixture: crate::backend::FixtureId::Healthy };
        let context = SafetyContext::new(Some(&profile), true, 1, Some(1), None);
        let error = context
            .authorize(ControlPlane::Lab, &SafetyAction::live_scenario())
            .expect_err("Fixture must not launch live interop");
        assert!(error.contains("cannot open external network"));
    }

    #[test]
    fn fixture_scenarios_use_runtime_capability() {
        let context = live_context(7);
        assert!(context.authorize(ControlPlane::Lab, &SafetyAction::fixture_scenario()).is_ok());
    }
}
