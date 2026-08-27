use dioxus::prelude::*;

use crate::daemon_bridge::DaemonCommand;
use crate::safety::{ConfirmationToken, ControlPlane, SafetyAction, SafetyContext};
use crate::state::PeerEntry;
use crate::stores::{FleetJob, FleetJobState, FleetOperation};

#[derive(Clone, PartialEq, Eq)]
struct Confirmation {
    target: String,
    operation: FleetOperation,
    profile_base64: Option<String>,
    token: ConfirmationToken,
}

#[component]
pub fn FleetPage(
    peers: Vec<PeerEntry>,
    managed_peers: Vec<String>,
    jobs: Vec<FleetJob>,
    safety: Memo<SafetyContext>,
    on_command: EventHandler<DaemonCommand>,
) -> Element {
    let safety_policy = safety;
    let mut selected = use_signal(|| None::<String>);
    let mut command = use_signal(String::new);
    let mut profile_base64 = use_signal(String::new);
    let mut confirmation = use_signal(|| None::<Confirmation>);
    let inventory: Vec<_> =
        peers.iter().filter(|peer| managed_peers.contains(&peer.hash)).cloned().collect();
    let selected_peer = selected
        .read()
        .as_ref()
        .and_then(|hash| inventory.iter().find(|peer| peer.hash == *hash))
        .cloned();
    let availability = |capability: &str, destructive: bool| {
        safety_policy
            .read()
            .authorize(ControlPlane::Operate, &SafetyAction::operate(capability, destructive))
    };
    let available = ["rpc.status", "rpc.exec", "rpc.reboot", "rpc.fleet_apply", "policy.update"]
        .iter()
        .any(|capability| availability(capability, *capability != "rpc.status").is_ok());
    let unavailable_reason = availability("rpc.status", false)
        .err()
        .unwrap_or_else(|| "No Fleet operation is authorized".into());

    rsx! {
        div { class: "fleet-page",
            aside { class: "sidebar",
                div { class: "sidebar-header", "Managed Inventory" }
                if inventory.is_empty() {
                    div { class: "sidebar-empty", "No peers advertise Fleet capabilities." }
                }
                for peer in inventory {
                    {
                        let hash = peer.hash.clone();
                        let selected_row = selected.read().as_deref() == Some(hash.as_str());
                        rsx! {
                            button {
                                class: if selected_row { "peer-item selected fleet-peer" } else { "peer-item fleet-peer" },
                                onclick: move |_| selected.set(Some(hash.clone())),
                                span { class: "peer-icon", "●" }
                                span { class: "peer-name", {peer.name.as_deref().unwrap_or("Unnamed peer")} }
                            }
                        }
                    }
                }
            }
            main { class: "main fleet-main",
                if !available {
                    div { class: "empty-state",
                        h2 { "Fleet unavailable" }
                        p { "{unavailable_reason}" }
                    }
                } else if let Some(peer) = selected_peer {
                    {
                        let status_policy = availability("rpc.status", false);
                        let exec_policy = availability("rpc.exec", true);
                        let reboot_policy = availability("rpc.reboot", true);
                        let apply_policy = availability("rpc.fleet_apply", true);
                        let block_policy = availability("policy.update", true);
                        let supports_status = supports(&peer, "status") && status_policy.is_ok();
                        let supports_exec = supports(&peer, "exec") && exec_policy.is_ok();
                        let supports_reboot = supports(&peer, "reboot") && reboot_policy.is_ok();
                        let supports_apply = supports(&peer, "apply") && apply_policy.is_ok();
                        let target_status = peer.hash.clone();
                        let target_exec = peer.hash.clone();
                        let target_reboot = peer.hash.clone();
                        let profile_requirement = FleetOperation::ApplyProfile.required_capability();
                        let status_reason = (!supports_status).then(|| status_policy.clone().err().unwrap_or_else(|| "Peer did not advertise fleet.status".into()));
                        let exec_reason = if !supports_exec {
                            Some(exec_policy.clone().err().unwrap_or_else(|| "Peer did not advertise fleet.exec".into()))
                        } else if command.read().trim().is_empty() {
                            Some("Enter a command before execution".into())
                        } else {
                            None
                        };
                        let reboot_reason = (!supports_reboot).then(|| reboot_policy.clone().err().unwrap_or_else(|| "Peer did not advertise fleet.reboot".into()));
                        let apply_reason = if !supports_apply {
                            Some(apply_policy.clone().err().unwrap_or_else(|| format!("Requires {profile_requirement}")))
                        } else if profile_base64.read().trim().is_empty() {
                            Some("Enter a signed profile before applying it".into())
                        } else {
                            None
                        };
                        let block_reason = if let Err(reason) = &block_policy {
                            Some(reason.clone())
                        } else if peer.identity_hash.is_none() {
                            Some("Identity hash is not reported; destination hash will not be substituted".into())
                        } else {
                            None
                        };
                        rsx! {
                            section { class: "fleet-device",
                                h2 { {peer.name.as_deref().unwrap_or("Managed peer")} }
                                code { "{peer.hash}" }
                                div { class: "fleet-capabilities",
                                    for capability in &peer.capabilities {
                                        span { "{capability}" }
                                    }
                                }
                                div { class: "fleet-actions",
                                    if let Some(reason) = &status_reason { p { id: "fleet-status-disabled", class: "control-disabled-reason", "Query Status disabled: {reason}" } }
                                    button {
                                        disabled: !supports_status,
                                        title: status_reason.clone().unwrap_or_else(|| "Query remote status".into()),
                                        aria_describedby: status_reason.as_ref().map(|_| "fleet-status-disabled"),
                                        onclick: move |_| on_command.call(DaemonCommand::FleetStatus {
                                            destination: target_status.clone(),
                                        }),
                                        "Query Status"
                                    }
                                    div { class: "fleet-exec",
                                        input {
                                            aria_label: "Remote command",
                                            placeholder: "Command",
                                            value: "{command}",
                                            oninput: move |event| command.set(event.value()),
                                        }
                                        button {
                                            disabled: !supports_exec || command.read().trim().is_empty(),
                                            title: exec_reason.clone().unwrap_or_else(|| "Confirm remote execution".into()),
                                            aria_describedby: exec_reason.as_ref().map(|_| "fleet-exec-disabled"),
                                            onclick: move |_| {
                                                let operation = FleetOperation::Execute {
                                                    command: command.read().clone(),
                                                    args: Vec::new(),
                                                };
                                                if let Some(pending) = fleet_confirmation(&safety_policy.read(), target_exec.clone(), operation, None) {
                                                    confirmation.set(Some(pending));
                                                }
                                            },
                                            "Execute"
                                        }
                                        if let Some(reason) = &exec_reason { p { id: "fleet-exec-disabled", class: "control-disabled-reason", "Execute disabled: {reason}" } }
                                    }
                                    if let Some(reason) = &reboot_reason { p { id: "fleet-reboot-disabled", class: "control-disabled-reason", "Reboot disabled: {reason}" } }
                                    button {
                                        class: "danger",
                                        disabled: !supports_reboot,
                                        title: reboot_reason.clone().unwrap_or_else(|| "Confirm remote reboot".into()),
                                        aria_describedby: reboot_reason.as_ref().map(|_| "fleet-reboot-disabled"),
                                        onclick: move |_| {
                                            if let Some(pending) = fleet_confirmation(
                                                &safety_policy.read(),
                                                target_reboot.clone(),
                                                FleetOperation::Reboot { delay_secs: Some(5) },
                                                None,
                                            ) {
                                                confirmation.set(Some(pending));
                                            }
                                        },
                                        "Reboot"
                                    }
                                    div { class: "fleet-profile",
                                        textarea {
                                            aria_label: "Base64 signed profile",
                                            placeholder: "Base64 signed profile",
                                            value: "{profile_base64}",
                                            oninput: move |event| profile_base64.set(event.value()),
                                        }
                                        button {
                                            disabled: !supports_apply || profile_base64.read().trim().is_empty(),
                                            title: apply_reason.clone().unwrap_or_else(|| "Confirm signed profile application".into()),
                                            aria_describedby: apply_reason.as_ref().map(|_| "fleet-apply-disabled"),
                                            onclick: {
                                                let destination = peer.hash.clone();
                                                move |_| {
                                                    if let Some(pending) = fleet_confirmation(
                                                        &safety_policy.read(),
                                                        destination.clone(),
                                                        FleetOperation::ApplyProfile,
                                                        Some(profile_base64.read().trim().to_string()),
                                                    ) {
                                                        confirmation.set(Some(pending));
                                                    }
                                                }
                                            },
                                            "Apply Profile"
                                        }
                                        if let Some(reason) = &apply_reason { p { id: "fleet-apply-disabled", class: "control-disabled-reason", "Apply Profile disabled: {reason}" } }
                                    }
                                    if let Some(reason) = &block_reason { p { id: "fleet-block-disabled", class: "control-disabled-reason", "Block Peer disabled: {reason}" } }
                                    button {
                                        class: "danger",
                                        disabled: block_policy.is_err() || peer.identity_hash.is_none(),
                                        title: block_reason.clone().unwrap_or_else(|| "Confirm local peer block".into()),
                                        aria_describedby: block_reason.as_ref().map(|_| "fleet-block-disabled"),
                                        onclick: {
                                            let identity_hash = peer.identity_hash.clone();
                                            move |_| if let Some(identity_hash) = identity_hash.clone() {
                                                if let Some(pending) = fleet_confirmation(
                                                    &safety_policy.read(),
                                                    identity_hash,
                                                    FleetOperation::Block,
                                                    None,
                                                ) {
                                                    confirmation.set(Some(pending));
                                                }
                                            }
                                        },
                                        "Block Peer"
                                    }
                                }
                            }
                        }
                    }
                } else {
                    div { class: "empty-state",
                        h2 { "Fleet Operations" }
                        p { "Select a peer whose announce advertises Fleet capabilities." }
                    }
                }
                section { class: "fleet-jobs",
                    h3 { "Audit Timeline" }
                    if jobs.is_empty() {
                        p { class: "fleet-empty", "No Fleet jobs have been submitted." }
                    }
                    for job in jobs.iter().rev() {
                        article { class: "fleet-job",
                            div {
                                strong { "{job.operation.label()}" }
                                code { "{job.id}" }
                            }
                            span { class: "fleet-job-target", "{short_hash(&job.target)}" }
                            span { class: job_state_class(&job.state), "{job.state:?}" }
                            span { "requires {job.operation.required_capability()}" }
                            if let Some(result) = &job.result {
                                p { "{result}" }
                            }
                            if let Some(error) = &job.error {
                                p { class: "fleet-job-error", "{error}" }
                            }
                        }
                    }
                }
            }
            if let Some(pending) = confirmation.read().clone() {
                div { class: "confirmation-backdrop",
                    div {
                        class: "confirmation-dialog",
                        role: "dialog",
                        aria_modal: "true",
                        aria_labelledby: "fleet-confirmation-title",
                        onkeydown: move |event: KeyboardEvent| if event.key() == Key::Escape {
                            confirmation.set(None);
                        },
                        h2 { id: "fleet-confirmation-title", "Confirm {pending.operation.label()}" }
                        p { "Target: {pending.target}" }
                        p { "Required capability: {pending.operation.required_capability()}" }
                        p { "Parameters: {fleet_parameters(&pending)}" }
                        p { {fleet_consequence(&pending.operation)} }
                        if let FleetOperation::Execute { command, args } = &pending.operation {
                            code { "{command} " {args.join(" ")} }
                        }
                        div { class: "confirmation-actions",
                            button { autofocus: true, onclick: move |_| confirmation.set(None), "Cancel" }
                            button {
                                class: "danger",
                                disabled: safety_policy.read().confirm(
                                    ControlPlane::Operate,
                                    &fleet_action(&pending.operation),
                                    pending.token,
                                ).is_err(),
                                onclick: move |_| {
                                    if let Err(error) = safety_policy.read().confirm(
                                        ControlPlane::Operate,
                                        &fleet_action(&pending.operation),
                                        pending.token,
                                    ) {
                                        tracing::warn!(target: "dx::safety", %error, "Fleet confirmation rejected");
                                        return;
                                    }
                                    match pending.operation.clone() {
                                        FleetOperation::Execute { command, args } => {
                                            on_command.call(DaemonCommand::FleetExec {
                                                destination: pending.target.clone(),
                                                command,
                                                args,
                                            });
                                        }
                                        FleetOperation::Reboot { delay_secs } => {
                                            on_command.call(DaemonCommand::FleetReboot {
                                                destination: pending.target.clone(),
                                                delay: delay_secs,
                                            });
                                        }
                                        FleetOperation::ApplyProfile => {
                                            if let Some(submitted_profile) = pending.profile_base64.clone() {
                                                on_command.call(DaemonCommand::FleetApply {
                                                    destination: pending.target.clone(),
                                                    profile_base64: submitted_profile,
                                                });
                                                profile_base64.set(String::new());
                                            }
                                        }
                                        FleetOperation::Block => {
                                            on_command.call(DaemonCommand::BlockPeer {
                                                identity_hash: pending.target.clone(),
                                            });
                                        }
                                        FleetOperation::Status => {}
                                    }
                                    confirmation.set(None);
                                },
                                "Confirm"
                            }
                        }
                        if let Err(reason) = safety_policy.read().confirm(
                            ControlPlane::Operate,
                            &fleet_action(&pending.operation),
                            pending.token,
                        ) {
                            p { class: "control-disabled-reason", "Confirmation unavailable: {reason}" }
                        }
                    }
                }
            }
        }
    }
}

