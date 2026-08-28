//! Page browser — NomadNet-compatible Micron page viewer.
//!
//! Displays the daemon's rendering projection and canonical source.

use crate::state::PageView;
use dioxus::prelude::*;
#[cfg(test)]
use styrene_ipc::PageAddress;
use styrene_ipc::types::{
    FileDownloadInfo, FileDownloadRequest, FileDownloadState, PageBrowseStage, PageBrowseStageKind,
    PageBrowseStageState, PageFormFieldKind, PageFormSubmission, PageNavigationAction,
    PageNavigationRequest,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PageMode {
    #[default]
    Rendered,
    Source,
}

fn stage_status(status: &PageBrowseStageState) -> String {
    match status {
        PageBrowseStageState::Pending => "Pending".into(),
        PageBrowseStageState::Succeeded => "Complete".into(),
        PageBrowseStageState::Failed { message, .. } => format!("Failed: {message}"),
        PageBrowseStageState::Skipped { reason } => format!("Skipped: {reason}"),
        _ => "Unknown".into(),
    }
}

fn stage_label(kind: PageBrowseStageKind) -> &'static str {
    match kind {
        PageBrowseStageKind::PathDiscovery => "Path discovery",
        PageBrowseStageKind::IdentityResolution => "Identity resolution",
        PageBrowseStageKind::LinkEstablishment => "Link establishment",
        PageBrowseStageKind::Identification => "Identification",
        PageBrowseStageKind::RequestSubmission => "Request submission",
        PageBrowseStageKind::Transfer => "Transfer",
        PageBrowseStageKind::Parse => "Parse",
        PageBrowseStageKind::Render => "Render",
        _ => "Unknown",
    }
}

fn stage_observation(stage: &PageBrowseStage) -> String {
    format!(
        "source={:?} evidence={:?} observed={} generation={} correlation={}",
        stage.observation.source,
        stage.evidence_source,
        stage.observation.observed_at.map_or_else(|| "pending".into(), |value| value.to_string()),
        stage
            .observation
            .connection_generation
            .map_or_else(|| "unreported".into(), |value| value.to_string()),
        stage.observation.correlation_id.as_deref().unwrap_or("unreported")
    )
}

fn format_bytes(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} B")
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn canonical_source(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap_or_else(|_| {
        let hex = bytes.iter().map(|byte| format!("{byte:02x}")).collect::<String>();
        format!("[non-UTF-8 canonical bytes; hexadecimal]\n{hex}")
    })
}

fn nonempty_session(session_id: &str) -> Option<String> {
    (!session_id.is_empty()).then(|| session_id.to_string())
}

fn initial_field_values(
    fields: &[styrene_ipc::types::PageFormField],
) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut values = std::collections::BTreeMap::<String, Vec<String>>::new();
    for field in fields {
        match field.kind {
            PageFormFieldKind::Text => {
                values.insert(field.name.clone(), vec![field.value.clone().unwrap_or_default()]);
            }
            PageFormFieldKind::Password => {
                values.insert(field.name.clone(), vec![String::new()]);
            }
            PageFormFieldKind::Checkbox | PageFormFieldKind::Radio if field.checked => {
                if let Some(value) = &field.value {
                    values.entry(field.name.clone()).or_default().push(value.clone());
                }
            }
            _ => {}
        }
    }
    values
}

fn link_submission(
    fields: &[String],
    values: &std::collections::BTreeMap<String, Vec<String>>,
) -> Option<PageFormSubmission> {
    if fields.is_empty() {
        return None;
    }
    let mut submission = PageFormSubmission::default();
    submission.values = fields
        .iter()
        .filter_map(|name| values.get(name).cloned().map(|value| (name.clone(), value)))
        .collect();
    Some(submission)
}

fn authoritative_field_reset(
    previous_correlation: &str,
    page: &styrene_ipc::types::PageContent,
) -> Option<(String, std::collections::BTreeMap<String, Vec<String>>)> {
    (previous_correlation != page.correlation_id)
        .then(|| (page.correlation_id.clone(), initial_field_values(&page.fields)))
}

#[cfg(test)]
fn resolve_navigation(url: &str, current: Option<&PageAddress>) -> Option<PageAddress> {
    current.and_then(|base| PageAddress::resolve(url, base).ok())
}

// ── Component ─────────────────────────────────────────────────────────────

