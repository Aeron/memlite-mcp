use std::fmt::Write;

use crate::db::types::{
    GraphStats, HubEntry, Memory, NeighborInfo, Neighbors, PathStep, TraverseResult,
};

const CONTENT_PREVIEW_CHARS: usize = 120;

/// Renders semantic search results as compact blocks with literal prose.
pub(crate) fn render_search_results(results: &[(Memory, f64)], include_content: bool) -> String {
    if results.is_empty() {
        return "memories: 0".to_owned();
    }

    let mut output = String::new();
    for (index, (memory, distance)) in results.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        render_memory(&mut output, memory, Some(*distance), false, include_content);
    }
    output
}

/// Renders chronological memory results as compact blocks with literal prose.
pub(crate) fn render_list_results(memories: &[Memory], include_content: bool) -> String {
    if memories.is_empty() {
        return "memories: 0".to_owned();
    }

    let mut output = String::new();
    for (index, memory) in memories.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        render_memory(&mut output, memory, None, true, include_content);
    }
    output
}

/// Converts chronological results into JSON while honoring content selection.
pub(crate) fn list_json(memories: &[Memory], include_content: bool) -> serde_json::Value {
    serde_json::Value::Array(
        memories
            .iter()
            .map(|memory| memory_json(memory, None, include_content))
            .collect(),
    )
}

/// Converts semantic results into JSON while honoring content selection.
pub(crate) fn search_json(results: &[(Memory, f64)], include_content: bool) -> serde_json::Value {
    serde_json::Value::Array(
        results
            .iter()
            .map(|(memory, distance)| memory_json(memory, Some(*distance), include_content))
            .collect(),
    )
}

/// Converts immediate neighbors into compact selection rows.
pub(crate) fn neighbors_json(neighbors: Neighbors) -> serde_json::Value {
    serde_json::json!({
        "outgoing": neighbors.outgoing.into_iter().map(neighbor_json).collect::<Vec<_>>(),
        "incoming": neighbors.incoming.into_iter().map(neighbor_json).collect::<Vec<_>>(),
    })
}

/// Converts traversal results into compact node and edge rows.
pub(crate) fn traverse_json(result: TraverseResult) -> serde_json::Value {
    let nodes: Vec<_> = result
        .nodes
        .into_iter()
        .map(|node| {
            let memory = node.memory;
            serde_json::json!({
                "id": memory.id,
                "depth": node.depth,
                "via_kind": node.via_kind.unwrap_or_default(),
                "memory_type": memory.memory_type.map_or_else(
                    || "untyped".to_owned(),
                    |value| value.to_string(),
                ),
                "summary": memory.summary.unwrap_or_default(),
            })
        })
        .collect();
    let edges: Vec<_> = result
        .edges
        .into_iter()
        .map(|(from_id, to_id, kind)| {
            serde_json::json!({"from_id": from_id, "to_id": to_id, "kind": kind})
        })
        .collect();
    serde_json::json!({"start_id": result.start_id, "nodes": nodes, "edges": edges})
}

/// Converts an optional path into compact selection rows.
pub(crate) fn path_json(result: Option<Vec<PathStep>>) -> serde_json::Value {
    let Some(steps) = result else {
        return serde_json::json!({"found": false, "path": []});
    };
    let path: Vec<_> = steps
        .into_iter()
        .map(|step| {
            let memory = step.memory;
            serde_json::json!({
                "id": memory.id,
                "via_kind": step.via_kind.unwrap_or_default(),
                "memory_type": memory.memory_type.map_or_else(
                    || "untyped".to_owned(),
                    |value| value.to_string(),
                ),
                "summary": memory.summary.unwrap_or_default(),
            })
        })
        .collect();
    serde_json::json!({"found": true, "path": path})
}

/// Converts hub entries into compact ranking rows.
pub(crate) fn hubs_json(entries: Vec<HubEntry>) -> serde_json::Value {
    serde_json::Value::Array(
        entries
            .into_iter()
            .map(|entry| {
                serde_json::json!({
                    "id": entry.id,
                    "summary": entry.summary.unwrap_or_default(),
                    "memory_type": entry.memory_type.map_or_else(
                        || "untyped".to_owned(),
                        |value| value.to_string(),
                    ),
                    "out_degree": entry.out_degree,
                    "in_degree": entry.in_degree,
                })
            })
            .collect(),
    )
}

/// Converts graph statistics into the public result shape.
pub(crate) fn stats_json(stats: GraphStats) -> serde_json::Value {
    let by_type = serde_json::Value::Object(
        stats
            .by_type
            .into_iter()
            .map(|(memory_type, count)| {
                (
                    memory_type.unwrap_or_else(|| "untyped".to_owned()),
                    serde_json::Value::from(count),
                )
            })
            .collect(),
    );
    serde_json::json!({
        "total_memories": stats.total_memories,
        "by_type": by_type,
        "total_relations": stats.total_relations,
        "relation_kinds": stats.relation_kinds,
    })
}

