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
    /// Structured category; required. One of: "user", "project", "reference", "task".
    pub memory_type: String,
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
    /// Structured category: one of "user", "project", "reference", or "task".
    pub memory_type: Option<String>,
}
