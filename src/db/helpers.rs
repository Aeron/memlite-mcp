use std::collections::{BTreeMap, HashMap, HashSet};

use anyhow::{Result, anyhow};
use libsql::Value;

use super::options::FilterOptions;
use super::types::{Memory, MemoryAttrs};

/// Fetches full memory rows for a set of ids, returning a map of id -> Memory.
/// Missing ids (deleted memories) are silently absent from the result.
pub(super) async fn fetch_memories_by_ids(
    conn: &libsql::Connection,
    ids: &[i64],
) -> Result<HashMap<i64, Memory>> {
    if ids.is_empty() {
        return Ok(HashMap::new());
    }
    let placeholders = vec!["?"; ids.len()].join(", ");
    let sql = format!(
        "select
            rowid, content, summary, tags, created_at, updated_at, context,
            confidence, decay_rate, last_seen_at, type, relations, refs, attrs
         from memories
         where rowid in ({placeholders})"
    );
    let params: Vec<Value> = ids.iter().map(|&id| Value::Integer(id)).collect();
    let mut rows = conn.query(&sql, libsql::params_from_iter(params)).await?;
    let mut map = HashMap::new();
    while let Some(row) = rows.next().await? {
        let m = parse_memory_row(&row)?;
        map.insert(m.id, m);
    }
    Ok(map)
}

// Builds a SQL WHERE clause from FilterOptions, appending bind params in placeholder order.
pub(super) fn build_where(filter: &FilterOptions<'_>, params: &mut Vec<Value>) -> Result<String> {
    let mut clauses: Vec<String> = Vec::new();
    if let Some(ids) = filter.ids {
        if ids.is_empty() {
            clauses.push("0".into());
        } else {
            let placeholders = vec!["?"; ids.len()].join(", ");
            clauses.push(format!("rowid in ({placeholders})"));
            params.extend(ids.iter().copied().map(Value::Integer));
        }
    }
    if let Some(ts) = filter.tags.filter(|t| !t.is_empty()) {
        // Using normalized memory_tags table for indexed filtering
        clauses.push(
            "rowid in (
                select memory_id from memory_tags
                where tag in (
                    select value from json_each(?)
                )
                group by memory_id having count(distinct tag) = ?
            )"
            .into(),
        );
        params.push(Value::Text(serde_json::to_string(ts)?));
        params.push(Value::Integer(ts.len() as i64));
    }
    if let Some(mt) = filter.memory_type {
        clauses.push("type = ?".into());
        params.push(Value::Text(mt.to_string()));
    }
    if let Some(sf) = filter.status {
        clauses.push("json_extract(attrs, '$.status') = ?".into());
        params.push(Value::Text(sf.to_string()));
    }
    if let Some(sev) = filter.severity {
        clauses.push("json_extract(attrs, '$.severity') = ?".into());
        params.push(Value::Text(sev.to_string()));
    }
    if let Some(p) = filter.priority {
        clauses.push("json_extract(attrs, '$.priority') = ?".into());
        params.push(Value::Text(p.to_string()));
    }
    if let Some(o) = filter.outcome {
        clauses.push("json_extract(attrs, '$.outcome') = ?".into());
        params.push(Value::Text(o.to_string()));
    }
    if let Some(ph) = filter.phase {
        clauses.push("json_extract(attrs, '$.phase') = ?".into());
        params.push(Value::Text(ph.to_string()));
    }
    if let Some(rc) = filter.refs_contain {
        clauses
            .push("exists (select 1 from json_each(refs) jt where instr(jt.value, ?) > 0)".into());
        params.push(Value::Text(rc.to_owned()));
    }
    Ok(if clauses.is_empty() {
        String::new()
    } else {
        format!("where {}", clauses.join(" and "))
    })
}