fn fleet_action(operation: &FleetOperation) -> SafetyAction {
    SafetyAction::operate(operation.required_capability(), operation != &FleetOperation::Status)
}

fn fleet_confirmation(
    safety: &SafetyContext,
    target: String,
    operation: FleetOperation,
    profile_base64: Option<String>,
) -> Option<Confirmation> {
    match safety.begin_confirmation(ControlPlane::Operate, &fleet_action(&operation)) {
        Ok(token) => Some(Confirmation { target, operation, profile_base64, token }),
        Err(error) => {
            tracing::warn!(target: "dx::safety", %error, "Fleet action rejected");
            None
        }
    }
}

fn fleet_consequence(operation: &FleetOperation) -> &'static str {
    match operation {
        FleetOperation::Status => "Reads remote status without changing the peer.",
        FleetOperation::Execute { .. } => "Executes the displayed command on the remote peer.",
        FleetOperation::Reboot { .. } => "Interrupts service by rebooting the remote peer.",
        FleetOperation::Block => "Updates local policy to deny this peer identity.",
        FleetOperation::ApplyProfile => "Replaces the remote peer's active signed profile.",
    }
}

fn fleet_parameters(confirmation: &Confirmation) -> String {
    match &confirmation.operation {
        FleetOperation::Status => "none".into(),
        FleetOperation::Execute { command, args } => {
            format!("command={command}, arguments={}", args.len())
        }
        FleetOperation::Reboot { delay_secs } => format!(
            "delay={} seconds",
            delay_secs.map(|value| value.to_string()).unwrap_or_else(|| "daemon default".into())
        ),
        FleetOperation::Block => "identity hash shown as target".into(),
        FleetOperation::ApplyProfile => format!(
            "signed profile supplied ({} encoded bytes; content hidden)",
            confirmation.profile_base64.as_ref().map_or(0, String::len)
        ),
    }
}

