use rmcp::schemars;
use serde::Deserialize;

use crate::db::MemoryAttrs;

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchParams {
    /// Natural-language query; results are ranked by cosine similarity.
    #[schemars(length(min = 1))]
    pub query: String,
    /// Maximum number of results to return (default: 10, max: 100).
    #[schemars(range(max = 100))]
    pub limit: Option<u32>,
    /// Full context and content in each result (default: false).
    /// Compact selection output when false includes the required summary.
    pub include_content: Option<bool>,
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
    /// Exact numeric identifiers to fetch; maximum 100. Missing identifiers are omitted.
    /// Results remain ordered newest first, not in input order.
    #[schemars(length(max = 100))]
    pub ids: Option<Vec<i64>>,
    /// Full context and content in each result (default: true).
    /// Full output omits redundant summary.
    pub include_content: Option<bool>,
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
pub struct StoreItem {
    /// Full text to embed and store.
    #[schemars(length(min = 1))]
    pub content: String,
    /// One-sentence description for quick recall without reading the full content.
    #[schemars(length(min = 1))]
    pub summary: String,
    /// Grouping labels; reuse existing tags, lowercase, singular where natural
    /// (e.g. ["architecture", "auth", "bug"]).
    pub tags: Option<Vec<String>>,
    /// Background rationale for this memory.
    pub context: Option<String>,
    /// Structured category; required. One of: "user" (preferences, working
    /// style, opinions on agent approach), "project" (conventions,
    /// architecture, constraints), "reference" (external or domain facts),
    /// "task" (follow-ups to complete), "finding" (observation, evaluation,
    /// or feedback on work quality), "issue" (defect or bug), "decision"
    /// (what was chosen, what rejected, why), "plan" (phased roadmap).
    /// Feedback is not a type: opinions on approach go to "user";
    /// work-quality observations go to "finding" or "issue". Unclear:
    /// use "project" for general codebase facts.
    pub memory_type: String,
    /// Initial confidence score in [0, 1] (default 1.0): 0.5-0.7 for
    /// speculative or inferred information, 0.8-0.9 for believed but
    /// unverified information, 1.0 for confirmed information.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: Option<f64>,
    /// Per-day confidence decay rate in [0, 1] (default 0.0): 0.0 for stable
    /// facts and decisions, 0.005-0.01 for slowly drifting details,
    /// 0.02-0.05 for environment state or workarounds, 0.1+ for
    /// time-sensitive information.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub decay_rate: Option<f64>,
    /// Typed metadata: status for tasks, findings, or issues; severity for
    /// findings or issues; priority for tasks; outcome and reversible for
    /// decisions; phase for plans.
    pub attrs: Option<MemoryAttrs>,
    /// Outgoing one-way links to existing memories as {"<target_id>": "<kind>"};
    /// common kinds: caused_by, causes, blocks, depends_on, supersedes,
    /// related_to, duplicate_of.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
    /// External pointers such as file paths, symbols, URLs, or issue
    /// identifiers. Use project-relative file paths in single-project workspaces;
    /// prefix the project root directory name in multi-project workspaces.
    pub refs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreParams {
    /// Items to store; maximum 100 per call.
    #[schemars(length(min = 1, max = 100))]
    pub items: Vec<StoreItem>,
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
    #[schemars(length(min = 1, max = 100))]
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
    #[schemars(length(min = 1))]
    pub content: Option<String>,
    /// Replacement summary; blank values are rejected.
    pub summary: Option<String>,
    /// Replacement grouping labels; reuse existing tags, lowercase, singular
    /// where natural.
    pub tags: Option<Vec<String>>,
    /// Replacement context; pass empty string to clear the field.
    pub context: Option<String>,
    /// Replacement confidence score in [0, 1]: 0.5-0.7 for speculative or
    /// inferred information, 0.8-0.9 for believed but unverified information,
    /// 1.0 for confirmed information.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub confidence: Option<f64>,
    /// Replacement per-day confidence decay rate in [0, 1]: 0.0 for stable
    /// facts and decisions, 0.005-0.01 for slowly drifting details,
    /// 0.02-0.05 for environment state or workarounds, 0.1+ for
    /// time-sensitive information.
    #[schemars(range(min = 0.0, max = 1.0))]
    pub decay_rate: Option<f64>,
    /// Replacement structured category. One of: "user" (preferences, working
    /// style, opinions on agent approach), "project" (conventions,
    /// architecture, constraints), "reference" (external or domain facts),
    /// "task" (follow-ups to complete), "finding" (observation, evaluation,
    /// or feedback on work quality), "issue" (defect or bug), "decision"
    /// (what was chosen, what rejected, why), "plan" (phased roadmap).
    /// Feedback is not a type: opinions on approach go to "user";
    /// work-quality observations go to "finding" or "issue". Empty string is
    /// rejected.
    pub memory_type: Option<String>,
    /// Replacement typed metadata; replaces the full set when provided.
    /// Status applies to tasks, findings, or issues; severity to findings or
    /// issues; priority to tasks; outcome and reversible to decisions; phase
    /// to plans. An empty object clears all metadata.
    pub attrs: Option<MemoryAttrs>,
    /// Replacement outgoing one-way links to existing memories as
    /// {"<target_id>": "<kind>"}; common kinds: caused_by, causes, blocks,
    /// depends_on, supersedes, related_to, duplicate_of. An empty object clears all links.
    pub relations: Option<std::collections::BTreeMap<String, String>>,
    /// Replacement external pointers such as file paths, symbols, URLs, or
    /// issue identifiers. Use project-relative file paths in single-project
    /// workspaces; prefix the project root directory name in multi-project
    /// workspaces. An empty array clears all refs.
    pub refs: Option<Vec<String>>,
}
