use dioxus::prelude::*;

use crate::state::{GraphEdge, GraphEdgeKind, GraphNode, GraphNodeType};

use super::network_interaction::Interaction;

pub(crate) type GraphTooltip = (f64, f64, String, String, String, String);

#[component]
pub(crate) fn GraphRenderer(
    nodes: Signal<Vec<GraphNode>>,
    edges: Signal<Vec<GraphEdge>>,
    visible: Vec<bool>,
    mut selected: Signal<Option<usize>>,
    selected_neighbors: Vec<usize>,
    search_matches: Vec<usize>,
    has_search: bool,
    zoom: f64,
    viewbox: String,
    interaction: Signal<Interaction>,
    mut tooltip: Signal<Option<GraphTooltip>>,
) -> Element {
    let show_all_labels = zoom > 1.5;
    let show_named_labels = zoom > 0.6;
    rsx! {
        svg {
            view_box: "{viewbox}",
            xmlns: "http://www.w3.org/2000/svg",
            role: "group",
            "aria-label": "Interactive mesh topology. Tab to a node and press Enter or Space to inspect it.",
            for edge in edges.read().iter() {
                {
                    let source_visible = edge.source < visible.len() && visible[edge.source];
                    let target_visible = edge.target < visible.len() && visible[edge.target];
                    if !source_visible || !target_visible {
                        rsx! {}
                    } else {
                        let selected_index = *selected.read();
                        let nodes = nodes.read();
                        let structural = edge.source < nodes.len() && edge.target < nodes.len() && {
                            let source = &nodes[edge.source].node_type;
                            let target = &nodes[edge.target].node_type;
                            matches!(source, GraphNodeType::Local | GraphNodeType::Interface { .. } | GraphNodeType::Hub { .. })
                                || matches!(target, GraphNodeType::Local | GraphNodeType::Interface { .. } | GraphNodeType::Hub { .. })
                        };
                        let show = structural || match selected_index {
                            Some(index) => edge.source == index || edge.target == index,
                            None => zoom > 0.8,
                        };
                        if show {
                            let source = &nodes[edge.source];
                            let target = &nodes[edge.target];
                            let state_class = if source.node_type.is_online() || target.node_type.is_online() {
                                "active"
                            } else {
                                "inactive"
                            };
                            let kind_class = match edge.kind {
                                GraphEdgeKind::Interface => "interface",
                                GraphEdgeKind::Route => "route",
                                GraphEdgeKind::Link => "link",
                                GraphEdgeKind::Association => "association",
                            };
                            let class = format!("graph-edge {state_class} {kind_class}");
                            rsx! {
                                line {
                                    class: "{class}",
                                    x1: "{source.x}",
                                    y1: "{source.y}",
                                    x2: "{target.x}",
                                    y2: "{target.y}",
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                }
            }
            {
                let nodes = nodes.read();
                let selected_index = *selected.read();
                rsx! {
                    for (index, node) in nodes.iter().enumerate() {
                        if index < visible.len() && visible[index] {
                            {
                                let color = node.color();
                                let border = node.border_color();
                                let radius = node.radius();
                                let x = node.x;
                                let y = node.y;
                                let label = node.label.clone();
                                let short_hash = if node.id.len() > 12 {
                                    format!("{}...", &node.id[..12])
                                } else {
                                    node.id.clone()
                                };
                                let local = node.node_type == GraphNodeType::Local;
                                let label_offset = radius + 14.0;
                                let custom_path = node.shape_path(x, y);
                                let tooltip_label = label.clone();
                                let tooltip_hash = node.id.clone();
                                let tooltip_type = node.type_label().to_string();
                                let tooltip_capabilities = node.capabilities.join(", ");
                                let accessible_label = format!(
                                    "{}; {}; {}",
                                    node.label,
                                    node.type_label(),
                                    if node.node_type.is_online() { "online" } else { "offline" }
                                );
                                let icon = match &node.node_type {
                                    GraphNodeType::Local => Some("⬡"),
                                    GraphNodeType::Hub { .. } => Some("⬢"),
                                    GraphNodeType::PageHost { .. } => Some("☰"),
                                    _ => None,
                                };
                                let is_selected = selected_index == Some(index);
                                let neighbor = selected_index.is_some() && selected_neighbors.contains(&index);
                                let search_match = has_search && search_matches.contains(&index);
                                let dragging = matches!(*interaction.read(), Interaction::DraggingNode { idx, .. } if idx == index);
                                let opacity = if dragging {
                                    "0.85"
                                } else if has_search && !search_match {
                                    "0.15"
                                } else if selected_index.is_some() && !is_selected && !neighbor && !local {
                                    "0.25"
                                } else {
                                    "1"
                                };
                                let show_label = is_selected
                                    || neighbor
                                    || search_match
                                    || show_all_labels
                                    || (show_named_labels && is_notable(node))
                                    || (show_named_labels && node.name_is_set());
                                rsx! {
                                    g {
                                        class: "graph-node",
                                        opacity: "{opacity}",
                                        role: "button",
                                        tabindex: "0",
                                        "aria-label": "{accessible_label}",
                                        "aria-pressed": if is_selected { "true" } else { "false" },
                                        onkeydown: move |event: KeyboardEvent| {
                                            let key = event.key();
                                            if key == Key::Enter
                                                || matches!(key, Key::Character(value) if value == " ")
                                            {
                                                selected.set(Some(index));
                                            }
                                        },
                                        onmouseenter: move |event: MouseEvent| {
                                            if *interaction.read() == Interaction::None {
                                                let coordinates = event.client_coordinates();
                                                tooltip.set(Some((
                                                    coordinates.x,
                                                    coordinates.y,
                                                    tooltip_label.clone(),
                                                    tooltip_hash.clone(),
                                                    tooltip_type.clone(),
                                                    tooltip_capabilities.clone(),
                                                )));
                                            }
                                        },
                                        onmouseleave: move |_| {
                                            if *interaction.read() == Interaction::None {
                                                tooltip.set(None);
                                            }
                                        },
                                        if is_selected || local || matches!(node.node_type, GraphNodeType::Hub { .. }) {
                                            circle {
                                                cx: "{x}", cy: "{y}", r: "{radius + 6.0}", fill: "none",
                                                stroke: if is_selected { "#fff" } else { color },
                                                stroke_opacity: if is_selected { "0.5" } else { "0.2" },
                                                stroke_width: if is_selected { "3" } else { "2" },
                                            }
                                        }
                                        if let Some(ref path) = custom_path {
                                            path {
                                                d: "{path}", fill: "{color}",
                                                stroke: if is_selected { "#fff" } else { border },
                                                stroke_width: if is_selected { "3" } else { "2" },
                                            }
                                        } else {
                                            circle {
                                                cx: "{x}", cy: "{y}", r: "{radius}", fill: "{color}",
                                                stroke: if is_selected { "#fff" } else { border },
                                                stroke_width: if is_selected { "3" } else { "2" },
                                            }
                                        }
                                        if let Some(glyph) = icon {
                                            text {
                                                x: "{x}", y: "{y + 3.0}", text_anchor: "middle",
                                                fill: "white", font_size: "10", font_weight: "bold",
                                                pointer_events: "none", "{glyph}"
                                            }
                                        }
                                        if show_label {
                                            text { class: "graph-label", x: "{x}", y: "{y + label_offset}", "{label}" }
                                            if !local {
                                                text {
                                                    class: "graph-label-hash", x: "{x}",
                                                    y: "{y + label_offset + 13.0}", "{short_hash}"
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

fn is_notable(node: &GraphNode) -> bool {
    matches!(
        node.node_type,
        GraphNodeType::Local
            | GraphNodeType::Interface { .. }
            | GraphNodeType::Hub { .. }
            | GraphNodeType::PageHost { .. }
    )
}
