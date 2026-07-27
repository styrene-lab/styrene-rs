use std::collections::HashSet;

use serde::{Deserialize, Serialize};

use crate::{AgentId, RootOperationId, RuntimeId, TaskState};

pub const MAX_AGENT_SNAPSHOT_RUNTIMES: usize = 4_096;
pub const MAX_AGENT_SNAPSHOT_TASKS: usize = 65_536;
pub const MAX_AGENT_SNAPSHOT_EDGES: usize = 65_535;
pub const MAX_AGENT_SNAPSHOT_WATERMARKS: usize = 65_536;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SequenceWatermark {
    pub source_runtime_id: RuntimeId,
    pub stream_id: String,
    /// Highest sequence for which every prior sequence in this incarnation is present.
    pub contiguous_through: u64,
    /// Highest sequence seen, including events retained beyond a gap.
    pub highest_observed: u64,
}

impl SequenceWatermark {
    pub fn has_gap(&self) -> bool {
        self.highest_observed > self.contiguous_through
    }

    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        if self.stream_id.trim().is_empty() || self.contiguous_through > self.highest_observed {
            return Err(SnapshotValidationError::InvalidWatermark);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshotRequest {
    pub root_operation_id: RootOperationId,
    #[serde(default)]
    pub known_watermarks: Vec<SequenceWatermark>,
}

impl AgentSnapshotRequest {
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        validate_watermarks(&self.known_watermarks)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancellationState {
    pub requested_at_ms: Option<u64>,
    pub accepted_at_ms: Option<u64>,
    pub termination_confirmed_at_ms: Option<u64>,
}

impl CancellationState {
    fn validate(&self) -> Result<(), SnapshotValidationError> {
        if self.accepted_at_ms.is_some() && self.requested_at_ms.is_none() {
            return Err(SnapshotValidationError::InvalidCancellationState);
        }
        if self.termination_confirmed_at_ms.is_some() && self.accepted_at_ms.is_none() {
            return Err(SnapshotValidationError::InvalidCancellationState);
        }
        if matches!((self.requested_at_ms, self.accepted_at_ms), (Some(requested), Some(accepted)) if accepted < requested)
            || matches!((self.accepted_at_ms, self.termination_confirmed_at_ms), (Some(accepted), Some(confirmed)) if confirmed < accepted)
        {
            return Err(SnapshotValidationError::InvalidCancellationState);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTaskSnapshot {
    pub task_id: String,
    pub agent_id: String,
    pub runtime_id: RuntimeId,
    /// A2A task state using the official protocol type and spelling.
    pub state: TaskState,
    #[serde(default)]
    pub cancellation: CancellationState,
    pub effective_grant_reference: Option<String>,
}

impl AgentTaskSnapshot {
    fn validate(&self) -> Result<(), SnapshotValidationError> {
        if self.task_id.trim().is_empty() || self.state == TaskState::Unspecified {
            return Err(SnapshotValidationError::InvalidTask);
        }
        AgentId::new(&self.agent_id).map_err(|_| SnapshotValidationError::InvalidTask)?;
        self.cancellation.validate()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GraphEdgeRelationship {
    Delegate,
    CleaveChild,
    Handoff,
    OperatorAttach,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentGraphEdge {
    pub parent_task_id: String,
    pub child_task_id: String,
    pub relationship: GraphEdgeRelationship,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSnapshot {
    pub root_operation_id: RootOperationId,
    pub generated_at_ms: u64,
    #[serde(default)]
    pub runtimes: Vec<RuntimeId>,
    #[serde(default)]
    pub tasks: Vec<AgentTaskSnapshot>,
    #[serde(default)]
    pub edges: Vec<AgentGraphEdge>,
    #[serde(default)]
    pub watermarks: Vec<SequenceWatermark>,
}

impl AgentSnapshot {
    pub fn validate(&self) -> Result<(), SnapshotValidationError> {
        if self.runtimes.len() > MAX_AGENT_SNAPSHOT_RUNTIMES
            || self.tasks.len() > MAX_AGENT_SNAPSHOT_TASKS
            || self.edges.len() > MAX_AGENT_SNAPSHOT_EDGES
            || self.watermarks.len() > MAX_AGENT_SNAPSHOT_WATERMARKS
        {
            return Err(SnapshotValidationError::CollectionLimitExceeded);
        }
        let runtime_ids: HashSet<RuntimeId> = self.runtimes.iter().copied().collect();
        if runtime_ids.len() != self.runtimes.len() {
            return Err(SnapshotValidationError::DuplicateRuntime);
        }
        let mut task_ids = HashSet::with_capacity(self.tasks.len());
        for task in &self.tasks {
            task.validate()?;
            if !runtime_ids.contains(&task.runtime_id) {
                return Err(SnapshotValidationError::UnknownRuntime);
            }
            if !task_ids.insert(task.task_id.as_str()) {
                return Err(SnapshotValidationError::DuplicateTask);
            }
        }
        let mut parent_by_child = std::collections::HashMap::new();
        let mut edge_keys = HashSet::with_capacity(self.edges.len());
        for edge in &self.edges {
            if edge.parent_task_id == edge.child_task_id
                || !task_ids.contains(edge.parent_task_id.as_str())
                || !task_ids.contains(edge.child_task_id.as_str())
            {
                return Err(SnapshotValidationError::DanglingGraphEdge);
            }
            if !edge_keys.insert((edge.parent_task_id.as_str(), edge.child_task_id.as_str())) {
                return Err(SnapshotValidationError::DuplicateGraphEdge);
            }
            if parent_by_child
                .insert(edge.child_task_id.as_str(), edge.parent_task_id.as_str())
                .is_some()
            {
                return Err(SnapshotValidationError::MultipleParents);
            }
        }
        if graph_has_cycle(&self.edges) {
            return Err(SnapshotValidationError::GraphCycle);
        }
        validate_watermarks(&self.watermarks)?;
        if self
            .watermarks
            .iter()
            .any(|watermark| !runtime_ids.contains(&watermark.source_runtime_id))
        {
            return Err(SnapshotValidationError::UnknownRuntime);
        }
        Ok(())
    }
}

fn graph_has_cycle(edges: &[AgentGraphEdge]) -> bool {
    let mut children: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    for edge in edges {
        children.entry(edge.parent_task_id.as_str()).or_default().push(edge.child_task_id.as_str());
    }

    fn visit<'a>(
        task: &'a str,
        children: &std::collections::HashMap<&'a str, Vec<&'a str>>,
        visiting: &mut HashSet<&'a str>,
        visited: &mut HashSet<&'a str>,
    ) -> bool {
        if visiting.contains(task) {
            return true;
        }
        if !visited.insert(task) {
            return false;
        }
        visiting.insert(task);
        if children
            .get(task)
            .is_some_and(|next| next.iter().any(|child| visit(child, children, visiting, visited)))
        {
            return true;
        }
        visiting.remove(task);
        false
    }

    let mut visiting = HashSet::new();
    let mut visited = HashSet::new();
    children.keys().any(|task| visit(task, &children, &mut visiting, &mut visited))
}

fn validate_watermarks(watermarks: &[SequenceWatermark]) -> Result<(), SnapshotValidationError> {
    let mut keys = HashSet::with_capacity(watermarks.len());
    for watermark in watermarks {
        watermark.validate()?;
        if !keys.insert((watermark.source_runtime_id, watermark.stream_id.as_str())) {
            return Err(SnapshotValidationError::DuplicateWatermark);
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SnapshotValidationError {
    #[error("snapshot collection exceeds its profile limit")]
    CollectionLimitExceeded,
    #[error("sequence watermark is invalid")]
    InvalidWatermark,
    #[error("sequence watermark key is duplicated")]
    DuplicateWatermark,
    #[error("task snapshot is invalid")]
    InvalidTask,
    #[error("task snapshot references a runtime absent from the snapshot")]
    UnknownRuntime,
    #[error("runtime incarnation is duplicated")]
    DuplicateRuntime,
    #[error("task id is duplicated")]
    DuplicateTask,
    #[error("graph edge references a missing task or itself")]
    DanglingGraphEdge,
    #[error("delegation graph edge is duplicated")]
    DuplicateGraphEdge,
    #[error("task has more than one immediate parent")]
    MultipleParents,
    #[error("delegation graph contains a cycle")]
    GraphCycle,
    #[error("cancellation milestones are missing or out of order")]
    InvalidCancellationState,
}