/// Encodes compact structured output as TOON text.
pub(crate) fn render_toon(mut value: serde_json::Value) -> Result<String, serde_toon::Error> {
    compact_value(&mut value);
    serde_toon::to_string(&value)
}

fn neighbor_json(neighbor: NeighborInfo) -> serde_json::Value {
    let (memory_type, summary) = neighbor.memory.map_or_else(
        || ("untyped".to_owned(), String::new()),
        |memory| {
            (
                memory
                    .memory_type
                    .map_or_else(|| "untyped".to_owned(), |value| value.to_string()),
                memory.summary.unwrap_or_default(),
            )
        },
    );
    serde_json::json!({
        "id": neighbor.other_id,
        "kind": neighbor.kind,
        "memory_type": memory_type,
        "summary": summary,
    })
}

fn memory_json(memory: &Memory, distance: Option<f64>, include_content: bool) -> serde_json::Value {
    let mut value = serde_json::to_value(memory).expect("Memory must serialize");
    let fields = value
        .as_object_mut()
        .expect("Memory must serialize as an object");
    fields.insert(
        "effective_confidence".to_owned(),
        serde_json::Value::from(memory.effective_confidence()),
    );
    if let Some(distance) = distance {
        fields.insert("distance".to_owned(), serde_json::Value::from(distance));
    }
    if let Some(freshness) = memory.freshness_warning() {
        fields.insert("freshness".to_owned(), serde_json::Value::from(freshness));
    }
    if include_content {
        fields.remove("summary");
    } else {
        fields.remove("content");
        fields.remove("context");
        if memory.summary.as_deref().is_none_or(str::is_empty) {
            fields.insert(
                "content_preview".to_owned(),
                serde_json::Value::from(content_preview(&memory.content)),
            );
        }
    }
    value
}

fn compact_value(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                compact_value(item);
            }
        }
        serde_json::Value::Object(fields) => {
            for value in fields.values_mut() {
                compact_value(value);
            }
            fields.retain(|_, value| {
                !matches!(value, serde_json::Value::Null)
                    && !matches!(value, serde_json::Value::Array(items) if items.is_empty())
                    && !matches!(value, serde_json::Value::Object(fields) if fields.is_empty())
            });
        }
        _ => {}
    }
}

fn render_memory(
    output: &mut String,
    memory: &Memory,
    distance: Option<f64>,
    include_decay: bool,
    include_content: bool,
) {
    // Writing identity and scoring fields in the header
    let memory_type = memory
        .memory_type
        .map_or_else(|| "untyped".to_owned(), |value| value.to_string());
    write!(output, "# {} {memory_type}", memory.id).unwrap();

    let attrs = format_attrs(memory);
    if !attrs.is_empty() {
        write!(output, " ({})", attrs.join(", ")).unwrap();
    }
    if let Some(distance) = distance {
        write!(output, " distance={}", round_to(distance, 4)).unwrap();
    }
    writeln!(
        output,
        " effective_confidence={}",
        round_to(memory.effective_confidence(), 3)
    )
    .unwrap();

    // Writing compact metadata before selection or full prose
    if !include_content {
        if let Some(summary) = memory.summary.as_deref().filter(|value| !value.is_empty()) {
            write_scalar(output, "summary", summary);
        } else {
            write_scalar(output, "content_preview", &content_preview(&memory.content));
        }
    }
    writeln!(
        output,
        "updated_at: {} (created_at {})",
        format_unix_timestamp(memory.updated_at),
        format_unix_timestamp(memory.created_at)
    )
    .unwrap();
    if include_decay && memory.decay_rate > 0.0 {
        let last_seen_at = if memory.last_seen_at > 0 {
            memory.last_seen_at
        } else {
            memory.created_at
        };
        writeln!(
            output,
            "decay: confidence={} rate={} last_seen_at={}",
            memory.confidence,
            memory.decay_rate,
            format_unix_timestamp(last_seen_at)
        )
        .unwrap();
    }
    if !memory.tags.is_empty() {
        writeln!(output, "tags: {}", memory.tags.join(", ")).unwrap();
    }
    if let Some(freshness) = memory.freshness_warning() {
        writeln!(output, "freshness: {freshness}").unwrap();
    }
    if !memory.relations.is_empty() {
        let relations: Vec<String> = memory
            .relations
            .iter()
            .map(|(id, kind)| format!("{id}:{kind}"))
            .collect();
        let relations = relations.join(", ");
        writeln!(output, "relations: {relations}").unwrap();
    }
    if !memory.refs.is_empty() {
        writeln!(output, "refs: {}", memory.refs.join(", ")).unwrap();
    }

    // Writing full prose last so multiline values remain contiguous
    if include_content {
        if let Some(context) = memory.context.as_deref().filter(|value| !value.is_empty()) {
            write_scalar(output, "context", context);
        }
        write_block(output, "content", &memory.content);
    }
}

fn content_preview(content: &str) -> String {
    let normalized = content.split_whitespace().collect::<Vec<_>>().join(" ");
    if normalized.chars().count() <= CONTENT_PREVIEW_CHARS {
        return normalized;
    }

    let mut preview: String = normalized.chars().take(CONTENT_PREVIEW_CHARS - 3).collect();
    preview.push_str("...");
    preview
}

