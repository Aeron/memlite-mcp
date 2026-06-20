use serde::{Deserialize, Serialize};

use super::helpers::unix_secs;

/// Represents a single stored memory row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    /// Row identifier assigned by the database.
    pub id: i64,
    /// Full text of the memory; the field that is embedded for semantic search.
    pub content: String,
    /// One-sentence description for quick recall without reading the full content.
    pub summary: Option<String>,
    /// Category labels attached to this memory for grouping and filtering.
    pub tags: Vec<String>,
    /// Identifier of the agent that created this memory.
    pub agent: String,
    /// Unix timestamp (seconds) when the row was first inserted.
    pub created_at: i64,
    /// Unix timestamp (seconds) when the row was last modified.
    pub updated_at: i64,
    /// Background rationale: why this memory matters, how decisions were reached, etc.
    pub context: Option<String>,
    /// Base confidence score in [0, 1]; 1.0 means fully trusted.
    pub confidence: f64,
    /// Per-day confidence decay rate; 0.0 means no decay.
    pub decay_rate: f64,
    /// Unix timestamp (seconds) when it was last seen (or confidence validated).
    /// 0 means the created_at timestamp is used as the baseline.
    pub last_seen_at: i64,
    /// Optional classification of the memory's origin or intended use.
    pub memory_type: Option<MemoryType>,
    /// Optional lifecycle state; most meaningful for task and issue memories.
    pub status: Option<MemoryStatus>,
    /// Optional urgency or impact level; most meaningful for finding, issue, and task memories.
    pub severity: Option<MemorySeverity>,
    /// Outgoing directed links keyed by stringified target identifier with a free-form kind label.
    pub relations: std::collections::BTreeMap<String, String>,
    /// External reference pointers: file paths, symbol names, URLs, issue IDs, etc.
    pub refs: Vec<String>,
}

impl Memory {
    /// Computes the current effective confidence after applying time-based decay.
    ///
    /// Returns `confidence` unchanged when `decay_rate` is zero or non-positive.
    /// Falls back to `created_at` as the validation baseline when `last_seen_at` is 0.
    /// Result is clamped to [0.0, 1.0].
    pub fn effective_confidence(&self) -> f64 {
        if self.decay_rate <= 0.0 {
            return self.confidence.clamp(0.0, 1.0);
        }
        let seen_at = if self.last_seen_at > 0 {
            self.last_seen_at
        } else {
            self.created_at
        };
        let now = unix_secs();
        let days = ((now - seen_at).max(0) as f64) / 86400.0;
        (self.confidence - self.decay_rate * days).clamp(0.0, 1.0)
    }

    /// Returns the number of whole days since this memory was last modified.
    pub fn age_days(&self) -> u64 {
        let now = unix_secs();
        ((now - self.updated_at).max(0) as u64) / 86400
    }

    /// Returns a plain-English warning when this memory may be unreliable.
    ///
    /// Triggers when decay is active and effective confidence has fallen below
    /// the initial value, or when the memory has not been updated in more than
    /// 30 days.
    pub fn freshness_warning(&self) -> Option<String> {
        let eff = self.effective_confidence();

        if self.decay_rate > 0.0 && eff < self.confidence.clamp(0.0, 1.0) {
            let seen_at = if self.last_seen_at > 0 {
                self.last_seen_at
            } else {
                self.created_at
            };
            let val_days = ((unix_secs() - seen_at).max(0) as u64) / 86400;
            return Some(format!(
                "Confidence decayed to {:.0}% (from {:.0}%; {} days since last validation). \
                 Call memory_update to revalidate and reset the decay clock.",
                eff * 100.0,
                self.confidence.clamp(0.0, 1.0) * 100.0,
                val_days,
            ));
        }

        let days = self.age_days();
        if days > 30 {
            return Some(format!(
                "Memory is {} days old; verify it is still current before acting on it.",
                days,
            ));
        }

        None
    }
}

