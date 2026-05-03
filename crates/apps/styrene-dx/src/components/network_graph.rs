//! Network graph visualization — force-directed SVG layout with pan/zoom/drag.
//!
//! Design principles for large meshes (200+ nodes):
//! - LOD: labels hidden at overview zoom, revealed when zoomed in or for named nodes
//! - Edge declutter: edges shown only for selected node + neighbors
//! - Selection: click a node to inspect it, dims everything else
//! - Search: filter nodes by name or hash from the sidebar

use dioxus::prelude::*;

use crate::state::{
    GraphEdge, GraphNode, GraphNodeType, MeshStatusInfo, PathEntry, PeerEntry, PeerRole,
};

// ── Force-directed layout ─────────────────────────────────────────────────

fn force_tick(nodes: &mut [GraphNode], edges: &[GraphEdge], pinned: &[usize]) {
    let repulsion = 6000.0_f64;
    let spring_k = 0.008_f64;
    let spring_len = 150.0_f64;
    let center_gravity = 0.01_f64;
    let damping = 0.75_f64;

    let positions: Vec<(f64, f64)> = nodes.iter().map(|n| (n.x, n.y)).collect();
    for i in 0..nodes.len() {
        for j in (i + 1)..nodes.len() {
            let dx = positions[i].0 - positions[j].0;
            let dy = positions[i].1 - positions[j].1;
            let dist_sq = dx * dx + dy * dy;
            let dist = dist_sq.sqrt().max(1.0);
            let force = repulsion / dist_sq.max(1.0);
            let fx = (dx / dist) * force;
            let fy = (dy / dist) * force;
            nodes[i].vx += fx;
            nodes[i].vy += fy;
            nodes[j].vx -= fx;
            nodes[j].vy -= fy;
        }
    }

    for edge in edges {
        let (sx, sy) = (nodes[edge.source].x, nodes[edge.source].y);
        let (tx, ty) = (nodes[edge.target].x, nodes[edge.target].y);
        let dx = tx - sx;
        let dy = ty - sy;
        let dist = (dx * dx + dy * dy).sqrt().max(1.0);
        let displacement = dist - spring_len;
        let force = spring_k * displacement;
        let fx = (dx / dist) * force;
        let fy = (dy / dist) * force;
        nodes[edge.source].vx += fx;
        nodes[edge.source].vy += fy;
        nodes[edge.target].vx -= fx;
        nodes[edge.target].vy -= fy;
    }

    for node in nodes.iter_mut() {
        node.vx -= node.x * center_gravity;
        node.vy -= node.y * center_gravity;
    }

    for (i, node) in nodes.iter_mut().enumerate() {
        if pinned.contains(&i) {
            node.vx = 0.0;
            node.vy = 0.0;
            continue;
        }
        node.vx *= damping;
        node.vy *= damping;
        node.x += node.vx;
        node.y += node.vy;
    }
}

fn kinetic_energy(nodes: &[GraphNode]) -> f64 {
    nodes.iter().map(|n| n.vx * n.vx + n.vy * n.vy).sum()
}

