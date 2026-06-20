use crate::db::types::{MemorySeverity, MemoryStatus, MemoryType};

/// Holds the optional metadata fields for inserting a new memory row.
#[derive(Default)]
pub struct InsertOptions<'a> {
    /// Short label for list views.
    pub summary: Option<&'a str>,
    /// Category labels stored as a JSON array.
    pub tags: &'a [String],
    /// Agent identifier; empty string means no specific agent.
    pub agent: &'a str,
    /// Background rationale for the memory.
    pub context: Option<&'a str>,
    /// Base trust score in [0.0, 1.0]; defaults to 1.0.
    pub confidence: Option<f64>,
    /// Per-day confidence decay in [0.0, 1.0]; defaults to 0.0.
    pub decay_rate: Option<f64>,
    /// Optional classification of the memory's origin or intended use.
    pub memory_type: Option<MemoryType>,
    /// Optional lifecycle status; defaults to NULL (no status).
    pub status: Option<MemoryStatus>,
    /// Optional urgency or impact level; defaults to NULL.
    pub severity: Option<MemorySeverity>,
    /// Outgoing directed links as (target id, kind) pairs; defaults to empty.
    pub relations: &'a [(i64, String)],
    /// External reference pointers; defaults to empty.
    pub refs: &'a [String],
}

/// Holds the fields that can be updated on an existing memory row.
#[derive(Default)]
pub struct UpdateOptions<'a> {
    /// Replacement content; triggers re-embedding when provided.
    pub content: Option<&'a str>,
    /// Replacement summary; pass Some("") to clear.
    pub summary: Option<&'a str>,
    /// Replacement tags.
    pub tags: Option<&'a [String]>,
    /// Replacement embedding vector.
    pub embedding: Option<&'a [f32]>,
    /// Replacement context; pass Some("") to clear.
    pub context: Option<&'a str>,
    /// Replacement confidence score.
    pub confidence: Option<f64>,
    /// Replacement per-day decay rate.
    pub decay_rate: Option<f64>,
    /// Replacement memory type; set to NULL when `clear_memory_type` is true.
    pub memory_type: Option<MemoryType>,
    /// When true, sets the type column to NULL regardless of `memory_type`.
    pub clear_memory_type: bool,
    /// Replacement lifecycle status; set to NULL when `clear_status` is true.
    pub status: Option<MemoryStatus>,
    /// When true, sets the status column to NULL regardless of `status`.
    pub clear_status: bool,
    /// Replacement severity level; set to NULL when `clear_severity` is true.
    pub severity: Option<MemorySeverity>,
    /// When true, sets the severity column to NULL regardless of `severity`.
    pub clear_severity: bool,
    /// Replacement outgoing relations as (target id, kind) pairs; replaces the full set when provided.
    pub relations: Option<&'a [(i64, String)]>,
    /// Replacement external reference pointers; replaces the full set when provided.
    pub refs: Option<&'a [String]>,
}

/// Shared pre-filter options for search and list queries.
#[derive(Default)]
pub struct FilterOptions<'a> {
    /// Restricts results to those owned by this agent identifier.
    pub agent_id: Option<&'a str>,
    /// Restricts results to those carrying all of the given tags (hard AND).
    pub tags: Option<&'a [String]>,
    /// Restricts results to the given memory type.
    pub memory_type: Option<MemoryType>,
    /// Restricts results to the given lifecycle status.
    pub status: Option<MemoryStatus>,
    /// Restricts results to the given severity level.
    pub severity: Option<MemorySeverity>,
    /// Restricts results to memories whose refs array contains this substring.
    pub refs_contain: Option<&'a str>,
}