/// Deserialises a database row into a [`Memory`] value.
///
/// Expects columns in the order selected by `list` and `search`:
/// 0=rowid, 1=content, 2=summary, 3=tags,
/// 4=created_at, 5=updated_at, 6=context,
/// 7=confidence, 8=decay_rate, 9=last_seen_at, 10=type,
/// 11=relations, 12=refs, 13=attrs.
pub(super) fn parse_memory_row(row: &libsql::Row) -> Result<Memory> {
    let id: i64 = row.get(0)?;
    let content: String = row.get(1)?;
    let summary: String = row.get(2)?;
    let tags_json: String = row.get(3)?;
    let created_at: i64 = row.get(4)?;
    let updated_at: i64 = row.get(5)?;
    let context = match row.get_value(6)? {
        Value::Null => None,
        Value::Text(s) => Some(s),
        other => return Err(anyhow!("unexpected type for context: {other:?}")),
    };
    let confidence: f64 = row.get(7)?;
    let decay_rate: f64 = row.get(8)?;
    let last_seen_at: i64 = row.get(9)?;
    let memory_type = match row.get_value(10)? {
        Value::Null => None,
        Value::Text(s) => s.parse().ok(),
        other => return Err(anyhow!("unexpected value for type column: {other:?}")),
    };
    let relations_json: String = row.get(11)?;
    let relations: BTreeMap<String, String> = serde_json::from_str(&relations_json)?;
    let tags: Vec<String> = serde_json::from_str(&tags_json)?;
    let refs_json: String = row.get(12)?;
    let refs: Vec<String> = serde_json::from_str(&refs_json)?;
    let attrs_json: String = row.get(13)?;
    let attrs: MemoryAttrs = serde_json::from_str(&attrs_json)?;

    Ok(Memory {
        id,
        content,
        summary,
        tags,
        created_at,
        updated_at,
        context,
        confidence,
        decay_rate,
        last_seen_at,
        memory_type,
        attrs,
        relations,
        refs,
    })
}

/// Tokenises a text string into a deduplicated set of lowercase alphanumeric words (length >= 2).
fn word_set(text: &str) -> HashSet<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter_map(|w| {
            let w = w.to_lowercase();
            if w.len() >= 2 { Some(w) } else { None }
        })
        .collect()
}

