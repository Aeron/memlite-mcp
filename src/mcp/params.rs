use rmcp::schemars;
use serde::Deserialize;

use crate::db::MemoryAttrs;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreParams {
    /// The full text to embed and store.
    #[schemars(length(min = 1))]
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
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: Option<f64>,
    /// Per-day confidence decay rate in [0, 1] (default 0.0); 0.01 reduces
    /// confidence by 1% per day since the last validation.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub decay_rate: Option<f64>,
    /// Structured category; required. One of: "user", "project", "reference", "task",
    /// "finding", "issue", "decision", "plan".
    pub memory_type: String,
    /// Type-specific extra fields. See MemoryAttrs for available typed fields.
    pub attrs: Option<MemoryAttrs>,
    /// Outgoing directed links to other memory identifiers. A JSON object
    /// mapping target memory id (as a string) to a free-form relation kind,
    /// e.g. {"7": "caused_by", "12": "blocks"}. Kind is any non-empty string;
    /// use consistent snake_case labels across the project (e.g. caused_by,
    /// blocks, depends_on, supersedes, related_to). Omit or pass null for none.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
    /// External reference pointers for this memory. Free-form strings: project-relative
    /// file paths (e.g. "src/db/store.rs"), symbol names (e.g. "MemoryStore::insert"),
    /// URLs, or issue IDs. Prefer short paths over absolute paths.
    pub refs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Natural-language query; results are ranked by cosine similarity.
    #[schemars(length(min = 1))]
    pub query: String,
    /// Maximum number of results to return (default: 10, max: 100).
    #[schemars(range(max = 100))]
    pub limit: Option<u32>,
    /// Restricts results to memories that carry all of the given tags.
    pub tags: Option<Vec<String>>,
    /// Restricts results to memories of the given type.
    pub memory_type: Option<String>,
    /// Restricts results to memories with the given status.
    pub status: Option<String>,
    /// Restricts results to memories with the given severity.
    pub severity: Option<String>,
    /// Restricts results to memories with the given priority.
    pub priority: Option<String>,
    /// Restricts results to memories with the given decision outcome.
    pub outcome: Option<String>,
    /// Restricts results to memories with the given plan phase.
    pub phase: Option<String>,
    /// Restricts results to memories whose refs array contains this substring.
    /// Useful for finding all memories that mention a specific file or symbol.
    pub refs_contain: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListParams {
    /// Maximum number of results per page (default: 20, max: 100).
    #[schemars(range(max = 100))]
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
    /// Restricts results to memories with the given priority.
    pub priority: Option<String>,
    /// Restricts results to memories with the given decision outcome.
    pub outcome: Option<String>,
    /// Restricts results to memories with the given plan phase.
    pub phase: Option<String>,
    /// Restricts results to memories whose refs array contains this substring.
    pub refs_contain: Option<String>,
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
    /// "finding", "issue", "decision", "plan".
    pub memory_type: Option<String>,
    /// Initial confidence score in [0, 1] (default 1.0).
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: Option<f64>,
    /// Per-day confidence decay rate in [0, 1] (default 0.0).
    #[schemars(range(min = 0.0, max = 1.0))]
    pub decay_rate: Option<f64>,
    /// Type-specific extra fields; same structure as memory_store attrs.
    pub attrs: Option<MemoryAttrs>,
    /// Outgoing directed links; same format as memory_store relations.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
    /// External reference pointers; same format as memory_store refs.
    pub refs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct BatchStoreParams {
    /// Items to store; maximum 100 per call.
    pub items: Vec<BatchStoreItem>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct EmptyParams {}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DeleteParams {
    /// Numeric identifier of the memory to delete (from memory_store or memory_list).
    pub id: i64,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RevalidateParams {
    /// Numeric identifiers of the memories to revalidate; resets the decay clock on each.
    pub ids: Vec<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct NeighborsParams {
    /// Numeric id of the memory to get neighbors for.
    pub id: i64,
    /// Optional relation kind filter (e.g. "blocks", "caused_by").
    /// When omitted, all edge kinds are returned.
    pub kind: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TraverseParams {
    /// Numeric id of the start memory.
    pub start_id: i64,
    /// BFS depth limit; default 2, max 5.
    #[schemars(range(min = 1, max = 5))]
    pub depth: Option<u32>,
    /// Optional relation kind filter; only edges with this exact kind are traversed.
    pub kind: Option<String>,
    /// Edge direction to follow: "out" (default), "in", or "both".
    pub direction: Option<String>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PathParams {
    /// Numeric id of the start memory.
    pub from_id: i64,
    /// Numeric id of the destination memory.
    pub to_id: i64,
    /// Maximum hops to explore before giving up; default 6, max 10.
    #[schemars(range(min = 1, max = 10))]
    pub max_hops: Option<u32>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HubsParams {
    /// Number of top hubs to return; default 10, max 50.
    #[schemars(range(min = 1, max = 50))]
    pub top_n: Option<u32>,
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
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: Option<f64>,
    /// Replacement per-day confidence decay rate; clamped to [0, 1].
    #[schemars(range(min = 0.0, max = 1.0))]
    pub decay_rate: Option<f64>,
    /// Structured category: one of "user", "project", "reference", "task",
    /// "finding", "issue", "decision", "plan". Pass as empty string to clear.
    pub memory_type: Option<String>,
    /// Replacement attrs; replaces the full set when provided.
    /// Omit fields to clear them; pass an empty object to clear all.
    pub attrs: Option<MemoryAttrs>,
    /// Replacement outgoing relations; replaces the full set when provided.
    /// Same format as memory_store: {"<id>": "<kind>"}. Pass {} to clear all links.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
    /// Replacement external reference pointers; replaces the full set when provided.
    /// Pass [] to clear all refs.
    pub refs: Option<Vec<String>>,
}
