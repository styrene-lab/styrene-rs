use dioxus::prelude::*;

#[component]
pub fn MeshStatus(
    transport_active: bool,
    peer_count: u32,
    link_count: u32,
    interface_count: u32,
) -> Element {
    let transport_label = if transport_active { "Active" } else { "Inactive" };
    let transport_class = if transport_active { "stat-value active" } else { "stat-value" };

    rsx! {
        div { class: "card",
            h2 { "Mesh Status" }
            div { class: "status-grid",
                div { class: "stat",
                    span { class: "{transport_class}", "{transport_label}" }
                    span { class: "stat-label", "Transport" }
                }
                div { class: "stat",
                    span { class: "stat-value", "{peer_count}" }
                    span { class: "stat-label", "Peers" }
                }
                div { class: "stat",
                    span { class: "stat-value", "{link_count}" }
                    span { class: "stat-label", "Links" }
                }
                div { class: "stat",
                    span { class: "stat-value", "{interface_count}" }
                    span { class: "stat-label", "Interfaces" }
                }
            }
        }
    }
}
