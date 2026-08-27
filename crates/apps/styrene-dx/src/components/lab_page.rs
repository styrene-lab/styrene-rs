use dioxus::prelude::*;

use crate::safety::{ConfirmationToken, ControlPlane, SafetyAction, SafetyContext};
use crate::scenario::{ScenarioDefinition, ScenarioProfile, ScenarioRun, ScenarioStatus};

#[derive(Clone, Debug, PartialEq, Eq)]
enum LabPending {
    Start(String),
    Cancel(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LabConfirmation {
    target: String,
    parameters: &'static str,
    consequence: &'static str,
    pending: LabPending,
    action: SafetyAction,
    token: ConfirmationToken,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct EvidenceView {
    topology: Vec<String>,
    milestones: Vec<String>,
    assertions: Vec<String>,
    revisions: Vec<String>,
    artifacts: Vec<String>,
    logs: Vec<String>,
    cleanup: Vec<String>,
    failure: Option<String>,
}

fn evidence_view(run: &ScenarioRun) -> EvidenceView {
    let Some(evidence) = &run.runner_evidence else {
        return EvidenceView {
            milestones: run.milestones.clone(),
            artifacts: run.evidence.clone(),
            ..EvidenceView::default()
        };
    };
    let topology = evidence
        .topology
        .ports
        .iter()
        .map(|(role, port)| format!("{role}: {}:{port}", evidence.topology.host))
        .chain(std::iter::once(format!(
            "allocation: {} / candidate {}",
            evidence.topology.allocation_key, evidence.topology.candidate
        )))
        .chain(run.evidence.iter().cloned())
        .collect();
    let milestones = evidence
        .milestones
        .iter()
        .map(|item| format!("{} at {} ms", item.name, item.elapsed_ms))
        .collect();
    let assertions = evidence
        .assertions
        .iter()
        .map(|item| {
            format!(
                "{}: {} at {} ms{}",
                item.name,
                if item.passed { "passed" } else { "failed" },
                item.elapsed_ms,
                item.detail.as_ref().map(|detail| format!(" / {detail}")).unwrap_or_default()
            )
        })
        .collect();
    let revisions = evidence
        .revisions
        .iter()
        .map(|item| {
            format!(
                "{}: actual {} / expected {} / match {} / dirty {}",
                item.name,
                item.actual.as_deref().unwrap_or("unavailable"),
                item.expected.as_deref().unwrap_or("any"),
                item.matches,
                item.worktree_dirty
                    .map(|dirty| dirty.to_string())
                    .unwrap_or_else(|| "not reported".into())
            )
        })
        .collect();
    let artifacts = evidence
        .artifacts
        .iter()
        .map(|item| {
            format!(
                "{}: {} bytes / sha256 {} / {}",
                item.name, item.bytes, item.sha256, item.retained_path
            )
        })
        .chain(
            evidence
                .artifact_rejections
                .iter()
                .map(|item| format!("{} rejected: {}", item.name, item.reason)),
        )
        .collect();
    let logs = evidence
        .logs
        .iter()
        .map(|item| {
            format!(
                "{}: {} bytes / truncated {} / content {}",
                item.stream, item.bytes_seen, item.truncated, item.text
            )
        })
        .collect();
    let cleanup = vec![
        format!("direct process reaped: {}", evidence.cleanup.direct_process_reaped),
        format!("process group gone: {}", evidence.cleanup.process_group_gone),
        format!("pipes drained: {}", evidence.cleanup.pipes_drained),
        format!("reader threads joined: {}", evidence.cleanup.reader_threads_joined),
        format!("temporary resources removed: {}", evidence.cleanup.temp_resources_removed),
        format!(
            "topology reservation released: {}",
            evidence.cleanup.topology_reservation_released
        ),
    ];
    EvidenceView {
        topology,
        milestones,
        assertions,
        revisions,
        artifacts,
        logs,
        cleanup,
        failure: evidence.failure.clone(),
    }
}

#[component]
pub fn LabPage(
    profile_label: String,
    fixture_available: bool,
    live_available: bool,
    safety: Memo<SafetyContext>,
    definitions: Vec<ScenarioDefinition>,
    run: Option<ScenarioRun>,
    on_start: EventHandler<String>,
    on_cancel: EventHandler<String>,
    on_export: EventHandler<String>,
) -> Element {
    let safety_policy = safety;
    let mut confirmation = use_signal(|| None::<LabConfirmation>);
    let evidence = run.as_ref().map(evidence_view);
    let active_profile = run.as_ref().and_then(|active| {
        definitions
            .iter()
            .find(|definition| definition.id == active.scenario_id)
            .map(|definition| definition.profile)
    });
    rsx! {
        main { class: "main lab-page",
            header { class: "lab-header",
                div {
                    span { class: "lab-kicker", "Controlled evidence workspace" }
                    h2 { "Protocol Lab" }
                    p { "Profile {profile_label}. Harness outcomes remain authoritative." }
                }
                div { class: "lab-availability",
                    span { class: if fixture_available { "ready" } else { "disabled" }, "Fixture" }
                    span { class: if live_available { "ready" } else { "disabled" }, "Pinned runner" }
                }
            }
            if let Some(active) = run.as_ref() {
                section { class: "scenario-active",
                    div {
                        span { class: "lab-kicker", "Active run" }
                        strong { "{active.scenario_id}" }
                        code { "{active.run_id}" }
                    }
                    span { class: "scenario-status", "{active.status:?}" }
                    div { class: "scenario-actions",
                        if active.status == ScenarioStatus::Running {
                            {
                                let profile = active_profile.unwrap_or(ScenarioProfile::LiveRunner);
                                let action = scenario_action(profile);
                                let cancel_reason = safety_policy.read().authorize(ControlPlane::Lab, &action).err();
                                rsx! {
                            button {
                                disabled: cancel_reason.is_some(),
                                title: cancel_reason.clone().unwrap_or_else(|| "Cancel scenario run".into()),
                                onclick: {
                                    let run_id = active.run_id.clone();
                                    move |_| {
                                        if action.is_destructive() {
                                            if let Some(pending) = lab_confirmation(
                                                &safety_policy.read(),
                                                run_id.clone(),
                                                "cancel the selected run ID",
                                                "Terminates the live runner topology and begins supervised cleanup.",
                                                LabPending::Cancel(run_id.clone()),
                                                action.clone(),
                                            ) {
                                                confirmation.set(Some(pending));
                                            }
                                        } else if safety_policy.read().authorize(ControlPlane::Lab, &action).is_ok() {
                                            on_cancel.call(run_id.clone());
                                        }
                                    }
                                },
                                "Cancel"
                            }
                            if let Some(reason) = &cancel_reason {
                                small { class: "control-disabled-reason", "Cancel disabled: {reason}" }
                            }
                                }
                            }
                        } else if active.status.is_terminal() {
                            {
                                let profile = active_profile.unwrap_or(ScenarioProfile::LiveRunner);
                                let action = scenario_action(profile);
                                let rerun_reason = safety_policy.read().authorize(ControlPlane::Lab, &action).err();
                                let export_action = SafetyAction::lab_evidence_export();
                                let export_reason = safety_policy.read().authorize(ControlPlane::Lab, &export_action).err();
                                rsx! {
                            button {
                                disabled: rerun_reason.is_some(),
                                title: rerun_reason.clone().unwrap_or_else(|| "Rerun scenario".into()),
                                onclick: {
                                    let scenario_id = active.scenario_id.to_string();
                                    move |_| {
                                        if action.is_destructive() {
                                            if let Some(pending) = lab_confirmation(
                                                &safety_policy.read(),
                                                scenario_id.clone(),
                                                "canonical pinned revision, declared topology, and declared controls",
                                                "Launches the pinned external runner and opens its declared loopback interfaces.",
                                                LabPending::Start(scenario_id.clone()),
                                                action.clone(),
                                            ) {
                                                confirmation.set(Some(pending));
                                            }
                                        } else if safety_policy.read().authorize(ControlPlane::Lab, &action).is_ok() {
                                            on_start.call(scenario_id.clone());
                                        }
                                    }
                                },
                                "Rerun"
                            }
                            button {
                                disabled: export_reason.is_some(),
                                title: export_reason.clone().unwrap_or_else(|| "Export bounded redacted evidence".into()),
                                onclick: {
                                    let run_id = active.run_id.clone();
                                    move |_| if safety_policy.read().authorize(ControlPlane::Lab, &export_action).is_ok() {
                                        on_export.call(run_id.clone());
                                    }
                                },
                                "Export evidence"
                            }
                            if let Some(reason) = &rerun_reason {
                                small { class: "control-disabled-reason", "Rerun disabled: {reason}" }
                            }
                            if let Some(reason) = &export_reason {
                                small { class: "control-disabled-reason", "Export disabled: {reason}" }
                            }
                                }
                            }
                        } else {
                            button { disabled: true, "Cancellation pending" }
                        }
                    }
                }
                if let Some(view) = evidence.as_ref() {
                    section { class: "lab-evidence-grid",
                        EvidencePanel { title: "Topology", empty: "Topology is pending runner allocation.", rows: view.topology.clone() }
                        EvidencePanel { title: "Milestones", empty: "No milestones reported.", rows: view.milestones.clone() }
                        EvidencePanel { title: "Assertions", empty: "No assertions reported.", rows: view.assertions.clone() }
                        EvidencePanel { title: "Revision provenance", empty: "Revision probes have not completed.", rows: view.revisions.clone() }
                        EvidencePanel { title: "Retained evidence", empty: "No artifacts retained.", rows: view.artifacts.clone() }
                        EvidencePanel { title: "Redacted logs", empty: "No log metadata reported.", rows: view.logs.clone() }
                        EvidencePanel { title: "Cleanup", empty: "Cleanup evidence is pending.", rows: view.cleanup.clone() }
                        if let Some(failure) = &view.failure {
                            article { class: "lab-evidence-panel failure",
                                h3 { "Harness outcome" }
                                p { "{failure}" }
                            }
                        }
                    }
                }
            }
            section { class: "scenario-grid",
                for definition in definitions {
                    {
                        let fixture = definition.profile == ScenarioProfile::Fixture;
                        let action = scenario_action(definition.profile);
                        let backend_available = if fixture { fixture_available } else { live_available };
                        let safety_reason = safety_policy.read().authorize(ControlPlane::Lab, &action).err();
                        let available = backend_available && safety_reason.is_none();
                        let scenario_id = definition.id.to_string();
                        let controls = definition.controls.join(", ");
                        rsx! {
                            article { class: "scenario-card",
                                div { class: "scenario-card-heading",
                                    h3 { "{definition.title}" }
                                    span { if fixture { "Fixture" } else { "Pinned live" } }
                                }
                                p { "{definition.description}" }
                                code { "{definition.revision}" }
                                div { class: "scenario-controls",
                                    div {
                                        span { "Declared controls" }
                                        strong { "{controls}" }
                                    }
                                    button {
                                        disabled: !available,
                                        title: safety_reason.clone().unwrap_or_else(|| "Start scenario".into()),
                                        onclick: move |_| {
                                            if action.is_destructive() {
                                                if let Some(pending) = lab_confirmation(
                                                    &safety_policy.read(),
                                                    scenario_id.clone(),
                                                    "canonical pinned revision, declared topology, and declared controls",
                                                    "Launches the pinned external runner and opens its declared loopback interfaces.",
                                                    LabPending::Start(scenario_id.clone()),
                                                    action.clone(),
                                                ) {
                                                    confirmation.set(Some(pending));
                                                }
                                            } else if safety_policy.read().authorize(ControlPlane::Lab, &action).is_ok() {
                                                on_start.call(scenario_id.clone());
                                            }
                                        },
                                        if fixture { "Load" } else if available { "Run" } else { "Runner disabled" }
                                    }
                                    if let Some(reason) = &safety_reason {
                                        small { class: "control-disabled-reason", "Unavailable: {reason}" }
                                    }
                                }
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
                        aria_labelledby: "lab-confirmation-title",
                        onkeydown: move |event: KeyboardEvent| if event.key() == Key::Escape {
                            confirmation.set(None);
                        },
                        h2 { id: "lab-confirmation-title", "Confirm Protocol Lab action" }
                        p { "Target: {pending.target}" }
                        p { "Parameters: {pending.parameters}" }
                        p { "Consequence: {pending.consequence}" }
                        div { class: "confirmation-actions",
                            button { autofocus: true, onclick: move |_| confirmation.set(None), "Cancel" }
                            button {
                                class: "danger",
                                disabled: safety_policy.read().confirm(ControlPlane::Lab, &pending.action, pending.token).is_err(),
                                onclick: move |_| {
                                    match safety_policy.read().confirm(ControlPlane::Lab, &pending.action, pending.token) {
                                        Ok(()) => match &pending.pending {
                                            LabPending::Start(scenario_id) => on_start.call(scenario_id.clone()),
                                            LabPending::Cancel(run_id) => on_cancel.call(run_id.clone()),
                                        },
                                        Err(error) => tracing::warn!(target: "dx::safety", %error, "Lab confirmation rejected"),
                                    }
                                    confirmation.set(None);
                                },
                                "Confirm"
                            }
                        }
                        if let Err(reason) = safety_policy.read().confirm(ControlPlane::Lab, &pending.action, pending.token) {
                            p { class: "control-disabled-reason", "Confirmation unavailable: {reason}" }
                        }
                    }
                }
            }
        }
    }
}

fn scenario_action(profile: ScenarioProfile) -> SafetyAction {
    match profile {
        ScenarioProfile::Fixture => SafetyAction::fixture_scenario(),
        ScenarioProfile::LiveRunner => SafetyAction::live_scenario(),
    }
}

fn lab_confirmation(
    safety: &SafetyContext,
    target: String,
    parameters: &'static str,
    consequence: &'static str,
    pending: LabPending,
    action: SafetyAction,
) -> Option<LabConfirmation> {
    match safety.begin_confirmation(ControlPlane::Lab, &action) {
        Ok(token) => {
            Some(LabConfirmation { target, parameters, consequence, pending, action, token })
        }
        Err(error) => {
            tracing::warn!(target: "dx::safety", %error, "Lab action rejected");
            None
        }
    }
}

#[component]
fn EvidencePanel(title: &'static str, empty: &'static str, rows: Vec<String>) -> Element {
    rsx! {
        article { class: "lab-evidence-panel",
            h3 { "{title}" }
            if rows.is_empty() {
                p { class: "lab-empty", "{empty}" }
            } else {
                ul {
                    for row in rows { li { "{row}" } }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use styrene_interop_runner::{
        AssertionEvidence, CleanupEvidence, LogEvidence, RunEvidence, RunStatus, TimedEvidence,
        TopologyEvidence,
    };

    #[test]
    fn evidence_view_preserves_authoritative_failure_classes_and_redaction() {
        let evidence = RunEvidence {
            schema_version: 2,
            scenario_id: "direct".into(),
            correlation_id: "corr-7".into(),
            status: RunStatus::TimedOut,
            topology: TopologyEvidence {
                allocation_key: "allocation-7".into(),
                candidate: 7,
                host: "127.0.0.1".into(),
                ports: BTreeMap::from([("rust_rpc".into(), 30000)]),
                reservation_invariant: "bounded".into(),
            },
            revisions: Vec::new(),
            milestones: vec![TimedEvidence { name: "rust-ready".into(), elapsed_ms: 12 }],
            assertions: vec![AssertionEvidence {
                name: "delivery".into(),
                passed: false,
                detail: Some("deadline".into()),
                elapsed_ms: 90,
            }],
            artifacts: Vec::new(),
            artifact_rejections: Vec::new(),
            event_rejections: Vec::new(),
            timings_ms: BTreeMap::new(),
            logs: vec![LogEvidence {
                stream: "stdout".into(),
                text: "[REDACTED]".into(),
                bytes_seen: 128,
                truncated: true,
            }],
            cleanup: CleanupEvidence::default(),
            process_exit: None,
            failure: Some("deadline exceeded".into()),
        };
        let run = ScenarioRun {
            run_id: "corr-7".into(),
            scenario_id: "direct",
            status: ScenarioStatus::TimedOut,
            milestones: Vec::new(),
            evidence: Vec::new(),
            runner_evidence: Some(evidence),
        };

        let view = evidence_view(&run);

        assert!(view.topology.iter().any(|row| row.contains("rust_rpc: 127.0.0.1:30000")));
        assert!(view.milestones.iter().any(|row| row.contains("rust-ready at 12 ms")));
        assert!(view.assertions.iter().any(|row| row.contains("delivery: failed")));
        assert!(view.logs.iter().any(|row| row.contains("content [REDACTED]")));
        assert!(!format!("{view:?}").contains("fixture-secret"));
        assert_eq!(view.failure.as_deref(), Some("deadline exceeded"));
    }

    #[test]
    fn running_fixture_uses_declared_milestones_without_fabricating_runner_evidence() {
        let run = ScenarioRun {
            run_id: "fixture-1".into(),
            scenario_id: "fixture-routed",
            status: ScenarioStatus::Running,
            milestones: vec!["fixture loaded".into()],
            evidence: Vec::new(),
            runner_evidence: None,
        };
        let view = evidence_view(&run);
        assert_eq!(view.milestones, ["fixture loaded"]);
        assert!(view.topology.is_empty());
        assert!(view.assertions.is_empty());
    }
}
