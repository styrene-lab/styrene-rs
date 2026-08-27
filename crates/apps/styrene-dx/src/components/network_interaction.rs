use crate::state::GraphNode;

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Interaction {
    None,
    Panning { last_x: f64, last_y: f64 },
    DraggingNode { idx: usize, last_x: f64, last_y: f64 },
}

pub(crate) fn hit_test(nodes: &[GraphNode], world_x: f64, world_y: f64) -> Option<usize> {
    for index in (0..nodes.len()).rev() {
        let node = &nodes[index];
        let dx = world_x - node.x;
        let dy = world_y - node.y;
        let radius = node.radius() + 8.0;
        if dx * dx + dy * dy <= radius * radius {
            return Some(index);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::GraphNodeType;

    #[test]
    fn hit_testing_prefers_topmost_node_and_respects_radius() {
        let node = |id: &str| GraphNode {
            id: id.into(),
            label: id.into(),
            node_type: GraphNodeType::Rns { online: true },
            capabilities: Vec::new(),
            x: 10.0,
            y: 10.0,
            vx: 0.0,
            vy: 0.0,
        };
        let nodes = vec![node("lower"), node("upper")];
        assert_eq!(hit_test(&nodes, 10.0, 10.0), Some(1));
        assert_eq!(hit_test(&nodes, 100.0, 100.0), None);
    }
}