/// Re-ranks candidates by whole-word overlap between the query and each memory's content.
///
/// Splits both query and content into deduplicated word sets so duplicate query words
/// do not inflate the score and substring matches (e.g. "is" inside "this") are excluded.
/// Sorts by overlap fraction descending, breaking ties by cosine distance ascending.
/// Truncates to `limit`. When the query has no usable words the original order is kept.
pub(super) fn rerank_by_overlap(
    query: &str,
    mut candidates: Vec<(Memory, f64)>,
    limit: usize,
) -> Vec<(Memory, f64)> {
    if limit == 0 {
        return Vec::new();
    }
    let query_words = word_set(query);
    if query_words.is_empty() {
        candidates.truncate(limit);
        return candidates;
    }
    let mut scored: Vec<(Memory, f64, f64)> = candidates
        .into_iter()
        .map(|(m, dist)| {
            let content_words = word_set(&m.content);
            let hits = query_words
                .iter()
                .filter(|w| content_words.contains(*w))
                .count();
            let overlap = hits as f64 / query_words.len() as f64;
            (m, dist, overlap)
        })
        .collect();
    // Higher overlap first; lower cosine distance as tiebreaker
    scored.sort_by(|a, b| {
        b.2.partial_cmp(&a.2)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    scored.truncate(limit);
    scored.into_iter().map(|(m, dist, _)| (m, dist)).collect()
}

/// Serialises tags after dropping empty and whitespace-only values.
pub(super) fn tags_to_json(tags: &[String]) -> Result<String> {
    let mut seen = HashSet::new();
    let tags: Vec<&String> = tags
        .iter()
        .filter(|tag| !tag.trim().is_empty() && seen.insert(tag.as_str()))
        .collect();
    Ok(serde_json::to_string(&tags)?)
}

/// Serialises directed relations into the JSON object stored in the `relations` column.
///
/// Maps each `(target_id, kind)` pair to a `{"<id>": "<kind>"}` entry, keyed by the
/// stringified id so the on-disk shape matches the BTreeMap the read path deserialises.
pub(super) fn relations_to_json(relations: &[(i64, String)]) -> Result<String> {
    let map: BTreeMap<String, &str> = relations
        .iter()
        .map(|(id, kind)| (id.to_string(), kind.as_str()))
        .collect();
    Ok(serde_json::to_string(&map)?)
}

/// Packs a float slice as little-endian IEEE 754 bytes, matching libsql's `float32` vector format.
pub(super) fn floats_to_blob(data: &[f32]) -> Vec<u8> {
    data.iter().flat_map(|f| f.to_le_bytes()).collect()
}

/// Returns the current Unix timestamp in whole seconds.
pub(super) fn unix_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

#[cfg(test)]
mod tests {
    use super::{floats_to_blob, rerank_by_overlap, tags_to_json};
    use crate::db::types::{Memory, MemoryAttrs};

    #[test]
    fn floats_to_blob_empty() {
        assert_eq!(floats_to_blob(&[]), Vec::<u8>::new());
    }

    #[test]
    fn floats_to_blob_roundtrips() {
        let vals = [1.0f32, 0.5, -0.25];
        let blob = floats_to_blob(&vals);
        assert_eq!(blob.len(), vals.len() * 4);
        let parsed: Vec<f32> = blob
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect();
        for (a, b) in vals.iter().zip(&parsed) {
            assert_eq!(a.to_bits(), b.to_bits(), "{a} != {b}");
        }
    }

    #[test]
    fn tags_to_json_drops_blank_values() {
        let tags = vec![
            "rust".to_owned(),
            String::new(),
            "  ".to_owned(),
            "rust".to_owned(),
        ];
        assert_eq!(tags_to_json(&tags).unwrap(), r#"["rust"]"#);
    }

    #[test]
    fn rerank_empty_query_preserves_cosine_order() {
        let make = |content: &str, dist: f64| -> (Memory, f64) {
            (
                Memory {
                    id: 1,
                    content: content.to_owned(),
                    summary: "summary".to_owned(),
                    tags: vec![],
                    created_at: 0,
                    updated_at: 0,
                    context: None,
                    confidence: 1.0,
                    decay_rate: 0.0,
                    last_seen_at: 0,
                    memory_type: None,
                    attrs: MemoryAttrs::default(),
                    relations: std::collections::BTreeMap::new(),
                    refs: vec![],
                },
                dist,
            )
        };
        let candidates = vec![make("a", 0.1), make("b", 0.5), make("c", 0.9)];
        let ranked = rerank_by_overlap("", candidates, 3);
        assert_eq!(ranked[0].0.content, "a");
        assert_eq!(ranked[1].0.content, "b");
        assert_eq!(ranked[2].0.content, "c");
    }

    #[test]
    fn rerank_overlap_beats_distance() {
        let make = |content: &str, dist: f64| -> (Memory, f64) {
            (
                Memory {
                    id: 1,
                    content: content.to_owned(),
                    summary: "summary".to_owned(),
                    tags: vec![],
                    created_at: 0,
                    updated_at: 0,
                    context: None,
                    confidence: 1.0,
                    decay_rate: 0.0,
                    last_seen_at: 0,
                    memory_type: None,
                    attrs: MemoryAttrs::default(),
                    relations: std::collections::BTreeMap::new(),
                    refs: vec![],
                },
                dist,
            )
        };
        // "rust tips" is farther (dist 0.9) but has two-word overlap.
        // "python docs" is closer (dist 0.1) but has no overlap
        let candidates = vec![make("python docs", 0.1), make("rust tips guide", 0.9)];
        let ranked = rerank_by_overlap("rust tips", candidates, 2);
        assert_eq!(ranked[0].0.content, "rust tips guide");
        assert_eq!(ranked[1].0.content, "python docs");
    }
}
