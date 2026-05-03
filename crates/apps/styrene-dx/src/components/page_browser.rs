//! Page browser — NomadNet-compatible Micron page viewer.
//!
//! Renders Micron markup fetched from local or remote nodes.
//! Supports: headings, paragraphs, links, and basic formatting.

use dioxus::prelude::*;

use crate::state::PageView;

/// A parsed Micron element for rendering.
#[derive(Clone, Debug)]
enum MicronElement {
    Heading { level: u8, text: String },
    Paragraph { text: String },
    Link { text: String, target: String },
    Separator,
    Preformatted { text: String },
}

/// Parse Micron markup into renderable elements.
fn parse_micron(source: &str) -> Vec<MicronElement> {
    let mut elements = Vec::new();

    for line in source.lines() {
        let trimmed = line.trim();

        if trimmed.is_empty() {
            continue;
        }

        // Heading: lines starting with > (more > = deeper heading)
        if trimmed.starts_with('>') {
            let level = trimmed.chars().take_while(|c| *c == '>').count() as u8;
            let text = trimmed[level as usize..].trim().to_string();
            // Strip Micron color codes like `F444` from the text
            let text = strip_micron_codes(&text);
            if !text.is_empty() {
                elements.push(MicronElement::Heading { level, text });
            }
            continue;
        }

        // Separator: lines that are all dashes or equals
        if trimmed.len() >= 3
            && (trimmed.chars().all(|c| c == '-') || trimmed.chars().all(|c| c == '='))
        {
            elements.push(MicronElement::Separator);
            continue;
        }

        // Links: `[display text]`target`
        if trimmed.contains('`') && trimmed.contains('[') {
            if let Some(link) = parse_micron_link(trimmed) {
                elements.push(link);
                continue;
            }
        }

        // Preformatted: lines starting with a space
        if line.starts_with("    ") || line.starts_with('\t') {
            elements.push(MicronElement::Preformatted { text: line.to_string() });
            continue;
        }

        // Regular paragraph text — strip color codes
        let text = strip_micron_codes(trimmed);
        if !text.is_empty() {
            elements.push(MicronElement::Paragraph { text });
        }
    }

    elements
}

/// Strip Micron inline formatting codes like `Faaa`, `faaa`, `B444`, etc.
fn strip_micron_codes(s: &str) -> String {
    let mut result = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '`' {
            // Check if this is a formatting code: `X...`
            // Format codes are: F (foreground), f (reset fg), B (background), b (reset bg),
            // and various single-char codes
            if let Some(&next) = chars.peek() {
                if "FfBb!".contains(next) {
                    // Skip until next backtick or end
                    chars.next(); // consume the code letter
                                  // Consume hex digits or content until space/end
                    while let Some(&c) = chars.peek() {
                        if c == '`' || c == ' ' {
                            break;
                        }
                        chars.next();
                    }
                    continue;
                }
            }
            // Not a format code, keep the backtick
            result.push(ch);
        } else {
            result.push(ch);
        }
    }

    result.trim().to_string()
}

/// Parse a Micron link: `[Display Text]`/path/to/page`
fn parse_micron_link(line: &str) -> Option<MicronElement> {
    let open = line.find('[')?;
    let close = line[open..].find(']')? + open;
    let text = line[open + 1..close].to_string();

    // Look for the target in backticks after the bracket
    let after = &line[close + 1..];
    let target_start = after.find('`')?;
    let rest = &after[target_start + 1..];
    let target_end = rest.find('`').unwrap_or(rest.len());
    let target = rest[..target_end].to_string();

    let text = strip_micron_codes(&text);

    Some(MicronElement::Link { text, target })
}

#[component]
pub fn PageBrowser(page: Option<PageView>, on_navigate: EventHandler<String>) -> Element {
    let mut url_input = use_signal(|| String::from("/"));

    rsx! {
        div { class: "page-browser",
            // URL bar
            div { class: "page-url-bar",
                input {
                    class: "page-url-input",
                    r#type: "text",
                    placeholder: "hash:/path or /local/path",
                    value: "{url_input}",
                    oninput: move |evt| url_input.set(evt.value()),
                    onkeypress: move |evt: KeyboardEvent| {
                        if evt.key() == Key::Enter {
                            let url = url_input.read().clone();
                            if !url.trim().is_empty() {
                                on_navigate.call(url);
                            }
                        }
                    },
                }
                button {
                    class: "page-go-btn",
                    onclick: move |_| {
                        let url = url_input.read().clone();
                        if !url.trim().is_empty() {
                            on_navigate.call(url);
                        }
                    },
                    "Go"
                }
            }

            // Page content
            div { class: "page-content",
                match page {
                    Some(ref pv) if pv.loading => rsx! {
                        div { class: "page-loading", "Loading page..." }
                    },
                    Some(ref pv) if pv.error.is_some() => rsx! {
                        div { class: "page-error",
                            h3 { "Page Error" }
                            p { "{pv.error.as_deref().unwrap_or(\"Unknown error\")}" }
                            p { class: "page-error-path",
                                "{pv.host}:{pv.path}"
                            }
                        }
                    },
                    Some(ref pv) if pv.content.is_some() => {
                        let elements = parse_micron(pv.content.as_deref().unwrap_or(""));
                        let host = pv.host.clone();
                        rsx! {
                            div { class: "micron-page",
                                for elem in elements.iter() {
                                    {match elem {
                                        MicronElement::Heading { level, text } => {
                                            let class = format!("micron-h{}", level.min(&3));
                                            rsx! { div { class: "{class}", "{text}" } }
                                        }
                                        MicronElement::Paragraph { text } => {
                                            rsx! { p { class: "micron-p", "{text}" } }
                                        }
                                        MicronElement::Link { text, target } => {
                                            let nav_url = if target.starts_with('/') && !host.is_empty() {
                                                format!("{host}:{target}")
                                            } else {
                                                target.clone()
                                            };
                                            rsx! {
                                                a {
                                                    class: "micron-link",
                                                    href: "#",
                                                    onclick: move |evt: MouseEvent| {
                                                        evt.prevent_default();
                                                        on_navigate.call(nav_url.clone());
                                                    },
                                                    "{text}"
                                                }
                                            }
                                        }
                                        MicronElement::Separator => {
                                            rsx! { hr { class: "micron-hr" } }
                                        }
                                        MicronElement::Preformatted { text } => {
                                            rsx! { pre { class: "micron-pre", "{text}" } }
                                        }
                                    }}
                                }
                            }
                        }
                    },
                    _ => rsx! {
                        div { class: "page-empty",
                            h3 { "Page Browser" }
                            p { "Enter a page address above or click a Page Host node in the Network view." }
                            p { class: "page-hint", "Try \"/\" to browse local pages." }
                        }
                    },
                }
            }
        }
    }
}