/// Classifies a memory's intended use, enabling targeted filtering.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    /// Facts about the person: preferences, identity, working style.
    User,
    /// Codebase knowledge: conventions, tech stack, architecture decisions, constraints.
    Project,
    /// Domain knowledge: docs, external facts, API details, terminology.
    Reference,
    /// Things to track, follow up on, or complete in a future session.
    Task,
    /// Discovered problem, audit result, or identified risk; not yet a tracked task.
    Finding,
    /// Known defect or bug.
    Issue,
    /// Architectural or design decision record: what was chosen, what rejected, and why.
    Decision,
    /// High-level plan or phased roadmap; links to tasks and decisions via relations.
    Plan,
}

impl MemoryType {
    /// Returns the lowercase string representation of the variant.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryType::User => "user",
            MemoryType::Project => "project",
            MemoryType::Reference => "reference",
            MemoryType::Task => "task",
            MemoryType::Finding => "finding",
            MemoryType::Issue => "issue",
            MemoryType::Decision => "decision",
            MemoryType::Plan => "plan",
        }
    }

    /// Parses a lowercase string into the matching variant, returning None on mismatch.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "user" => Some(MemoryType::User),
            "project" => Some(MemoryType::Project),
            "reference" => Some(MemoryType::Reference),
            "task" => Some(MemoryType::Task),
            "finding" => Some(MemoryType::Finding),
            "issue" => Some(MemoryType::Issue),
            "decision" => Some(MemoryType::Decision),
            "plan" => Some(MemoryType::Plan),
            _ => None,
        }
    }
}

impl TryFrom<&str> for MemoryType {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s).ok_or_else(|| {
            format!("unknown memory_type {s:?}; expected: user, project, reference, task, finding, issue, decision, or plan")
        })
    }
}

/// Tracks the lifecycle state of a memory; most useful for task and issue types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryStatus {
    /// Active and unresolved; the default when set without further qualification.
    Open,
    /// Completed or fixed; no longer requires action.
    Resolved,
    /// Acknowledged but will not be addressed.
    Wontfix,
    /// Postponed for later consideration.
    Deferred,
}

impl MemoryStatus {
    /// Returns the lowercase string representation of the variant.
    pub fn as_str(self) -> &'static str {
        match self {
            MemoryStatus::Open => "open",
            MemoryStatus::Resolved => "resolved",
            MemoryStatus::Wontfix => "wontfix",
            MemoryStatus::Deferred => "deferred",
        }
    }

    /// Parses a lowercase string into the matching variant, returning None on mismatch.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "open" => Some(MemoryStatus::Open),
            "resolved" => Some(MemoryStatus::Resolved),
            "wontfix" => Some(MemoryStatus::Wontfix),
            "deferred" => Some(MemoryStatus::Deferred),
            _ => None,
        }
    }
}

impl TryFrom<&str> for MemoryStatus {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s).ok_or_else(|| {
            format!("unknown status {s:?}; expected: open, resolved, wontfix, or deferred")
        })
    }
}

/// Indicates the urgency or impact of a finding, issue, or task.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemorySeverity {
    /// Requires immediate attention; may block work or cause data loss.
    Critical,
    /// Significant impact; should be addressed soon.
    High,
    /// Noticeable impact; can be scheduled.
    Medium,
    /// Minor impact; address when convenient.
    Low,
}

impl MemorySeverity {
    /// Returns the lowercase string representation of the variant.
    pub fn as_str(self) -> &'static str {
        match self {
            MemorySeverity::Critical => "critical",
            MemorySeverity::High => "high",
            MemorySeverity::Medium => "medium",
            MemorySeverity::Low => "low",
        }
    }

    /// Parses a lowercase string into the matching variant, returning None on mismatch.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "critical" => Some(MemorySeverity::Critical),
            "high" => Some(MemorySeverity::High),
            "medium" => Some(MemorySeverity::Medium),
            "low" => Some(MemorySeverity::Low),
            _ => None,
        }
    }
}

impl TryFrom<&str> for MemorySeverity {
    type Error = String;

    fn try_from(s: &str) -> Result<Self, Self::Error> {
        Self::from_str(s).ok_or_else(|| {
            format!("unknown severity {s:?}; expected: critical, high, medium, or low")
        })
    }
}

