use crate::state::{GraphEdge, GraphNode};

pub(crate) fn force_tick(nodes: &mut [GraphNode], edges: &[GraphEdge], pinned: &[usize]) {
    let repulsion = 6000.0_f64;
    let spring_k = 0.008_f64;
    let spring_len = 150.0_f64;
    let center_gravity = 0.01_f64;
    let damping = 0.75_f64;

    let positions: Vec<(f64, f64)> = nodes.iter().map(|node| (node.x, node.y)).collect();
    for index in 0..nodes.len() {
        for other in (index + 1)..nodes.len() {
            let dx = positions[index].0 - positions[other].0;
            let dy = positions[index].1 - positions[other].1;
            let distance_squared = dx * dx + dy * dy;
            let distance = distance_squared.sqrt().max(1.0);
            let force = repulsion / distance_squared.max(1.0);
            let force_x = (dx / distance) * force;
            let force_y = (dy / distance) * force;
            nodes[index].vx += force_x;
            nodes[index].vy += force_y;
            nodes[other].vx -= force_x;
            nodes[other].vy -= force_y;
        }
    }

    for edge in edges {
        let (source_x, source_y) = (nodes[edge.source].x, nodes[edge.source].y);
        let (target_x, target_y) = (nodes[edge.target].x, nodes[edge.target].y);
        let dx = target_x - source_x;
        let dy = target_y - source_y;
        let distance = (dx * dx + dy * dy).sqrt().max(1.0);
        let force = spring_k * (distance - spring_len);
        let force_x = (dx / distance) * force;
        let force_y = (dy / distance) * force;
        nodes[edge.source].vx += force_x;
        nodes[edge.source].vy += force_y;
        nodes[edge.target].vx -= force_x;
        nodes[edge.target].vy -= force_y;
    }

    for node in nodes.iter_mut() {
        node.vx -= node.x * center_gravity;
        node.vy -= node.y * center_gravity;
    }

    for (index, node) in nodes.iter_mut().enumerate() {
        if pinned.contains(&index) {
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

pub(crate) fn kinetic_energy(nodes: &[GraphNode]) -> f64 {
    nodes.iter().map(|node| node.vx * node.vx + node.vy * node.vy).sum()
}

pub(crate) fn settle_layout(nodes: &mut [GraphNode], edges: &[GraphEdge], pinned: &[usize]) {
    for _ in 0..layout_iterations(nodes.len()) {
        force_tick(nodes, edges, pinned);
    }
}

pub(crate) fn layout_iterations(node_count: usize) -> usize {
    if node_count > 200 { 8 } else { 150 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GraphNodeType;

    #[test]
    fn pinned_nodes_remain_stationary() {
        let mut nodes = vec![GraphNode {
            id: "local".into(),
            label: "Local".into(),
            node_type: GraphNodeType::Local,
            capabilities: Vec::new(),
            x: 10.0,
            y: 20.0,
            vx: 5.0,
            vy: 5.0,
        }];
        force_tick(&mut nodes, &[], &[0]);
        assert_eq!((nodes[0].x, nodes[0].y), (10.0, 20.0));
        assert_eq!((nodes[0].vx, nodes[0].vy), (0.0, 0.0));
    }

    #[test]
    fn high_cardinality_layout_has_a_bounded_iteration_budget() {
        assert_eq!(layout_iterations(501), 8);
        assert_eq!(layout_iterations(200), 150);
    }
}