#[component]
pub fn PageBrowser(
    page: Option<PageView>,
    download: Option<FileDownloadInfo>,
    available: bool,
    unavailable_reason: Option<String>,
    on_navigate: EventHandler<PageNavigationRequest>,
    on_close: EventHandler<String>,
    on_download: EventHandler<FileDownloadRequest>,
    on_refresh_download: EventHandler<String>,
    on_cancel_download: EventHandler<String>,
    on_save_download: EventHandler<(String, String)>,
) -> Element {
    let mut url_input = use_signal(|| String::from("/page/index.mu"));
    let mut last_page_url = use_signal(String::new);
    let mut last_page_correlation = use_signal(String::new);
    let mut page_mode = use_signal(PageMode::default);
    let mut field_values = use_signal(std::collections::BTreeMap::<String, Vec<String>>::new);
    let mut page_session = use_signal(String::new);
    let mut save_path = use_signal(String::new);

    if let Some(ref pv) = page {
        let current_url =
            if pv.host.is_empty() { pv.path.clone() } else { format!("{}:{}", pv.host, pv.path) };
        if *last_page_url.read() != current_url {
            url_input.set(current_url.clone());
            last_page_url.set(current_url);
        }
        if let Some((correlation, values)) = pv
            .authoritative
            .as_ref()
            .and_then(|page| authoritative_field_reset(&last_page_correlation.read(), page))
        {
            last_page_correlation.set(correlation);
            field_values.set(values);
        }
    } else if !page_session.read().is_empty() {
        page_session.set(String::new());
    }

    let navigation =
        page.as_ref().and_then(|page| page.authoritative.as_ref()).map(|page| &page.navigation);
    let can_back = navigation.is_some_and(|state| state.can_back);
    let can_forward = navigation.is_some_and(|state| state.can_forward);
    if let Some(state) = navigation
        && *page_session.read() != state.session_id
    {
        page_session.set(state.session_id.clone());
    }
    let is_loading = page.as_ref().map(|p| p.loading).unwrap_or(false);

    rsx! {
        if let Some(reason) = unavailable_reason.as_deref() {
            p { id: "page-browser-disabled-reason", class: "control-disabled-reason", "Page controls disabled: {reason}" }
        }
        fieldset {
            class: "page-browser",
            disabled: !available,
            aria_label: "Page browser",
            aria_describedby: (!available).then_some("page-browser-disabled-reason"),
            div { class: "page-nav-bar",
                button {
                    class: "page-nav-btn",
                    aria_label: "Back",
                    disabled: !can_back,
                    onclick: move |_| {
                        let mut request = PageNavigationRequest::default();
                        request.session_id = nonempty_session(&page_session.read());
                        request.action = PageNavigationAction::Back;
                        on_navigate.call(request);
                    },
                    "<"
                }
                button {
                    class: "page-nav-btn",
                    aria_label: "Forward",
                    disabled: !can_forward,
                    onclick: move |_| {
                        let mut request = PageNavigationRequest::default();
                        request.session_id = nonempty_session(&page_session.read());
                        request.action = PageNavigationAction::Forward;
                        on_navigate.call(request);
                    },
                    ">"
                }
                button {
                    class: "page-nav-btn",
                    disabled: is_loading,
                    onclick: move |_| {
                        let mut request = PageNavigationRequest::default();
                        request.session_id = nonempty_session(&page_session.read());
                        request.action = PageNavigationAction::Reload;
                        on_navigate.call(request);
                    },
                    if is_loading { "..." } else { "Reload" }
                }
                button {
                    class: "page-nav-btn",
                    disabled: is_loading,
                    onclick: move |_| {
                        let mut request = PageNavigationRequest::default();
                        request.session_id = nonempty_session(&page_session.read());
                        request.target = Some(last_page_url.read().clone());
                        request.bypass_cache = true;
                        on_navigate.call(request);
                    },
                    "Bypass cache"
                }
                button {
                    class: "page-nav-btn",
                    disabled: page_session.read().is_empty(),
                    onclick: move |_| {
                        if let Some(session) = nonempty_session(&page_session.read()) {
                            on_close.call(session);
                        }
                    },
                    "Close"
                }
                input {
                    class: "page-url-input",
                    r#type: "text",
                    aria_label: "Page address",
                    placeholder: "destination-hash:/page/path.mu",
                    value: "{url_input}",
                    oninput: move |evt| url_input.set(evt.value()),
                    onkeypress: move |evt: KeyboardEvent| {
                        if evt.key() == Key::Enter {
                            let target = url_input.read().clone();
                            if !target.trim().is_empty() {
                                let mut request = PageNavigationRequest::default();
                                request.session_id = nonempty_session(&page_session.read());
                                request.target = Some(target);
                                on_navigate.call(request);
                            }
                        }
                    },
                }
                button {
                    class: "page-go-btn",
                    onclick: move |_| {
                        let target = url_input.read().clone();
                        if !target.trim().is_empty() {
                            let mut request = PageNavigationRequest::default();
                            request.session_id = nonempty_session(&page_session.read());
                            request.target = Some(target);
                            on_navigate.call(request);
                        }
                    },
                    "Go"
                }
                div { class: "page-view-toggle",
                    button {
                        class: if *page_mode.read() == PageMode::Rendered { "active" } else { "" },
                        onclick: move |_| page_mode.set(PageMode::Rendered),
                        "Rendered"
                    }
                    button {
                        class: if *page_mode.read() == PageMode::Source { "active" } else { "" },
                        disabled: page.as_ref().and_then(|page| page.authoritative.as_ref()).is_none(),
                        onclick: move |_| page_mode.set(PageMode::Source),
                        "Source"
                    }
                }
            }

            if let Some(ref page) = page {
                div { class: "page-diagnostics",
                    div { class: "page-stage-list",
                        for stage in page.authoritative.iter().flat_map(|page| &page.stages) {
                            div {
                                class: match &stage.state {
                                    PageBrowseStageState::Succeeded => "page-stage complete",
                                    PageBrowseStageState::Failed { .. } => "page-stage failed",
                                    PageBrowseStageState::Pending => "page-stage pending",
                                    PageBrowseStageState::Skipped { .. } => "page-stage unreported",
                                    _ => "page-stage unreported",
                                },
                                span { class: "page-stage-dot" }
                                span { "{stage_label(stage.kind)}" }
                                small { "{stage_status(&stage.state)}" }
                                small { "{stage_observation(stage)}" }
                            }
                        }
                    }
                    div { class: "page-request-metrics",
                        span { "Time: " {page.elapsed_ms.map(|value| format!("{value} ms")).unwrap_or_else(|| "pending".into())} }
                        span { "Bytes: " {page.authoritative.as_ref().map(|value| format_bytes(value.transfer.received_bytes)).unwrap_or_else(|| "unknown".into())} }
                        span { "Fetched: " {page.authoritative.as_ref().map(|value| value.fetched_at.to_string()).unwrap_or_else(|| "not reported".into())} }
                        span { "Cache: " {page.authoritative.as_ref().map(|value| format!("{:?}", value.cache.status)).unwrap_or_else(|| "unknown".into())} }
                        span { "Outcome: " {page.authoritative.as_ref().map(|value| format!("{:?}", value.outcome)).unwrap_or_else(|| "unknown".into())} }
                        if let Some(value) = page.authoritative.as_ref() {
                            span { "Correlation: {value.correlation_id}" }
                            span { "Checksum: {value.source_checksum}" }
                            span { "Request: " {value.request.request_id.as_deref().unwrap_or("unreported")} }
                            span { "Link: " {value.request.link_id.as_deref().unwrap_or("unreported")} }
                            span { "Path hash: {value.request.path_hash}" }
                            span { "Transfer: {value.transfer.kind:?} verified={value.transfer.verified}" }
                            span { "Resource: " {value.transfer.resource_hash.as_deref().unwrap_or("none")} }
                            span { "Cache origin: " {value.cache.origin_correlation_id.as_deref().unwrap_or("none")} }
                            if let Some(failure) = value.failure.as_ref() {
                                span { "Failure: {failure.code} retryable={failure.retryable}" }
                            }
                            for warning in &value.parser_warnings {
                                span { "Parser warning {warning.code}: {warning.message}" }
                            }
                        }
                    }
                }
            }

            div { class: "page-content",
                match page {
                    Some(ref pv) if pv.loading => rsx! {
                        div { class: "page-loading", "Loading page..." }
                    },
                    Some(ref pv) if pv.error.is_some() => rsx! {
                        div { class: "page-error",
                            h3 { "Page Error" }
                            p { "{pv.error.as_deref().unwrap_or(\"Unknown error\")}" }
                            p { class: "page-error-path", "{pv.host}:{pv.path}" }
                            div { class: "page-error-actions",
                                if pv.retryable {
                                    button {
                                        class: "action-btn primary",
                                        onclick: {
                                            let host = pv.host.clone();
                                            let path = pv.path.clone();
                                            move |_| {
                                                let url = if host.is_empty() {
                                                    path.clone()
                                                } else {
                                                    format!("{host}:{path}")
                                                };
                                                let mut request = PageNavigationRequest::default();
                                                request.session_id = nonempty_session(&page_session.read());
                                                request.target = Some(url);
                                                request.bypass_cache = true;
                                                on_navigate.call(request);
                                            }
                                        },
                                        "Retry"
                                    }
                                }
                                span { "Inspect the failed stage above and broker diagnostics in System." }
                            }
                        }
                    },
                    Some(ref pv) if pv.authoritative.is_some() => {
                        let authoritative = &pv.authoritative.as_slice()[0];
                        let source = canonical_source(&authoritative.source_bytes);
                        if source.trim().is_empty() {
                            let host_display = if pv.host.is_empty() { "local".to_string() } else { pv.host[..12.min(pv.host.len())].to_string() };
                            rsx! {
                                div { class: "page-empty",
                                    h3 { "No Content" }
                                    p { "The node at {host_display} returned an empty page for {pv.path}" }
                                    p { class: "page-hint", "The host may not have pages configured, or the path doesn't exist." }
                                }
                            }
                        } else if *page_mode.read() == PageMode::Source {
                            rsx! { pre { class: "page-source", "{source}" } }
                        } else {
                            rsx! {
                                pre { class: "micron-page", "{authoritative.rendered_text}" }
                                if !authoritative.fields.is_empty() {
                                    div { class: "page-fields",
                                        for field in &authoritative.fields {
                                            label {
                                                "{field.name}"
                                                input {
                                                    r#type: match field.kind {
                                                        PageFormFieldKind::Password => "password",
                                                        PageFormFieldKind::Checkbox => "checkbox",
                                                        PageFormFieldKind::Radio => "radio",
                                                        _ => "text",
                                                    },
                                                    checked: field.value.as_ref().is_some_and(|value| field_values.read().get(&field.name).is_some_and(|values| values.contains(value))),
                                                    value: field_values.read().get(&field.name).and_then(|values| values.first()).cloned().unwrap_or_default(),
                                                    oninput: {
                                                        let name = field.name.clone();
                                                        let kind = field.kind;
                                                        let field_value = field.value.clone().unwrap_or_default();
                                                        move |event: FormEvent| {
                                                            if kind == PageFormFieldKind::Radio {
                                                                if event.checked() {
                                                                    field_values.write().insert(name.clone(), vec![field_value.clone()]);
                                                                }
                                                            } else if kind == PageFormFieldKind::Checkbox {
                                                                if event.checked() {
                                                                    let mut values = field_values.write();
                                                                    let selected = values.entry(name.clone()).or_default();
                                                                    if !selected.contains(&field_value) { selected.push(field_value.clone()); }
                                                                } else {
                                                                    if let Some(selected) = field_values.write().get_mut(&name) {
                                                                        selected.retain(|value| value != &field_value);
                                                                    }
                                                                }
                                                            } else {
                                                                field_values.write().insert(name.clone(), vec![event.value()]);
                                                            }
                                                        }
                                                    },
                                                }
                                            }
                                        }
                                    }
                                }
                                for link in &authoritative.link_targets {
                                    button {
                                        class: "page-link",
                                        onclick: {
                                            let target = link.target.clone();
                                            let submitted_fields = link.submitted_fields.clone();
                                            move |_| {
                                                let submission = link_submission(
                                                    &submitted_fields,
                                                    &field_values.read(),
                                                );
                                                if target.contains("/file/") {
                                                    let mut request = FileDownloadRequest::default();
                                                    request.session_id = nonempty_session(&page_session.read());
                                                    request.target = target.clone();
                                                    on_download.call(request);
                                                } else {
                                                    let mut request = PageNavigationRequest::default();
                                                    request.session_id = nonempty_session(&page_session.read());
                                                    request.target = Some(target.clone());
                                                    request.submission = submission;
                                                    on_navigate.call(request);
                                                }
                                            }
                                        },
                                        {link.label.as_deref().unwrap_or(&link.target)}
                                    }
                                }
                            }
                        }
                    },
                    _ => rsx! {
                        div { class: "page-empty",
                            h3 { "Page Browser" }
                            p { "Enter a page address above or click a Page Host node in the Network view." }
                            button {
                                class: "action-btn primary",
                                style: "margin-top: 12px;",
                                onclick: move |_| {
                                    let mut request = PageNavigationRequest::default();
                                    request.target = Some("/page/index.mu".into());
                                    on_navigate.call(request);
                                },
                                "Browse Local Pages"
                            }
                        }
                    },
                }
            }
            if let Some(download) = download {
                div { class: "page-download",
                    strong { "Download: {download.native_path}" }
                    p { "State: {download.state:?} | {download.received_bytes}/{download.total_bytes} bytes | progress={download.progress}" }
                    p {
                        "Correlation: {download.correlation_id} | Transfer: {download.transfer:?} | Resource: "
                        {download.resource_hash.as_deref().unwrap_or("none")}
                    }
                    p { "SHA-256: " {download.sha256.as_deref().unwrap_or("pending")} }
                    p { "Integrity: {download.integrity_verified}" }
                    if let Some(error) = download.error.as_deref() {
                        p { class: "page-error", "Failure: {error}" }
                    }
                    button {
                        onclick: {
                            let id = download.download_id.clone();
                            move |_| on_refresh_download.call(id.clone())
                        },
                        "Refresh"
                    }
                    if !download.state.is_terminal() {
                        button {
                            onclick: {
                                let id = download.download_id.clone();
                                move |_| on_cancel_download.call(id.clone())
                            },
                            "Cancel"
                        }
                    }
                    if download.state == FileDownloadState::Completed && download.integrity_verified {
                        input {
                            r#type: "text",
                            aria_label: "Save destination",
                            placeholder: "/path/to/save",
                            value: "{save_path}",
                            oninput: move |event| save_path.set(event.value()),
                        }
                        button {
                            disabled: save_path.read().trim().is_empty(),
                            onclick: {
                                let id = download.download_id.clone();
                                move |_| on_save_download.call((id.clone(), save_path.read().clone()))
                            },
                            "Save verified file"
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_view_distinguishes_failure_and_missing_telemetry() {
        assert_eq!(
            stage_status(&PageBrowseStageState::Skipped { reason: "not attempted".into() }),
            "Skipped: not attempted"
        );
        assert_eq!(
            stage_status(&PageBrowseStageState::Failed {
                code: "identity_resolution_failed".into(),
                message: "identity unavailable".into(),
            }),
            "Failed: identity unavailable"
        );
    }

    #[test]
    fn browser_navigation_uses_typed_native_address_resolution() {
        let current = PageAddress::parse("0123456789abcdef0123456789abcdef:/page/docs/start.mu")
            .expect("valid current page");

        assert_eq!(
            resolve_navigation(":/page/about.mu", Some(&current))
                .map(|address| address.to_string()),
            Some("0123456789abcdef0123456789abcdef:/page/about.mu".into())
        );
        assert_eq!(
            resolve_navigation("next.mu", Some(&current)).map(|address| address.to_string()),
            Some("0123456789abcdef0123456789abcdef:/page/docs/next.mu".into())
        );
        assert!(resolve_navigation(":/file/archive.bin", Some(&current)).is_none());
        assert!(resolve_navigation("ambiguous:page", Some(&current)).is_none());
    }

    #[test]
    fn page_change_state_initializes_shared_checks_and_redacts_passwords() {
        let mut password = styrene_ipc::types::PageFormField::default();
        password.name = "password".into();
        password.kind = PageFormFieldKind::Password;
        let mut red = styrene_ipc::types::PageFormField::default();
        red.name = "opts".into();
        red.kind = PageFormFieldKind::Checkbox;
        red.value = Some("red".into());
        red.checked = true;
        let mut blue = red.clone();
        blue.value = Some("blue".into());
        blue.checked = false;

        let values = initial_field_values(&[password, red, blue]);
        assert_eq!(values["password"], [""]);
        assert_eq!(values["opts"], ["red"]);
        let mut submission = PageFormSubmission::default();
        submission.values = values;
        assert!(!format!("{submission:?}").contains("red"));
    }

    #[test]
    fn ordinary_links_send_nil_while_form_links_submit_only_declared_fields() {
        let values = std::collections::BTreeMap::from([
            ("name".into(), vec!["Ada".into()]),
            ("password".into(), vec!["secret".into()]),
        ]);

        assert!(link_submission(&[], &values).is_none());
        let submission = link_submission(&["name".into()], &values).expect("form submission");
        assert_eq!(submission.values.len(), 1);
        assert_eq!(submission.values["name"], ["Ada"]);
        assert!(!submission.values.contains_key("password"));
    }

    #[test]
    fn same_url_authoritative_reload_resets_password_field_state() {
        let mut password = styrene_ipc::types::PageFormField::default();
        password.name = "password".into();
        password.kind = PageFormFieldKind::Password;
        let mut first = styrene_ipc::types::PageContent::default();
        first.correlation_id = "load-1".into();
        first.navigation.address = "/page/index.mu".into();
        first.fields.push(password.clone());
        let (correlation, mut values) = authoritative_field_reset("", &first).unwrap();
        values.insert("password".into(), vec!["secret".into()]);

        let mut reloaded = styrene_ipc::types::PageContent::default();
        reloaded.correlation_id = "load-2".into();
        reloaded.navigation.address = first.navigation.address.clone();
        reloaded.fields.push(password);
        let (_, values) = authoritative_field_reset(&correlation, &reloaded).unwrap();

        assert_eq!(values["password"], [""]);
    }
}
