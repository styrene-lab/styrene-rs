use styrene_a2a::{
    AgentGraphEdge, AgentSnapshot, AgentSnapshotRequest, AgentTaskSnapshot, CancellationState,
    GraphEdgeRelationship, RootOperationId, RuntimeId, SequenceWatermark, SnapshotValidationError,
    TaskState, MAX_AGENT_SNAPSHOT_TASKS,
};

fn root() -> RootOperationId {
    RootOperationId::new("root-1").unwrap()
}

fn snapshot() -> AgentSnapshot {
    AgentSnapshot {
        root_operation_id: root(),
        generated_at_ms: 1_700_000_000_000,
        runtimes: vec![RuntimeId::from_bytes([0x11; 16]), RuntimeId::from_bytes([0x22; 16])],
        tasks: vec![
            AgentTaskSnapshot {
                task_id: "task-root".to_owned(),
                agent_id: "styrene:agent:root".to_owned(),
                runtime_id: RuntimeId::from_bytes([0x11; 16]),
                state: TaskState::Working,
                cancellation: CancellationState::default(),
                effective_grant_reference: Some("grant-root".to_owned()),
            },
            AgentTaskSnapshot {
                task_id: "task-child".to_owned(),
                agent_id: "styrene:agent:child".to_owned(),
                runtime_id: RuntimeId::from_bytes([0x22; 16]),
                state: TaskState::Submitted,
                cancellation: CancellationState::default(),
                effective_grant_reference: Some("grant-child".to_owned()),
            },
        ],
        edges: vec![AgentGraphEdge {
            parent_task_id: "task-root".to_owned(),
            child_task_id: "task-child".to_owned(),
            relationship: GraphEdgeRelationship::Delegate,
        }],
        watermarks: vec![SequenceWatermark {
            source_runtime_id: RuntimeId::from_bytes([0x11; 16]),
            stream_id: "task-root".to_owned(),
            contiguous_through: 7,
            highest_observed: 9,
        }],
    }
}

#[test]
fn snapshot_request_requires_root_and_carries_known_watermarks() {
    let request = AgentSnapshotRequest {
        root_operation_id: root(),
        known_watermarks: vec![SequenceWatermark {
            source_runtime_id: RuntimeId::from_bytes([0x11; 16]),
            stream_id: "task-root".to_owned(),
            contiguous_through: 7,
            highest_observed: 9,
        }],
    };

    assert!(request.validate().is_ok());
    let json = serde_json::to_vec(&request).unwrap();
    assert_eq!(serde_json::from_slice::<AgentSnapshotRequest>(&json).unwrap(), request);
}

#[test]
fn watermark_preserves_gap_state_and_rejects_impossible_ranges() {
    let gap = SequenceWatermark {
        source_runtime_id: RuntimeId::from_bytes([0x11; 16]),
        stream_id: "task-root".to_owned(),
        contiguous_through: 7,
        highest_observed: 9,
    };
    assert!(gap.has_gap());
    assert!(gap.validate().is_ok());

    let impossible = SequenceWatermark { contiguous_through: 10, ..gap };
    assert_eq!(impossible.validate(), Err(SnapshotValidationError::InvalidWatermark));
}

#[test]
fn snapshot_validates_graph_references_and_unique_watermarks() {
    let valid = snapshot();
    assert!(valid.validate().is_ok());

    let mut dangling = valid.clone();
    dangling.edges[0].child_task_id = "missing".to_owned();
    assert_eq!(dangling.validate(), Err(SnapshotValidationError::DanglingGraphEdge));

    let mut duplicate = valid.clone();
    duplicate.watermarks.push(duplicate.watermarks[0].clone());
    assert_eq!(duplicate.validate(), Err(SnapshotValidationError::DuplicateWatermark));

    let mut cyclic = valid.clone();
    cyclic.edges.push(AgentGraphEdge {
        parent_task_id: "task-child".to_owned(),
        child_task_id: "task-root".to_owned(),
        relationship: GraphEdgeRelationship::Delegate,
    });
    assert_eq!(cyclic.validate(), Err(SnapshotValidationError::GraphCycle));

    let mut unknown_runtime = valid.clone();
    unknown_runtime.tasks[1].runtime_id = RuntimeId::from_bytes([0x44; 16]);
    assert_eq!(unknown_runtime.validate(), Err(SnapshotValidationError::UnknownRuntime));

    let mut multiple_parents = valid.clone();
    multiple_parents.tasks.push(AgentTaskSnapshot {
        task_id: "task-other-parent".to_owned(),
        agent_id: "styrene:agent:other".to_owned(),
        runtime_id: RuntimeId::from_bytes([0x11; 16]),
        state: TaskState::Working,
        cancellation: CancellationState::default(),
        effective_grant_reference: None,
    });
    multiple_parents.edges.push(AgentGraphEdge {
        parent_task_id: "task-other-parent".to_owned(),
        child_task_id: "task-child".to_owned(),
        relationship: GraphEdgeRelationship::Delegate,
    });
    assert_eq!(multiple_parents.validate(), Err(SnapshotValidationError::MultipleParents));

    let mut duplicate_edge = valid.clone();
    duplicate_edge.edges.push(duplicate_edge.edges[0].clone());
    assert_eq!(duplicate_edge.validate(), Err(SnapshotValidationError::DuplicateGraphEdge));

    let mut unknown_watermark_runtime = valid;
    unknown_watermark_runtime.watermarks[0].source_runtime_id = RuntimeId::from_bytes([0x44; 16]);
    assert_eq!(unknown_watermark_runtime.validate(), Err(SnapshotValidationError::UnknownRuntime));
}

#[test]
fn snapshot_rejects_unspecified_task_state_and_oversized_collections() {
    let mut unspecified = snapshot();
    unspecified.tasks[0].state = TaskState::Unspecified;
    assert_eq!(unspecified.validate(), Err(SnapshotValidationError::InvalidTask));

    let mut oversized = snapshot();
    oversized.tasks = std::iter::repeat_with(|| oversized.tasks[0].clone())
        .take(MAX_AGENT_SNAPSHOT_TASKS + 1)
        .collect();
    assert_eq!(oversized.validate(), Err(SnapshotValidationError::CollectionLimitExceeded));
}

#[test]
fn snapshot_round_trips_without_collapsing_cancellation_state() {
    let mut value = snapshot();
    value.tasks[1].cancellation = CancellationState {
        requested_at_ms: Some(10),
        accepted_at_ms: Some(20),
        termination_confirmed_at_ms: None,
    };

    let json = serde_json::to_vec(&value).unwrap();
    let decoded: AgentSnapshot = serde_json::from_slice(&json).unwrap();
    assert_eq!(decoded, value);
    assert!(decoded.tasks[1].cancellation.requested_at_ms.is_some());
    assert!(decoded.tasks[1].cancellation.termination_confirmed_at_ms.is_none());
}