/// Holds owned data for a single row in a batch insert operation.
pub struct BatchInsertItem {
    /// Full text to embed and associate with this memory.
    pub content: String,
    /// Pre-computed embedding vector for `content`.
    pub embedding: Vec<f32>,
    /// Short label for list views.
    pub summary: Option<String>,
    /// Category labels.
    pub tags: Vec<String>,
    /// Agent identifier.
    pub agent: String,
    /// Background rationale.
    pub context: Option<String>,
    /// Base trust score in [0.0, 1.0]; caller is responsible for clamping.
    pub confidence: f64,
    /// Per-day confidence decay in [0.0, 1.0]; caller is responsible for clamping.
    pub decay_rate: f64,
    /// Optional memory type classification.
    pub memory_type: Option<MemoryType>,
    /// Optional lifecycle status.
    pub status: Option<MemoryStatus>,
    /// Optional urgency or impact level.
    pub severity: Option<MemorySeverity>,
    /// Outgoing directed links as (target id, kind) pairs.
    pub relations: Vec<(i64, String)>,
    /// External reference pointers.
    pub refs: Vec<String>,
}

/// One neighbor edge with the referenced memory object (if it still exists).
pub struct NeighborInfo {
    /// Memory id on the other end of the edge.
    pub other_id: i64,
    /// Relation kind (e.g. "caused_by", "blocks").
    pub kind: String,
    /// Full memory for `other_id`; None when the target was deleted.
    pub memory: Option<Memory>,
}

/// Immediate in-neighbors and out-neighbors of a single memory node.
pub struct Neighbors {
    /// Nodes this memory links to (outgoing edges).
    pub outgoing: Vec<NeighborInfo>,
    /// Nodes that link to this memory (incoming edges).
    pub incoming: Vec<NeighborInfo>,
}

/// Specifies which edge directions to follow during graph traversal.
pub enum TraverseDirection {
    /// Follow outgoing edges only (default).
    Out,
    /// Follow incoming edges only.
    In,
    /// Follow both directions.
    Both,
}

impl From<Option<&str>> for TraverseDirection {
    fn from(s: Option<&str>) -> Self {
        match s {
            Some("in") => TraverseDirection::In,
            Some("both") => TraverseDirection::Both,
            _ => TraverseDirection::Out,
        }
    }
}

/// One node discovered during a BFS graph traversal.
pub struct TraverseNode {
    /// Full memory at this node.
    pub memory: Memory,
    /// BFS depth at which this node was first discovered (0 = start node).
    pub depth: u32,
    /// Relation kind that led to this node; None for the start node.
    pub via_kind: Option<String>,
}

/// Result of a BFS graph traversal from a starting node.
pub struct TraverseResult {
    /// Id of the node the traversal started from.
    pub start_id: i64,
    /// All discovered nodes, ordered by depth then id.
    pub nodes: Vec<TraverseNode>,
    /// All traversed edges as (from_id, to_id, kind), deduplicated.
    pub edges: Vec<(i64, i64, String)>,
}

/// One step along a path between two memory nodes.
pub struct PathStep {
    /// Full memory at this step.
    pub memory: Memory,
    /// Kind of the edge that led to this node from the previous step; None for the start.
    pub via_kind: Option<String>,
}

/// One entry in the hub (most-connected nodes) ranking.
pub struct HubEntry {
    /// Memory id.
    pub id: i64,
    /// Short description for identification.
    pub summary: Option<String>,
    /// Memory type classification.
    pub memory_type: Option<MemoryType>,
    /// Number of outgoing relation edges.
    pub out_degree: u32,
    /// Number of incoming relation edges.
    pub in_degree: u32,
}

/// Graph-level aggregate statistics.
pub struct GraphStats {
    /// Total memory count (filtered by agent when applicable).
    pub total_memories: u64,
    /// Count per memory type; None key = untyped memories.
    pub by_type: Vec<(Option<String>, u64)>,
    /// Total relation edges across the entire graph.
    pub total_relations: u64,
    /// All distinct relation kind labels present in the graph.
    pub relation_kinds: Vec<String>,
}