fn supports(peer: &PeerEntry, operation: &str) -> bool {
    peer.capabilities.iter().any(|capability| {
        matches!(capability.as_str(), "fleet" | "api")
            || capability == operation
            || capability == &format!("fleet.{operation}")
    })
}

fn short_hash(hash: &str) -> String {
    hash[..12.min(hash.len())].to_string()
}

fn job_state_class(state: &FleetJobState) -> &'static str {
    match state {
        FleetJobState::Running => "fleet-job-state running",
        FleetJobState::Succeeded => "fleet-job-state succeeded",
        FleetJobState::Denied
        | FleetJobState::Unsupported
        | FleetJobState::TimedOut
        | FleetJobState::Failed => "fleet-job-state failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PeerRole;

    #[test]
    fn peer_actions_require_advertised_capability() {
        let mut peer = PeerEntry {
            hash: "peer".into(),
            identity_hash: Some("identity".into()),
            name: None,
            status: "online".into(),
            node_role: PeerRole::Styrene,
            capabilities: vec!["status".into()],
            version: None,
            last_announce: None,
            announce_count: 1,
        };
        assert!(supports(&peer, "status"));
        assert!(!supports(&peer, "reboot"));
        peer.capabilities.push("fleet".into());
        assert!(supports(&peer, "reboot"));
    }

    #[test]
    fn profile_confirmation_reports_parameters_without_exposing_profile() {
        let confirmation = Confirmation {
            target: "peer".into(),
            operation: FleetOperation::ApplyProfile,
            profile_base64: Some("secret-profile".into()),
            token: ConfirmationToken::fixture(1, Some(1)),
        };
        let parameters = fleet_parameters(&confirmation);
        assert!(parameters.contains("14 encoded bytes"));
        assert!(!parameters.contains("secret-profile"));
    }
}