fn build_graph(
    local_hash: &str,
    local_name: Option<&str>,
    peers: &[PeerEntry],
    _paths: &[PathEntry],
) -> (Vec<GraphNode>, Vec<GraphEdge>) {
    let mut nodes = Vec::new();
    let mut edges = Vec::new();

    // Layout: Local on the left, Interface next, Hub in center, mesh radiates right.
    // This creates a clear left-to-right flow: Me → Interface → Hub → Mesh.

    // Local node — left side
    nodes.push(GraphNode {
        id: local_hash.to_string(),
        label: local_name.unwrap_or("Local Node").to_string(),
        node_type: GraphNodeType::Local,
        capabilities: Vec::new(),
        x: -250.0,
        y: 0.0,
        vx: 0.0,
        vy: 0.0,
    });

    // Interface node — between local and hub
    let iface_idx = nodes.len();
    nodes.push(GraphNode {
        id: "iface:tcp-client".to_string(),
        label: "TCP Transport".to_string(),
        node_type: GraphNodeType::Interface { online: true },
        capabilities: Vec::new(),
        x: -150.0,
        y: 0.0,
        vx: 0.0,
        vy: 0.0,
    });
    edges.push(GraphEdge { source: 0, target: iface_idx, hops: 0 });

    // Find hub nodes and place them center-right
    let mut hub_indices: Vec<usize> = Vec::new();

    // Build peer nodes — hubs near center, mesh radiates to the right
    let peer_count = peers.len().max(1);
    for (i, peer) in peers.iter().enumerate() {
        // Spread peers in a semicircle to the right of the hub
        let t = i as f64 / peer_count as f64;
        let angle = (t - 0.5) * std::f64::consts::PI * 1.6; // ~±145 degrees, biased right
        let online = peer.status != "offline" && !peer.status.is_empty();

        let (base_x, base_y) = match &peer.node_role {
            PeerRole::Hub => (0.0, 0.0), // Hub at the center
            PeerRole::PageHost => {
                let r = 100.0;
                (r * angle.cos() + 50.0, r * angle.sin())
            }
            PeerRole::Styrene => {
                let r = 140.0;
                (r * angle.cos() + 60.0, r * angle.sin())
            }
            PeerRole::Rns if peer.name.is_some() => {
                let r = 180.0;
                (r * angle.cos() + 70.0, r * angle.sin())
            }
            PeerRole::Rns => {
                let r = 250.0;
                (r * angle.cos() + 80.0, r * angle.sin())
            }
        };

        let node_type = match &peer.node_role {
            PeerRole::Hub => GraphNodeType::Hub { online },
            PeerRole::PageHost => GraphNodeType::PageHost { online },
            PeerRole::Styrene => GraphNodeType::Styrene { online },
            PeerRole::Rns => GraphNodeType::Rns { online },
        };

        let idx = nodes.len();
        if peer.node_role == PeerRole::Hub {
            hub_indices.push(idx);
        }

        nodes.push(GraphNode {
            id: peer.hash.clone(),
            label: peer
                .name
                .clone()
                .unwrap_or_else(|| peer.hash[..8.min(peer.hash.len())].to_string()),
            node_type,
            capabilities: peer.capabilities.clone(),
            x: base_x,
            y: base_y,
            vx: 0.0,
            vy: 0.0,
        });
    }

    // Edges: Interface → Hub(s) → Peers
    if hub_indices.is_empty() {
        for i in (iface_idx + 1)..nodes.len() {
            edges.push(GraphEdge { source: iface_idx, target: i, hops: 1 });
        }
    } else {
        for &hub_idx in &hub_indices {
            edges.push(GraphEdge { source: iface_idx, target: hub_idx, hops: 1 });
        }
        let primary_hub = hub_indices[0];
        for i in (iface_idx + 1)..nodes.len() {
            if hub_indices.contains(&i) {
                continue;
            }
            edges.push(GraphEdge { source: primary_hub, target: i, hops: 1 });
        }
    }

    // Pin local and interface during force layout so they stay on the left
    let pinned: Vec<usize> = vec![0, iface_idx];
    for _ in 0..150 {
        force_tick(&mut nodes, &edges, &pinned);
    }
    (nodes, edges)
}

// ── Interaction state ─────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
enum Interaction {
    None,
    Panning { last_x: f64, last_y: f64 },
    DraggingNode { idx: usize, last_x: f64, last_y: f64 },
}

fn peer_key(peers: &[PeerEntry]) -> String {
    let mut parts: Vec<&str> = peers.iter().map(|p| p.hash.as_str()).collect();
    parts.sort();
    parts.join(",")
}

/// Whether a node is "important" enough to always show its label (even at overview zoom).
fn is_notable(node: &GraphNode) -> bool {
    matches!(
        node.node_type,
        GraphNodeType::Local
            | GraphNodeType::Interface { .. }
            | GraphNodeType::Hub { .. }
            | GraphNodeType::PageHost { .. }
    )
}

/// Whether a node should be visible with the default filter (hide unnamed RNS noise).
fn is_interesting(node: &GraphNode) -> bool {
    match &node.node_type {
        GraphNodeType::Local | GraphNodeType::Interface { .. } | GraphNodeType::Hub { .. } => true,
        GraphNodeType::PageHost { .. } | GraphNodeType::Styrene { .. } => true,
        GraphNodeType::Rns { .. } => node.name_is_set(),
    }
}

