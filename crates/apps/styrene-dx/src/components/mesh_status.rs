use dioxus::prelude::*;

#[component]
pub fn MeshStatus() -> Element {
    // Placeholder — will connect to live transport state
    rsx! {
        div { class: "card",
            h2 { "Mesh Status" }
            div { class: "status-grid",
                div { class: "stat",
                    span { class: "stat-value", "0" }
                    span { class: "stat-label", "Interfaces" }
                }
                div { class: "stat",
                    span { class: "stat-value", "0" }
                    span { class: "stat-label", "Known Paths" }
                }
                div { class: "stat",
                    span { class: "stat-value", "—" }
                    span { class: "stat-label", "Transport" }
                }
            }
        }
    }
}