fn format_attrs(memory: &Memory) -> Vec<String> {
    let mut attrs = Vec::with_capacity(6);
    if let Some(value) = memory.attrs.status {
        attrs.push(format!("status={value}"));
    }
    if let Some(value) = memory.attrs.severity {
        attrs.push(format!("severity={value}"));
    }
    if let Some(value) = memory.attrs.priority {
        attrs.push(format!("priority={value}"));
    }
    if let Some(value) = memory.attrs.outcome {
        attrs.push(format!("outcome={value}"));
    }
    if let Some(value) = memory.attrs.reversible {
        attrs.push(format!("reversible={value}"));
    }
    if let Some(value) = memory.attrs.phase {
        attrs.push(format!("phase={value}"));
    }
    attrs
}

fn write_scalar(output: &mut String, label: &str, value: &str) {
    if value.contains('\n') {
        write_block(output, label, value);
    } else {
        writeln!(output, "{label}: {value}").unwrap();
    }
}

fn write_block(output: &mut String, label: &str, value: &str) {
    writeln!(output, "{label}:").unwrap();
    for line in value.split('\n') {
        writeln!(output, "  {line}").unwrap();
    }
}

fn round_to(value: f64, decimal_places: i32) -> f64 {
    let scale = 10_f64.powi(decimal_places);
    (value * scale).round() / scale
}

/// Formats a Unix timestamp as an RFC 3339 UTC timestamp.
fn format_unix_timestamp(timestamp: i64) -> String {
    const SECONDS_PER_DAY: i64 = 86_400;

    // Splitting with Euclidean division to support dates before 1970
    let days = timestamp.div_euclid(SECONDS_PER_DAY);
    let seconds = timestamp.rem_euclid(SECONDS_PER_DAY);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds / 3_600;
    let minute = seconds % 3_600 / 60;
    let second = seconds % 60;

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

fn civil_from_days(days_since_epoch: i64) -> (i64, i64, i64) {
    let days = days_since_epoch + 719_468;
    let era = days.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use crate::db::{MemoryAttrPhase, MemoryAttrs, MemoryType};

    use super::*;

    fn sample_memory(now: i64) -> Memory {
        Memory {
            id: 49,
            content: "line one\n\nline two".to_owned(),
            summary: Some("Output plan".to_owned()),
            tags: vec!["mcp".to_owned(), "performance".to_owned()],
            created_at: now,
            updated_at: now,
            context: None,
            confidence: 0.9,
            decay_rate: 0.0,
            last_seen_at: now,
            memory_type: Some(MemoryType::Plan),
            attrs: MemoryAttrs {
                phase: Some(MemoryAttrPhase::Draft),
                ..MemoryAttrs::default()
            },
            relations: BTreeMap::from([("50".to_owned(), "supported_by".to_owned())]),
            refs: vec!["src/mcp/server.rs".to_owned()],
        }
    }

    #[test]
    fn formats_known_unix_timestamps() {
        assert_eq!(format_unix_timestamp(-1), "1969-12-31T23:59:59Z");
        assert_eq!(format_unix_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_unix_timestamp(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn renders_memory_as_literal_block_text() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let timestamp = format_unix_timestamp(now);
        let actual = render_list_results(&[sample_memory(now)], true);
        let expected = format!(
            "# 49 plan (phase=draft) effective_confidence=0.9\nupdated_at: {timestamp} (created_at {timestamp})\ntags: mcp, performance\nrelations: 50:supported_by\nrefs: src/mcp/server.rs\ncontent:\n  line one\n  \n  line two\n"
        );

        assert_eq!(actual, expected);
        assert!(!actual.contains("attrs"));
        assert!(!actual.contains("age_days"));
        assert!(!actual.contains("\\n"));
    }

    #[test]
    fn renders_compact_results_with_summary_without_full_prose() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut memory = sample_memory(now);
        memory.context = Some("Decision rationale".to_owned());

        let actual = render_search_results(&[(memory, 0.12345)], false);

        assert!(actual.contains("summary: Output plan\n"));
        assert!(!actual.contains("context:"));
        assert!(!actual.contains("content:"));
        assert!(!actual.contains("content_preview:"));
    }

    #[test]
    fn renders_bounded_content_preview_when_summary_is_missing() {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let mut memory = sample_memory(now);
        memory.summary = None;
        memory.content = "alpha\n beta\tgamma".to_owned();

        let actual = render_search_results(&[(memory, 0.12345)], false);

        assert!(actual.contains("content_preview: alpha beta gamma\n"));
        assert_eq!(
            content_preview(&"x".repeat(121)),
            format!("{}...", "x".repeat(117))
        );
    }

    #[test]
    fn renders_empty_results_explicitly() {
        assert_eq!(render_list_results(&[], true), "memories: 0");
        assert_eq!(render_search_results(&[], false), "memories: 0");
    }
}
