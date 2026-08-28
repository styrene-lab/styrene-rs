//! Combined topology coordinator. Model, layout, rendering, interaction, filters,
//! and inspection live behind separate component/module boundaries.

use dioxus::prelude::*;

use crate::state::{GraphEdge, GraphNode, GraphNodeType, MeshStatusInfo, PathEntry, PeerEntry};

use super::network_inspector::NetworkInspector;
use super::network_interaction::{self, Interaction};
use super::network_renderer::{GraphRenderer, GraphTooltip};
use super::{network_layout, network_model};

#[component]
pub fn NetworkGraph(
    peers: Vec<PeerEntry>,
    paths: Vec<PathEntry>,
    status: MeshStatusInfo,
    local_hash: String,
    local_name: Option<String>,
    on_select_peer: EventHandler<String>,
    on_browse_page: EventHandler<String>,
    links: Vec<crate::state::LinkInfo>,
    interfaces: Vec<crate::state::InterfaceInfo>,
) -> Element {
    let mut nodes = use_signal(Vec::<GraphNode>::new);
    let mut edges = use_signal(Vec::<GraphEdge>::new);
    let mut model_key = use_signal(String::new);
    let current_key = format!("{}|{:?}|{:?}|{:?}", peer_key(&peers), paths, interfaces, links);
    if *model_key.read() != current_key {
        let (mut next_nodes, next_edges) = network_model::build_graph(
            &local_hash,
            local_name.as_deref(),
            &peers,
            &paths,
            &interfaces,
            &links,
            &[],
        );
        network_model::preserve_positions(&nodes.read(), &mut next_nodes);
        nodes.set(next_nodes);
        edges.set(next_edges);
        model_key.set(current_key);
    }

    let mut camera_x = use_signal(|| 0.0_f64);
    let mut camera_y = use_signal(|| 0.0_f64);
    let mut zoom = use_signal(|| 1.0_f64);
    let mut interaction = use_signal(|| Interaction::None);
    let mut container_size = use_signal(|| (800.0_f64, 600.0_f64));
    let mut tooltip = use_signal(|| None::<GraphTooltip>);
    let physics_on = use_signal(initial_physics_enabled);
    let mut selected = use_signal(|| None::<usize>);
    let search_query = use_signal(String::new);
    let show_all = use_signal(|| false);

    let visible: Vec<bool> =
        nodes.read().iter().map(|node| *show_all.read() || is_interesting(node)).collect();
    let hidden_count = visible.iter().filter(|visible| !**visible).count();

    let _physics = use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(33)).await;
            if !*physics_on.read() {
                continue;
            }
            let mut pinned = vec![0];
            if let Interaction::DraggingNode { idx, .. } = *interaction.read()
                && !pinned.contains(&idx)
            {
                pinned.push(idx);
            }
            let edge_snapshot = edges.read().clone();
            let mut node_snapshot = nodes.read().clone();
            if node_snapshot.is_empty() {
                continue;
            }
            network_layout::force_tick(&mut node_snapshot, &edge_snapshot, &pinned);
            if network_layout::kinetic_energy(&node_snapshot) < 0.01
                && *interaction.read() == Interaction::None
            {
                continue;
            }
            nodes.set(node_snapshot);
        }
    });

    let zoom_value = *zoom.read();
    let width = 800.0 / zoom_value;
    let height = 600.0 / zoom_value;
    let viewbox = format!(
        "{} {} {width} {height}",
        *camera_x.read() - width / 2.0,
        *camera_y.read() - height / 2.0
    );
    let pixel_to_world = {
        let (container_width, container_height) = *container_size.read();
        (width / container_width.max(1.0), height / container_height.max(1.0))
    };

    let selected_neighbors: Vec<usize> = selected
        .read()
        .and_then(|selected| {
            Some(
                edges
                    .read()
                    .iter()
                    .filter_map(|edge| {
                        if edge.source == selected {
                            Some(edge.target)
                        } else if edge.target == selected {
                            Some(edge.source)
                        } else {
                            None
                        }
                    })
                    .collect(),
            )
        })
        .unwrap_or_default();
    let query = search_query.read().to_ascii_lowercase();
    let search_matches: Vec<usize> = if query.is_empty() {
        Vec::new()
    } else {
        nodes
            .read()
            .iter()
            .enumerate()
            .filter(|(_, node)| {
                node.label.to_ascii_lowercase().contains(&query)
                    || node.id.to_ascii_lowercase().contains(&query)
            })
            .map(|(index, _)| index)
            .collect()
    };
    let selected_detail = selected.read().and_then(|index| nodes.read().get(index).cloned());

    rsx! {
        div { class: "network-view",
            div {
                class: "graph-container",
                onmounted: move |event| {
                    let data = event.data();
                    spawn(async move {
                        if let Ok(rect) = data.get_client_rect().await {
                            container_size.set((rect.size.width, rect.size.height));
                        }
                    });
                },
                onmousedown: move |event: MouseEvent| {
                    let client = event.client_coordinates();
                    let element = event.element_coordinates();
                    let (container_width, container_height) = *container_size.read();
                    let zoom_value = *zoom.read();
                    let width = 800.0 / zoom_value;
                    let height = 600.0 / zoom_value;
                    let world_x = (*camera_x.read() - width / 2.0)
                        + (element.x / container_width.max(1.0)) * width;
                    let world_y = (*camera_y.read() - height / 2.0)
                        + (element.y / container_height.max(1.0)) * height;
                    if let Some(index) = network_interaction::hit_test(&nodes.read(), world_x, world_y) {
                        selected.set(Some(index));
                        interaction.set(Interaction::DraggingNode {
                            idx: index,
                            last_x: client.x,
                            last_y: client.y,
                        });
                    } else {
                        selected.set(None);
                        interaction.set(Interaction::Panning {
                            last_x: client.x,
                            last_y: client.y,
                        });
                    }
                    tooltip.set(None);
                },
                onmousemove: move |event: MouseEvent| {
                    let client = event.client_coordinates();
                    let current_interaction = interaction.read().clone();
                    match current_interaction {
                        Interaction::Panning { last_x, last_y } => {
                            camera_x -= (client.x - last_x) * pixel_to_world.0;
                            camera_y -= (client.y - last_y) * pixel_to_world.1;
                            interaction.set(Interaction::Panning {
                                last_x: client.x,
                                last_y: client.y,
                            });
                        }
                        Interaction::DraggingNode { idx, last_x, last_y } => {
                            if let Some(node) = nodes.write().get_mut(idx) {
                                node.x += (client.x - last_x) * pixel_to_world.0;
                                node.y += (client.y - last_y) * pixel_to_world.1;
                                node.vx = 0.0;
                                node.vy = 0.0;
                            }
                            interaction.set(Interaction::DraggingNode {
                                idx,
                                last_x: client.x,
                                last_y: client.y,
                            });
                        }
                        Interaction::None => {}
                    }
                },
                onmouseup: move |_| interaction.set(Interaction::None),
                onmouseleave: move |_| {
                    interaction.set(Interaction::None);
                    tooltip.set(None);
                },
                onwheel: move |event: WheelEvent| {
                    let factor = if event.delta().strip_units().y < 0.0 { 1.1 } else { 1.0 / 1.1 };
                    let current_zoom = *zoom.read();
                    zoom.set((current_zoom * factor).clamp(0.15, 5.0));
                },
                GraphRenderer {
                    nodes,
                    edges,
                    visible,
                    selected,
                    selected_neighbors,
                    search_matches: search_matches.clone(),
                    has_search: !query.is_empty(),
                    zoom: zoom_value,
                    viewbox,
                    interaction,
                    tooltip,
                }
                if let Some((x, y, label, hash, status, capabilities)) = tooltip.read().clone() {
                    div {
                        class: "graph-tooltip",
                        style: "left: {x + 12.0}px; top: {y - 10.0}px;",
                        div { class: "tt-label", "{label}" }
                        div { class: "tt-hash", "{hash}" }
                        div { class: "tt-status", "{status}" }
                        if !capabilities.is_empty() {
                            div { class: "tt-caps", "{capabilities}" }
                        }
                    }
                }
            }
            NetworkInspector {
                selected: selected_detail,
                peers,
                links,
                status,
                search_query,
                search_count: search_matches.len(),
                hidden_count,
                show_all,
                physics_on,
                nodes,
                on_select_peer,
                on_browse_page,
            }
        }
    }
}

const fn initial_physics_enabled() -> bool {
    // Start static so platform reduced-motion preferences are honored without script detection.
    false
}

fn peer_key(peers: &[PeerEntry]) -> String {
    let mut hashes: Vec<&str> = peers.iter().map(|peer| peer.hash.as_str()).collect();
    hashes.sort_unstable();
    hashes.join(",")
}

fn is_interesting(node: &GraphNode) -> bool {
    match &node.node_type {
        GraphNodeType::Local | GraphNodeType::Interface { .. } | GraphNodeType::Hub { .. } => true,
        GraphNodeType::PageHost { .. } | GraphNodeType::Styrene { .. } => true,
        GraphNodeType::Rns { .. } => node.name_is_set(),
    }
}

#[cfg(test)]
mod accessibility_tests {
    use super::*;

    #[test]
    fn force_layout_requires_explicit_operator_opt_in() {
        assert!(!initial_physics_enabled());
    }
}
