use rmcp::schemars;
use serde::Deserialize;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreParams {
    /// The full text to embed and store.
    pub content: String,
    /// One-sentence description for quick recall without reading the full
    /// content. Provide when content is long, complex, or not self-describing.
    pub summary: Option<String>,
    /// Category labels for grouping (e.g. ["architecture", "auth", "bug"]).
    /// Provide when the content clearly belongs to a named topic or domain.
    pub tags: Option<Vec<String>>,
    /// Background rationale for this memory: why the decision was made, why it
    /// matters, how it fits the bigger picture, or anything that gives meaning
    /// beyond what the content itself conveys.
    pub context: Option<String>,
    /// Initial confidence score in [0, 1] (default 1.0); lower values indicate
    /// uncertain or potentially outdated information.
    pub confidence: Option<f64>,
    /// Per-day confidence decay rate in [0, 1] (default 0.0); 0.01 reduces
    /// confidence by 1% per day since the last validation.
    pub decay_rate: Option<f64>,
    /// Structured category; required. One of: "user", "project", "reference", "task",
    /// "finding", "issue", "decision".
    pub memory_type: String,
    /// Lifecycle status; one of: "open", "resolved", "wontfix", "deferred".
    /// Set for task and issue memories. Omit for general knowledge.
    pub status: Option<String>,
    /// Urgency or impact level; one of: "critical", "high", "medium", "low".
    /// Most meaningful for finding, issue, and task memories.
    pub severity: Option<String>,
    /// Outgoing directed links to other memory identifiers. A JSON object
    /// mapping target memory id (as a string) to a free-form relation kind,
    /// e.g. {"7": "caused_by", "12": "blocks"}. Kind is any non-empty string;
    /// use consistent snake_case labels across the project (e.g. caused_by,
    /// blocks, depends_on, supersedes, related_to). Omit or pass null for none.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Natural-language query; results are ranked by cosine similarity.
    pub query: String,
    /// Maximum number of results to return (default: 10, max: 100).
    pub limit: Option<u32>,
    /// Restricts results to memories that carry all of the given tags.
    pub tags: Option<Vec<String>>,
    /// Restricts results to memories of the given type.
    pub memory_type: Option<String>,
    /// Restricts results to memories with the given status.
    pub status: Option<String>,
    /// Restricts results to memories with the given severity.
    pub severity: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Maximum number of results per page (default: 20, max: 100).
    pub limit: Option<u32>,
    /// Zero-based offset for pagination (default: 0).
    pub offset: Option<u32>,
    /// Restricts results to memories that carry all of the given tags.
    pub tags: Option<Vec<String>>,
    /// Restricts results to memories of the given type.
    pub memory_type: Option<String>,
    /// Restricts results to memories with the given status.
    pub status: Option<String>,
    /// Restricts results to memories with the given severity.
    pub severity: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BatchStoreItem {
    /// Full text to embed and store.
    pub content: String,
    /// One-sentence description for quick recall without reading the full
    /// content. Provide when content is long, complex, or not self-describing.
    pub summary: Option<String>,
    /// Category labels for grouping (e.g. ["architecture", "auth", "bug"]).
    pub tags: Option<Vec<String>>,
    /// Background rationale for this memory.
    pub context: Option<String>,
    /// Structured category: one of "user", "project", "reference", "task",
    /// "finding", "issue", "decision".
    pub memory_type: Option<String>,
    /// Initial confidence score in [0, 1] (default 1.0).
    pub confidence: Option<f64>,
    /// Per-day confidence decay rate in [0, 1] (default 0.0).
    pub decay_rate: Option<f64>,
    /// Lifecycle status; one of: "open", "resolved", "wontfix", "deferred".
    pub status: Option<String>,
    /// Urgency or impact level; one of: "critical", "high", "medium", "low".
    pub severity: Option<String>,
    /// Outgoing directed links; same format as memory_store relations.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BatchStoreParams {
    /// Items to store; maximum 100 per call.
    pub items: Vec<BatchStoreItem>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TagsParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteParams {
    /// Numeric identifier of the memory to delete (from memory_store or memory_list).
    pub id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct UpdateParams {
    /// Numeric identifier of the memory to update.
    pub id: i64,
    /// Replacement content; triggers re-embedding when provided.
    pub content: Option<String>,
    /// Replacement summary; pass empty string to clear the field.
    pub summary: Option<String>,
    /// Replacement tags.
    pub tags: Option<Vec<String>>,
    /// Replacement context; pass empty string to clear the field.
    pub context: Option<String>,
    /// Replacement confidence score; clamped to [0, 1].
    pub confidence: Option<f64>,
    /// Replacement per-day confidence decay rate; clamped to [0, 1].
    pub decay_rate: Option<f64>,
    /// Structured category: one of "user", "project", "reference", "task",
    /// "finding", "issue", "decision".
    pub memory_type: Option<String>,
    /// Replacement lifecycle status; one of: "open", "resolved", "wontfix", "deferred".
    /// Pass to transition a task or issue through its lifecycle.
    pub status: Option<String>,
    /// Replacement severity level; one of: "critical", "high", "medium", "low".
    pub severity: Option<String>,
    /// Replacement outgoing relations; replaces the full set when provided.
    /// Same format as memory_store: {"<id>": "<kind>"}. Pass {} to clear all links.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
}
