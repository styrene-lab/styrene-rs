//! Page browser — NomadNet-compatible Micron page viewer.
//!
//! Uses `styrene_micron::parse()` for spec-compliant parsing,
//! then renders the document model to Dioxus RSX.

use dioxus::prelude::*;
use styrene_micron::{Alignment, Block, ChildBlock, InlineNode, Line};

use crate::state::PageView;

// ── Rendering helpers ─────────────────────────────────────────────────────

/// Convert a 3-char hex color (Micron spec) to a CSS color string.
fn micron_color(hex3: &str) -> String {
    if hex3.len() == 3 {
        let chars: Vec<char> = hex3.chars().collect();
        format!("#{0}{0}{1}{1}{2}{2}", chars[0], chars[1], chars[2])
    } else {
        format!("#{hex3}")
    }
}

/// CSS class for alignment.
fn align_class(a: &Alignment) -> &'static str {
    match a {
        Alignment::Center => "micron-center",
        Alignment::Right => "micron-right",
        _ => "",
    }
}

// ── Component ─────────────────────────────────────────────────────────────

#[component]
pub fn PageBrowser(page: Option<PageView>, on_navigate: EventHandler<String>) -> Element {
    let mut url_input = use_signal(|| String::from("/"));
    let mut last_page_url = use_signal(String::new);
    let mut history = use_signal(Vec::<String>::new);
    let mut history_pos = use_signal(|| 0_usize);

    if let Some(ref pv) = page {
        let current_url =
            if pv.host.is_empty() { pv.path.clone() } else { format!("{}:{}", pv.host, pv.path) };
        if *last_page_url.read() != current_url {
            url_input.set(current_url.clone());
            let pos = *history_pos.read();
            let mut h = history.write();
            h.truncate(pos);
            h.push(current_url.clone());
            drop(h);
            history_pos.set(pos + 1);
            last_page_url.set(current_url);
        }
    }

    let can_back = *history_pos.read() > 1;
    let can_forward = *history_pos.read() < history.read().len();
    let is_loading = page.as_ref().map(|p| p.loading).unwrap_or(false);

    rsx! {
        div { class: "page-browser",
            div { class: "page-nav-bar",
                button {
                    class: "page-nav-btn",
                    disabled: !can_back,
                    onclick: move |_| {
                        let pos = *history_pos.read();
                        if pos > 1 {
                            history_pos.set(pos - 1);
                            let url = history.read()[pos - 2].clone();
                            url_input.set(url.clone());
                            on_navigate.call(url);
                        }
                    },
                    "<"
                }
                button {
                    class: "page-nav-btn",
                    disabled: !can_forward,
                    onclick: move |_| {
                        let pos = *history_pos.read();
                        let len = history.read().len();
                        if pos < len {
                            history_pos.set(pos + 1);
                            let url = history.read()[pos].clone();
                            url_input.set(url.clone());
                            on_navigate.call(url);
                        }
                    },
                    ">"
                }
                button {
                    class: "page-nav-btn",
                    disabled: is_loading,
                    onclick: move |_| {
                        let url = url_input.read().clone();
                        if !url.trim().is_empty() { on_navigate.call(url); }
                    },
                    if is_loading { "..." } else { "Reload" }
                }
                input {
                    class: "page-url-input",
                    r#type: "text",
                    placeholder: "hash:/path or /local/path",
                    value: "{url_input}",
                    oninput: move |evt| url_input.set(evt.value()),
                    onkeypress: move |evt: KeyboardEvent| {
                        if evt.key() == Key::Enter {
                            let url = url_input.read().clone();
                            if !url.trim().is_empty() { on_navigate.call(url); }
                        }
                    },
                }
                button {
                    class: "page-go-btn",
                    onclick: move |_| {
                        let url = url_input.read().clone();
                        if !url.trim().is_empty() { on_navigate.call(url); }
                    },
                    "Go"
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
                        }
                    },
                    Some(ref pv) if pv.content.is_some() => {
                        let source = pv.content.as_deref().unwrap_or("");
                        if source.trim().is_empty() {
                            let host_display = if pv.host.is_empty() { "local".to_string() } else { pv.host[..12.min(pv.host.len())].to_string() };
                            rsx! {
                                div { class: "page-empty",
                                    h3 { "No Content" }
                                    p { "The node at {host_display} returned an empty page for {pv.path}" }
                                    p { class: "page-hint", "The host may not have pages configured, or the path doesn't exist." }
                                }
                            }
                        } else {
                            let doc = styrene_micron::parse(source);
                            let host = pv.host.clone();
                            rsx! {
                                div { class: "micron-page",
                                    for block in doc.blocks.iter() {
                                        {render_block(block, &host, on_navigate)}
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
                                onclick: move |_| { on_navigate.call("/".to_string()); },
                                "Browse Local Pages"
                            }
                        }
                    },
                }
            }
        }
    }
}

