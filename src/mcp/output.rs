use std::fmt::Write;

use crate::db::types::Memory;

/// Renders semantic search results as compact blocks with literal prose.
pub(super) fn render_search_results(results: &[(Memory, f64)]) -> String {
    if results.is_empty() {
        return "memories: 0".to_owned();
    }

    let mut output = String::new();
    for (index, (memory, distance)) in results.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        render_memory(&mut output, memory, Some(*distance), false);
    }
    output
}

/// Renders chronological memory results as compact blocks with literal prose.
pub(super) fn render_list_results(memories: &[Memory]) -> String {
    if memories.is_empty() {
        return "memories: 0".to_owned();
    }

    let mut output = String::new();
    for (index, memory) in memories.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }
        render_memory(&mut output, memory, None, true);
    }
    output
}

fn render_memory(output: &mut String, memory: &Memory, distance: Option<f64>, include_decay: bool) {
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

    // Writing compact metadata before literal prose blocks
    if let Some(summary) = memory.summary.as_deref().filter(|value| !value.is_empty()) {
        write_scalar(output, "summary", summary);
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
        let relations = memory
            .relations
            .iter()
            .map(|(id, kind)| format!("{id}:{kind}"))
            .collect::<Vec<_>>()
            .join(", ");
        writeln!(output, "relations: {relations}").unwrap();
    }
    if !memory.refs.is_empty() {
        writeln!(output, "refs: {}", memory.refs.join(", ")).unwrap();
    }

    // Writing prose last so multiline values remain contiguous
    if let Some(context) = memory.context.as_deref().filter(|value| !value.is_empty()) {
        write_scalar(output, "context", context);
    }
    write_block(output, "content", &memory.content);
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
        let actual = render_list_results(&[sample_memory(now)]);
        let expected = format!(
            "# 49 plan (phase=draft) effective_confidence=0.9\nsummary: Output plan\nupdated_at: {timestamp} (created_at {timestamp})\ntags: mcp, performance\nrelations: 50:supported_by\nrefs: src/mcp/server.rs\ncontent:\n  line one\n  \n  line two\n"
        );

        assert_eq!(actual, expected);
        assert!(!actual.contains("attrs"));
        assert!(!actual.contains("age_days"));
        assert!(!actual.contains("\\n"));
    }

    #[test]
    fn renders_empty_results_explicitly() {
        assert_eq!(render_list_results(&[]), "memories: 0");
        assert_eq!(render_search_results(&[]), "memories: 0");
    }
}