// ── Component ─────────────────────────────────────────────────────────────

#[component]
pub fn NetworkGraph(
    peers: Vec<PeerEntry>,
    paths: Vec<PathEntry>,
    status: MeshStatusInfo,
    local_hash: String,
    local_name: Option<String>,
    on_select_peer: EventHandler<String>,
) -> Element {
    // Graph data — rebuilt when peer membership or path table changes
    let mut nodes = use_signal(Vec::<GraphNode>::new);
    let mut edges = use_signal(Vec::<GraphEdge>::new);
    let mut last_peer_key = use_signal(String::new);

    let current_key = format!("{}|{}", peer_key(&peers), paths.len());
    if *last_peer_key.read() != current_key {
        let (new_nodes, new_edges) =
            build_graph(&local_hash, local_name.as_deref(), &peers, &paths);
        nodes.set(new_nodes);
        edges.set(new_edges);
        last_peer_key.set(current_key);
    }

    // Camera
    let mut cam_x = use_signal(|| 0.0_f64);
    let mut cam_y = use_signal(|| 0.0_f64);
    let mut zoom = use_signal(|| 1.0_f64);

    // Interaction
    let mut interaction = use_signal(|| Interaction::None);
    let mut container_size = use_signal(|| (800.0_f64, 600.0_f64));
    let mut tooltip = use_signal(|| None::<(f64, f64, String, String, String, String)>);
    let mut physics_on = use_signal(|| true);

    // Selection: index of selected node in the nodes vec
    let mut selected = use_signal(|| None::<usize>);

    // Search filter
    let mut search_query = use_signal(String::new);

    // Show all vs interesting-only filter
    let mut show_all = use_signal(|| false);

    // Build visible node set — filter out unnamed RNS noise by default
    let show_all_val = *show_all.read();
    let visible: Vec<bool> = {
        let ns = nodes.read();
        ns.iter().map(|n| show_all_val || is_interesting(n)).collect()
    };
    let visible_count = visible.iter().filter(|v| **v).count();
    let hidden_count = visible.len().saturating_sub(visible_count);

    // Physics loop
    let _physics = use_coroutine(move |_rx: UnboundedReceiver<()>| async move {
        loop {
            tokio::time::sleep(tokio::time::Duration::from_millis(33)).await;
            if !*physics_on.read() {
                continue;
            }
            let mut pinned = vec![0usize];
            if let Interaction::DraggingNode { idx, .. } = *interaction.read() {
                if !pinned.contains(&idx) {
                    pinned.push(idx);
                }
            }
            let edge_snapshot = edges.read().clone();
            let mut node_snapshot = nodes.read().clone();
            if node_snapshot.is_empty() {
                continue;
            }
            force_tick(&mut node_snapshot, &edge_snapshot, &pinned);
            let ke = kinetic_energy(&node_snapshot);
            if ke < 0.01 && *interaction.read() == Interaction::None {
                continue;
            }
            nodes.set(node_snapshot);
        }
    });

    // ViewBox
    let base_w = 800.0_f64;
    let base_h = 600.0_f64;
    let z = *zoom.read();
    let vb_w = base_w / z;
    let vb_h = base_h / z;
    let vb_x = *cam_x.read() - vb_w / 2.0;
    let vb_y = *cam_y.read() - vb_h / 2.0;
    let viewbox = format!("{vb_x} {vb_y} {vb_w} {vb_h}");

    let pixel_to_world = {
        let (cw, ch) = *container_size.read();
        (vb_w / cw.max(1.0), vb_h / ch.max(1.0))
    };

    // LOD thresholds
    let show_all_labels = z > 1.5;
    let show_named_labels = z > 0.6;

    // Stats
    let online_count =
        peers.iter().filter(|p| p.status != "offline" && !p.status.is_empty()).count();
    let total_count = peers.len();

    let hit_test = move |world_x: f64, world_y: f64| -> Option<usize> {
        let ns = nodes.read();
        for i in (0..ns.len()).rev() {
            let n = &ns[i];
            let dx = world_x - n.x;
            let dy = world_y - n.y;
            let hit_r = n.radius() + 8.0;
            if dx * dx + dy * dy <= hit_r * hit_r {
                return Some(i);
            }
        }
        None
    };

    // Build the set of node indices connected to the selected node (for edge visibility)
    let selected_neighbors: Vec<usize> = {
        if let Some(sel) = *selected.read() {
            edges
                .read()
                .iter()
                .filter_map(|e| {
                    if e.source == sel {
                        Some(e.target)
                    } else if e.target == sel {
                        Some(e.source)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            Vec::new()
        }
    };

    // Search match set
    let query = search_query.read().to_ascii_lowercase();
    let has_search = !query.is_empty();
    let search_matches: Vec<usize> = if has_search {
        nodes
            .read()
            .iter()
            .enumerate()
            .filter(|(_, n)| {
                n.label.to_ascii_lowercase().contains(&query)
                    || n.id.to_ascii_lowercase().contains(&query)
            })
            .map(|(i, _)| i)
            .collect()
    } else {
        Vec::new()
    };

    // Selected node detail for the sidebar
    let selected_detail: Option<GraphNode> =
        selected.read().and_then(|idx| nodes.read().get(idx).cloned());

    rsx! {
        div { class: "network-view",
            div {
                class: "graph-container",

                onmounted: move |evt| {
                    let data = evt.data();
                    spawn(async move {
                        if let Ok(rect) = data.get_client_rect().await {
                            container_size.set((rect.size.width, rect.size.height));
                        }
                    });
                },

                onmousedown: move |evt: MouseEvent| {
                    let client = evt.client_coordinates();
                    let el = evt.element_coordinates();
                    let (cw, ch) = *container_size.read();
                    let z = *zoom.read();
                    let bw = 800.0 / z;
                    let bh = 600.0 / z;
                    let wx = (*cam_x.read() - bw / 2.0) + (el.x / cw.max(1.0)) * bw;
                    let wy = (*cam_y.read() - bh / 2.0) + (el.y / ch.max(1.0)) * bh;

                    if let Some(idx) = hit_test(wx, wy) {
                        selected.set(Some(idx));
                        if *physics_on.read() {
                            let mut ns = nodes.write();
                            for edge in edges.read().iter() {
                                let other = if edge.source == idx {
                                    edge.target
                                } else if edge.target == idx {
                                    edge.source
                                } else {
                                    continue;
                                };
                                if let Some(n) = ns.get_mut(other) {
                                    n.vx += 0.1;
                                    n.vy += 0.1;
                                }
                            }
                        }
                        interaction.set(Interaction::DraggingNode {
                            idx,
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

                onmousemove: move |evt: MouseEvent| {
                    let client = evt.client_coordinates();
                    let current = interaction.read().clone();
                    match current {
                        Interaction::Panning { last_x, last_y } => {
                            let dx = (client.x - last_x) * pixel_to_world.0;
                            let dy = (client.y - last_y) * pixel_to_world.1;
                            cam_x -= dx;
                            cam_y -= dy;
                            interaction.set(Interaction::Panning {
                                last_x: client.x,
                                last_y: client.y,
                            });
                        }
                        Interaction::DraggingNode { idx, last_x, last_y } => {
                            let dx = (client.x - last_x) * pixel_to_world.0;
                            let dy = (client.y - last_y) * pixel_to_world.1;
                            let mut ns = nodes.write();
                            if let Some(node) = ns.get_mut(idx) {
                                node.x += dx;
                                node.y += dy;
                                node.vx = 0.0;
                                node.vy = 0.0;
                            }
                            drop(ns);
                            interaction.set(Interaction::DraggingNode {
                                idx,
                                last_x: client.x,
                                last_y: client.y,
                            });
                        }
                        Interaction::None => {}
                    }
                },

                onmouseup: move |_| {
                    interaction.set(Interaction::None);
                },

                onmouseleave: move |_| {
                    interaction.set(Interaction::None);
                    tooltip.set(None);
                },

                onwheel: move |evt: WheelEvent| {
                    let scroll_y = evt.delta().strip_units().y;
                    let factor = if scroll_y < 0.0 { 1.1 } else { 1.0 / 1.1 };
                    let new_zoom = (*zoom.read() * factor).clamp(0.15, 5.0);
                    zoom.set(new_zoom);
                },

                svg {
                    view_box: "{viewbox}",
                    xmlns: "http://www.w3.org/2000/svg",

                    // Edges
                    for edge in edges.read().iter() {
                        {
                            let src_vis = edge.source < visible.len() && visible[edge.source];
                            let tgt_vis = edge.target < visible.len() && visible[edge.target];
                            if !src_vis || !tgt_vis {
                                rsx! {}
                            } else {
                                let sel = *selected.read();
                                let ns = nodes.read();
                                let is_structural = edge.source < ns.len() && edge.target < ns.len() && {
                                    let src = &ns[edge.source].node_type;
                                    let tgt = &ns[edge.target].node_type;
                                    matches!(src, GraphNodeType::Local | GraphNodeType::Interface { .. } | GraphNodeType::Hub { .. })
                                    || matches!(tgt, GraphNodeType::Local | GraphNodeType::Interface { .. } | GraphNodeType::Hub { .. })
                                };
                                let show_edge = is_structural
                                    || match sel {
                                        Some(s) => edge.source == s || edge.target == s,
                                        None => z > 0.8,
                                    };
                                if show_edge {
                                    let src = &ns[edge.source];
                                    let tgt = &ns[edge.target];
                                    let is_online = src.node_type.is_online() || tgt.node_type.is_online();
                                    let class = if is_online { "graph-edge active" } else { "graph-edge inactive" };
                                    let (sx, sy, tx, ty) = (src.x, src.y, tgt.x, tgt.y);
                                    rsx! {
                                        line {
                                            class: "{class}",
                                            x1: "{sx}",
                                            y1: "{sy}",
                                            x2: "{tx}",
                                            y2: "{ty}",
                                        }
                                    }
                                } else {
                                    rsx! {}
                                }
                            }
                        }
                    }

                    // Nodes — skip hidden ones
                    {
                        let ns = nodes.read();
                        let sel = *selected.read();
                        rsx! {
                            for (idx, node) in ns.iter().enumerate() {
                                {
                                    let node_visible = idx < visible.len() && visible[idx];
                                    if !node_visible {
                                        rsx! {}
                                    } else {
                                    let color = node.color();
                                    let border = node.border_color();
                                    let r = node.radius();
                                    let nx = node.x;
                                    let ny = node.y;
                                    let label = node.label.clone();
                                    let hash_short = if node.id.len() > 12 {
                                        format!("{}...", &node.id[..12])
                                    } else {
                                        node.id.clone()
                                    };
                                    let is_local = node.node_type == GraphNodeType::Local;
                                    let label_offset = r + 14.0;

                                    let custom_path = node.shape_path(nx, ny);

                                    let tt_label = label.clone();
                                    let tt_hash = node.id.clone();
                                    let tt_type = node.type_label().to_string();
                                    let tt_caps = if node.capabilities.is_empty() {
                                        String::new()
                                    } else {
                                        node.capabilities.join(", ")
                                    };

                                    let icon = match &node.node_type {
                                        GraphNodeType::Local => Some("⬡"),
                                        GraphNodeType::Hub { .. } => Some("⬢"),
                                        GraphNodeType::PageHost { .. } => Some("☰"),
                                        GraphNodeType::Interface { .. } => None, // too small for an icon
                                        _ => None,
                                    };

                                    // Determine opacity: dim nodes that aren't related to selection or search
                                    let is_selected = sel == Some(idx);
                                    let is_neighbor = sel.is_some() && selected_neighbors.contains(&idx);
                                    let is_search_match = has_search && search_matches.contains(&idx);
                                    let is_dragging = matches!(*interaction.read(), Interaction::DraggingNode { idx: d, .. } if d == idx);

                                    let opacity = if is_dragging {
                                        "0.85"
                                    } else if has_search && !is_search_match {
                                        "0.15"
                                    } else if sel.is_some() && !is_selected && !is_neighbor && !is_local {
                                        "0.25"
                                    } else {
                                        "1"
                                    };

                                    // LOD: when to show labels
                                    let show_label = is_selected
                                        || is_neighbor
                                        || is_search_match
                                        || show_all_labels
                                        || (show_named_labels && is_notable(node))
                                        || (show_named_labels && node.name_is_set());

                                    rsx! {
                                        g {
                                            class: "graph-node",
                                            opacity: "{opacity}",

                                            onmouseenter: move |evt: MouseEvent| {
                                                if *interaction.read() == Interaction::None {
                                                    let coords = evt.client_coordinates();
                                                    tooltip.set(Some((coords.x, coords.y, tt_label.clone(), tt_hash.clone(), tt_type.clone(), tt_caps.clone())));
                                                }
                                            },
                                            onmouseleave: move |_| {
                                                if *interaction.read() == Interaction::None {
                                                    tooltip.set(None);
                                                }
                                            },

                                            // Glow for selected or important nodes
                                            if is_selected || (is_local || matches!(node.node_type, GraphNodeType::Hub { .. })) {
                                                circle {
                                                    cx: "{nx}",
                                                    cy: "{ny}",
                                                    r: "{r + 6.0}",
                                                    fill: "none",
                                                    stroke: if is_selected { "#fff" } else { color },
                                                    stroke_opacity: if is_selected { "0.5" } else { "0.2" },
                                                    stroke_width: if is_selected { "3" } else { "2" },
                                                }
                                            }

                                            // Node shape
                                            if let Some(ref d) = custom_path {
                                                path {
                                                    d: "{d}",
                                                    fill: "{color}",
                                                    stroke: if is_selected { "#fff" } else { border },
                                                    stroke_width: if is_selected { "3" } else { "2" },
                                                }
                                            } else {
                                                circle {
                                                    cx: "{nx}",
                                                    cy: "{ny}",
                                                    r: "{r}",
                                                    fill: "{color}",
                                                    stroke: if is_selected { "#fff" } else { border },
                                                    stroke_width: if is_selected { "3" } else { "2" },
                                                }
                                            }

                                            // Icon glyph
                                            if let Some(glyph) = icon {
                                                text {
                                                    x: "{nx}",
                                                    y: "{ny + 3.0}",
                                                    text_anchor: "middle",
                                                    fill: "white",
                                                    font_size: "10",
                                                    font_weight: "bold",
                                                    pointer_events: "none",
                                                    "{glyph}"
                                                }
                                            }

                                            // Label (LOD-controlled)
                                            if show_label {
                                                text {
                                                    class: "graph-label",
                                                    x: "{nx}",
                                                    y: "{ny + label_offset}",
                                                    "{label}"
                                                }
                                                if !is_local {
                                                    text {
                                                        class: "graph-label-hash",
                                                        x: "{nx}",
                                                        y: "{ny + label_offset + 13.0}",
                                                        "{hash_short}"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    } // else (visible)
                                }
                            }
                        }
                    }
                }

                // Tooltip overlay
                if let Some((tx, ty, label, hash, status_text, caps)) = tooltip.read().clone() {
                    div {
                        class: "graph-tooltip",
                        style: "left: {tx + 12.0}px; top: {ty - 10.0}px;",
                        div { class: "tt-label", "{label}" }
                        div { class: "tt-hash", "{hash}" }
                        div { class: "tt-status", "{status_text}" }
                        if !caps.is_empty() {
                            div { class: "tt-caps", "{caps}" }
                        }
                    }
                }
            }

            // ── Sidebar ───────────────────────────────────────────────────
            div { class: "graph-sidebar",

                // Search
                div {
                    h3 { "Search" }
                    input {
                        class: "graph-search",
                        r#type: "text",
                        placeholder: "Name or hash...",
                        value: "{search_query}",
                        oninput: move |evt| search_query.set(evt.value()),
                    }
                    if has_search {
                        {
                            let count = search_matches.len();
                            let suffix = if count == 1 { "" } else { "es" };
                            rsx! {
                                div { class: "search-count", "{count} match{suffix}" }
                            }
                        }
                    }
                }

                // Selected node detail
                if let Some(ref detail) = selected_detail {
                    div { class: "node-detail",
                        h3 { "Selected Node" }
                        div { class: "detail-name", "{detail.label}" }
                        div { class: "detail-hash", "{detail.id}" }
                        div { class: "detail-type", "{detail.type_label()}" }
                        if !detail.capabilities.is_empty() {
                            div { class: "detail-caps",
                                for cap in detail.capabilities.iter() {
                                    span { class: "cap-badge", "{cap}" }
                                }
                            }
                        }

                        // Action buttons based on node type
                        div { class: "detail-actions",
                            if !matches!(detail.node_type, GraphNodeType::Local) {
                                {
                                    let peer_hash = detail.id.clone();
                                    rsx! {
                                        button {
                                            class: "action-btn primary",
                                            onclick: move |_| {
                                                on_select_peer.call(peer_hash.clone());
                                            },
                                            "Message"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Visibility filter
                div {
                    h3 { "Filter" }
                    div {
                        class: "physics-toggle",
                        onclick: move |_| {
                            let current = *show_all.read();
                            show_all.set(!current);
                        },
                        span {
                            class: if *show_all.read() { "toggle-indicator on" } else { "toggle-indicator off" },
                        }
                        span { class: "toggle-label",
                            if *show_all.read() { "All Nodes" } else { "Named Only" }
                        }
                    }
                    if hidden_count > 0 {
                        div { class: "search-count",
                            "{hidden_count} anonymous nodes hidden"
                        }
                    }
                }

                // Layout toggle
                div {
                    h3 { "Layout" }
                    div {
                        class: "physics-toggle",
                        onclick: move |_| {
                            let currently_on = *physics_on.read();
                            if !currently_on {
                                let mut ns = nodes.write();
                                for node in ns.iter_mut() {
                                    node.vx += 0.5;
                                    node.vy += 0.5;
                                }
                            }
                            physics_on.set(!currently_on);
                        },
                        span {
                            class: if *physics_on.read() { "toggle-indicator on" } else { "toggle-indicator off" },
                        }
                        span { class: "toggle-label",
                            if *physics_on.read() { "Live Layout" } else { "Layout Paused" }
                        }
                    }
                }

                // Stats
                div {
                    h3 { "Network Stats" }
                    div { class: "graph-stat",
                        span { class: "graph-stat-label", "Peers" }
                        span { class: "graph-stat-value", "{total_count}" }
                    }
                    div { class: "graph-stat",
                        span { class: "graph-stat-label", "Online" }
                        span { class: "graph-stat-value", style: "color: var(--green);", "{online_count}" }
                    }
                    div { class: "graph-stat",
                        span { class: "graph-stat-label", "Links" }
                        span { class: "graph-stat-value", "{status.link_count}" }
                    }
                    div { class: "graph-stat",
                        span { class: "graph-stat-label", "Interfaces" }
                        span { class: "graph-stat-value", "{status.interface_count}" }
                    }
                    div { class: "graph-stat",
                        span { class: "graph-stat-label", "Transport" }
                        span { class: "graph-stat-value",
                            style: if status.transport_active { "color: var(--green);" } else { "color: var(--text-dim);" },
                            if status.transport_active { "Active" } else { "Inactive" }
                        }
                    }
                }

                // Legend
                div {
                    h3 { "Legend" }
                    div { class: "graph-legend",
                        div { class: "legend-item",
                            span { class: "legend-dot", style: "background: #58a6ff;" }
                            "Local ⬡"
                        }
                        div { class: "legend-item",
                            span { class: "legend-dot", style: "background: #3fb950; border-radius: 2px;" }
                            "Interface ◻"
                        }
                        div { class: "legend-item",
                            span { class: "legend-dot", style: "background: #bc8cff;" }
                            "Hub ◇"
                        }
                        div { class: "legend-item",
                            span { class: "legend-dot", style: "background: #39d2c0;" }
                            "Page Host"
                        }
                        div { class: "legend-item",
                            span { class: "legend-dot", style: "background: #3fb950;" }
                            "Styrene"
                        }
                        div { class: "legend-item",
                            span { class: "legend-dot", style: "background: #d29922;" }
                            "RNS"
                        }
                    }
                }
            }
        }
    }
}