/// Render a top-level Block to RSX.
fn render_block(block: &Block, host: &str, on_navigate: EventHandler<String>) -> Element {
    match block {
        Block::Section { level, heading, children } => {
            let class = format!("micron-h{}", (*level).min(3));
            rsx! {
                div { class: "micron-section",
                    if let Some(h) = heading {
                        div { class: "{class}", {render_line_inline(h, host, on_navigate)} }
                    }
                    div { class: "micron-section-body",
                        for child in children.iter() {
                            {render_child_block(child, host, on_navigate)}
                        }
                    }
                }
            }
        }
        Block::Line(line) => render_line(line, host, on_navigate),
        Block::EmptyLine => rsx! { div { class: "micron-spacer" } },
        Block::Divider { .. } => rsx! { hr { class: "micron-hr" } },
        Block::Literal { content } => rsx! { pre { class: "micron-pre", "{content}" } },
        Block::Directive { .. } => rsx! {}, // hidden
    }
}

/// Render a ChildBlock (inside a Section).
fn render_child_block(
    block: &ChildBlock,
    host: &str,
    on_navigate: EventHandler<String>,
) -> Element {
    match block {
        ChildBlock::Section { level, heading, children } => {
            let class = format!("micron-h{}", (*level).min(3));
            rsx! {
                div { class: "micron-section",
                    if let Some(h) = heading {
                        div { class: "{class}", {render_line_inline(h, host, on_navigate)} }
                    }
                    div { class: "micron-section-body",
                        for child in children.iter() {
                            {render_child_block(child, host, on_navigate)}
                        }
                    }
                }
            }
        }
        ChildBlock::Line(line) => render_line(line, host, on_navigate),
        ChildBlock::EmptyLine => rsx! { div { class: "micron-spacer" } },
        ChildBlock::Divider { .. } => rsx! { hr { class: "micron-hr" } },
        ChildBlock::Literal { content } => rsx! { pre { class: "micron-pre", "{content}" } },
    }
}

/// Render a Line as a paragraph with inline nodes.
fn render_line(line: &Line, host: &str, on_navigate: EventHandler<String>) -> Element {
    let align = align_class(&line.alignment);
    let class = if align.is_empty() { "micron-p".to_string() } else { format!("micron-p {align}") };
    rsx! {
        p { class: "{class}", {render_line_inline(line, host, on_navigate)} }
    }
}

/// Render inline nodes of a Line.
fn render_line_inline(line: &Line, host: &str, on_navigate: EventHandler<String>) -> Element {
    rsx! {
        for node in line.nodes.iter() {
            {match node {
                InlineNode::Text { style, text } => {
                    let css = build_inline_css(style);
                    if css.is_empty() {
                        rsx! { span { "{text}" } }
                    } else {
                        rsx! { span { style: "{css}", "{text}" } }
                    }
                }
                InlineNode::Link { label, url, .. } => {
                    let display = label.as_deref().unwrap_or(url).to_string();
                    // Micron URLs: "HASH:/path" for remote, ":/path" for local
                    let clean_url = url.strip_prefix(':').unwrap_or(url);
                    let nav_url = if clean_url.starts_with('/') && !host.is_empty() {
                        // Local-relative path on a remote host — prepend host
                        format!("{host}:{clean_url}")
                    } else if clean_url.starts_with('/') {
                        // Local path
                        clean_url.to_string()
                    } else {
                        // Remote: "HASH:/path" — pass through as-is
                        clean_url.to_string()
                    };
                    rsx! {
                        a {
                            class: "micron-link",
                            href: "#",
                            onclick: move |evt: MouseEvent| {
                                evt.prevent_default();
                                on_navigate.call(nav_url.clone());
                            },
                            "{display}"
                        }
                    }
                }
                InlineNode::Newline => rsx! { br {} },
                InlineNode::Field { .. } => rsx! { span { class: "micron-field", "[field]" } },
            }}
        }
    }
}

/// Build CSS string from a StyleSet.
fn build_inline_css(style: &styrene_micron::StyleSet) -> String {
    let mut css = String::new();
    if style.has_bold() {
        css.push_str("font-weight:700;");
    }
    if style.has_italic() {
        css.push_str("font-style:italic;");
    }
    if style.has_underline() {
        css.push_str("text-decoration:underline;");
    }
    if let Some(fg) = style.fg_color() {
        css.push_str(&format!("color:{};", micron_color(fg)));
    }
    if let Some(bg) = style.bg_color() {
        css.push_str(&format!("background:{};", micron_color(bg)));
    }
    css
}
