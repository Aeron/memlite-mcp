use rmcp::schemars;
use serde::Deserialize;

use crate::db::{
    MemoryAttrOutcome, MemoryAttrPhase, MemoryAttrPriority, MemoryAttrSeverity, MemoryAttrs,
    MemoryStatus, MemoryType, TraverseDirection,
};

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
    pub memory_type: Option<MemoryType>,
    /// Restricts results to memories with the given status.
    pub status: Option<MemoryStatus>,
    /// Restricts results to memories with the given severity.
    pub severity: Option<MemoryAttrSeverity>,
    /// Restricts results to memories with the given priority.
    pub priority: Option<MemoryAttrPriority>,
    /// Restricts results to memories with the given decision outcome.
    pub outcome: Option<MemoryAttrOutcome>,
    /// Restricts results to memories with the given plan phase.
    pub phase: Option<MemoryAttrPhase>,
    /// Restricts results to memories whose refs array contains this substring.
    /// Uses literal matching without path normalization; query file refs using the
    /// same single-root or root-prefixed form used when storing them.
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
    pub memory_type: Option<MemoryType>,
    /// Restricts results to memories with the given status.
    pub status: Option<MemoryStatus>,
    /// Restricts results to memories with the given severity.
    pub severity: Option<MemoryAttrSeverity>,
    /// Restricts results to memories with the given priority.
    pub priority: Option<MemoryAttrPriority>,
    /// Restricts results to memories with the given decision outcome.
    pub outcome: Option<MemoryAttrOutcome>,
    /// Restricts results to memories with the given plan phase.
    pub phase: Option<MemoryAttrPhase>,
    /// Restricts results to memories whose refs array contains this substring.
    /// Uses literal matching without path normalization; query file refs using the
    /// same single-root or root-prefixed form used when storing them.
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
    /// Structured category; required. Classify content, not activity or source.
    /// "user" (personal preferences), "project" (general codebase facts/shared rules),
    /// "reference" (reusable knowledge/research synthesis), "task" (action),
    /// "finding" (assessment/risk), "issue" (confirmed defect), "decision"
    /// (recorded choice/rationale), "plan" (multi-step strategy/roadmap).
    /// Research: external knowledge is "reference"; codebase facts "project";
    /// evaluations "finding"; choices "decision"; unfinished research "task".
    /// Feedback: personal preferences are "user"; shared rules "project";
    /// work evaluations "finding"; confirmed bugs "issue".
    /// Keep problem records as "finding"/"issue"; remediation actions are "task".
    /// Recommendations alone are "finding"; recorded choices are "decision".
    /// Project-local assessments, defects, choices, actions, and roadmaps use
    /// their specific types, not "project".
    /// Use confidence for uncertain facts; completed records keep their type.
    pub memory_type: MemoryType,
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
    /// prefix the project root directory name in multi-project workspaces. Paths
    /// remain exactly as provided.
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
    pub direction: Option<TraverseDirection>,
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
    /// Replacement structured category. Classify content, not activity or source.
    /// "user" (personal preferences), "project" (general codebase facts/shared rules),
    /// "reference" (reusable knowledge/research synthesis), "task" (action),
    /// "finding" (assessment/risk), "issue" (confirmed defect), "decision"
    /// (recorded choice/rationale), "plan" (multi-step strategy/roadmap).
    /// Research: external knowledge is "reference"; codebase facts "project";
    /// evaluations "finding"; choices "decision"; unfinished research "task".
    /// Feedback: personal preferences are "user"; shared rules "project";
    /// work evaluations "finding"; confirmed bugs "issue".
    /// Keep problem records as "finding"/"issue"; remediation actions are "task".
    /// Recommendations alone are "finding"; recorded choices are "decision".
    /// Project-local assessments, defects, choices, actions, and roadmaps use
    /// their specific types, not "project".
    /// Use confidence for uncertain facts; completed records keep their type.
    pub memory_type: Option<MemoryType>,
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
    /// workspaces. Paths remain exactly as provided. An empty array clears all refs.
    pub refs: Option<Vec<String>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filters_deserialize_closed_vocabularies() {
        let fields = [
            ("memory_type", "project"),
            ("status", "resolved"),
            ("severity", "high"),
            ("priority", "urgent"),
            ("outcome", "accepted"),
            ("phase", "active"),
        ];
        for (field, value) in fields {
            let mut input = json!({"query": "memory"});
            input[field] = json!(value);
            assert!(serde_json::from_value::<SearchParams>(input.clone()).is_ok());
            assert!(serde_json::from_value::<ListParams>(input.clone()).is_ok());
            input[field] = json!("unknown");
            assert!(serde_json::from_value::<SearchParams>(input.clone()).is_err());
            assert!(serde_json::from_value::<ListParams>(input).is_err());
        }
    }

    #[test]
    fn store_and_update_deserialize_memory_types() {
        for memory_type in [
            "user",
            "project",
            "reference",
            "task",
            "finding",
            "issue",
            "decision",
            "plan",
        ] {
            assert!(serde_json::from_value::<StoreParams>(json!({
                "items": [{"content": "content", "summary": "summary", "memory_type": memory_type}]
            })).is_ok());
            assert!(
                serde_json::from_value::<UpdateParams>(
                    json!({"id": 1, "memory_type": memory_type})
                )
                .is_ok()
            );
        }
        for memory_type in ["", "unknown", "Project", "research", "feedback"] {
            assert!(serde_json::from_value::<StoreParams>(json!({
                "items": [{"content": "content", "summary": "summary", "memory_type": memory_type}]
            })).is_err());
            assert!(
                serde_json::from_value::<UpdateParams>(
                    json!({"id": 1, "memory_type": memory_type})
                )
                .is_err()
            );
        }
    }

    #[test]
    fn parameter_schemas_expose_memory_type_guidance() {
        for (schema, description_path) in [
            (
                serde_json::to_value(schemars::schema_for!(StoreParams)).unwrap(),
                "/$defs/StoreItem/properties/memory_type/description",
            ),
            (
                serde_json::to_value(schemars::schema_for!(UpdateParams)).unwrap(),
                "/properties/memory_type/description",
            ),
        ] {
            let description = schema.pointer(description_path).unwrap().as_str().unwrap();
            for guidance in [
                "Classify content, not activity or source",
                "Research:",
                "Feedback:",
                "Keep problem records",
                "Recommendations alone",
                "Project-local assessments",
                "Use confidence",
            ] {
                assert!(description.contains(guidance), "missing {guidance}");
            }

            // Checking shared type boundaries alongside field-level guidance
            let types = schema["$defs"]["MemoryType"].to_string();
            for guidance in [
                "shared repository rules belong to project",
                "choice rationale belongs to decision",
                "research synthesis",
                "unfinished research",
                "confirmed defects belong to issue",
                "remediation actions belong to task",
                "a recommendation alone belongs to finding",
                "a single action belongs to task",
            ] {
                assert!(types.contains(guidance), "missing {guidance}");
            }
        }
    }

    #[test]
    fn parameter_schemas_reference_domain_enums() {
        let schema = serde_json::to_value(schemars::schema_for!(SearchParams)).unwrap();
        for name in [
            "MemoryType",
            "MemoryStatus",
            "MemoryAttrSeverity",
            "MemoryAttrPriority",
            "MemoryAttrOutcome",
            "MemoryAttrPhase",
        ] {
            assert!(schema["$defs"][name].is_object(), "missing {name} schema");
        }
        let schema = serde_json::to_value(schemars::schema_for!(TraverseParams)).unwrap();
        assert!(schema["$defs"]["TraverseDirection"].is_object());
    }
}
