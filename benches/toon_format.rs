//! Compares compact JSON and TOON encoding for representative MCP result payloads.

use std::{fmt::Write, hint::black_box};

use criterion::{Criterion, criterion_group, criterion_main};
use serde_json::{Value, json};

const PROSE: &str = "Investigated MCP result serialization for agent memory.\nRoot cause: repeated object keys consume context while prose fields require careful quoting.\nDecision: preserve the JSON-RPC envelope and encode only structured tool text as TOON v4.1.";

fn search_payload() -> Value {
    let rows: Vec<_> = (0..20)
        .map(|i| {
            let mut row = json!({
                "id": i + 1,
                "content": format!("{PROSE}\nRecord: {i}"),
                "summary": "TOON output format investigation and decision",
                "tags": ["memlite-mcp", "mcp", "performance"],
                "created_at": "2026-07-25T18:40:00Z",
                "updated_at": "2026-07-25T18:41:40Z",
                "distance": 0.1234,
                "effective_confidence": 0.975,
                "memory_type": "finding",
                "attrs": {"status": "open", "severity": "low"},
                "relations": ["49:implements"],
                "refs": ["src/mcp/server.rs", "benches/toon_format.rs"]
            });
            if i % 2 == 0 {
                row.as_object_mut().unwrap().remove("relations");
            }
            if i % 3 == 0 {
                let fields = row.as_object_mut().unwrap();
                fields.remove("attrs");
                fields.remove("refs");
            }
            row
        })
        .collect();
    Value::Array(rows)
}

fn search_block_text(payload: &Value) -> String {
    let rows = payload.as_array().unwrap();
    if rows.is_empty() {
        return "memories: 0".to_owned();
    }

    let mut output = String::new();
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            output.push('\n');
        }

        write!(
            output,
            "# {} {}",
            row["id"].as_i64().unwrap(),
            row["memory_type"].as_str().unwrap()
        )
        .unwrap();
        if let Some(attrs) = row.get("attrs").and_then(Value::as_object) {
            let values: Vec<String> = [
                "status",
                "severity",
                "priority",
                "outcome",
                "reversible",
                "phase",
            ]
            .into_iter()
            .filter_map(|key| {
                attrs.get(key).map(|value| {
                    let value = value
                        .as_str()
                        .map_or_else(|| value.to_string(), str::to_owned);
                    format!("{key}={value}")
                })
            })
            .collect();
            if !values.is_empty() {
                write!(output, " ({})", values.join(", ")).unwrap();
            }
        }
        writeln!(
            output,
            " distance={} effective_confidence={}",
            row["distance"], row["effective_confidence"]
        )
        .unwrap();

        writeln!(output, "summary: {}", row["summary"].as_str().unwrap()).unwrap();
        writeln!(
            output,
            "updated_at: {} (created_at {})",
            row["updated_at"].as_str().unwrap(),
            row["created_at"].as_str().unwrap()
        )
        .unwrap();
        if let Some(tags) = row.get("tags").and_then(Value::as_array) {
            let tags: Vec<&str> = tags.iter().map(|value| value.as_str().unwrap()).collect();
            writeln!(output, "tags: {}", tags.join(", ")).unwrap();
        }
        if let Some(relations) = row.get("relations").and_then(Value::as_array) {
            let relations: Vec<&str> = relations
                .iter()
                .map(|value| value.as_str().unwrap())
                .collect();
            writeln!(output, "relations: {}", relations.join(", ")).unwrap();
        }
        if let Some(refs) = row.get("refs").and_then(Value::as_array) {
            let refs: Vec<&str> = refs.iter().map(|value| value.as_str().unwrap()).collect();
            writeln!(output, "refs: {}", refs.join(", ")).unwrap();
        }
        writeln!(output, "content:").unwrap();
        for line in row["content"].as_str().unwrap().split('\n') {
            writeln!(output, "  {line}").unwrap();
        }
    }
    output
}

fn hubs_payload() -> Value {
    Value::Array(
        (0..20)
            .map(|i| {
                json!({
                    "id": i + 1,
                    "summary": format!("Architecture hub {i}"),
                    "memory_type": "project",
                    "out_degree": 8 + i,
                    "in_degree": 5 + i
                })
            })
            .collect(),
    )
}

fn graph_summary(i: usize) -> String {
    format!("Graph memory node {i}: compact selection row for deferred retrieval")
}

fn neighbors_payload() -> Value {
    let rows = |start: usize| -> Vec<Value> {
        (start..start + 10)
            .map(|i| {
                json!({
                    "id": i + 1,
                    "kind": "related_to",
                    "memory_type": "project",
                    "summary": graph_summary(i),
                })
            })
            .collect()
    };
    json!({"outgoing": rows(0), "incoming": rows(10)})
}

fn traverse_payload() -> Value {
    let nodes: Vec<_> = (0..20)
        .map(|i| {
            json!({
                "id": i + 1,
                "depth": i % 4,
                "via_kind": if i == 0 { "" } else { "related_to" },
                "memory_type": "project",
                "summary": graph_summary(i),
            })
        })
        .collect();
    let edges: Vec<_> = (1..20)
        .map(|i| json!({"from_id": i, "to_id": i + 1, "kind": "related_to"}))
        .collect();
    json!({"start_id": 1, "nodes": nodes, "edges": edges})
}

fn path_payload() -> Value {
    let path: Vec<_> = (0..10)
        .map(|i| {
            json!({
                "id": i + 1,
                "via_kind": if i == 0 { "" } else { "related_to" },
                "memory_type": "project",
                "summary": graph_summary(i),
            })
        })
        .collect();
    json!({"found": true, "path": path})
}

fn payloads() -> Vec<(&'static str, Value)> {
    vec![
        ("store_100", json!({"ids": (1..=100).collect::<Vec<_>>()})),
        ("search_20_mixed_prose", search_payload()),
        ("hubs_20_uniform", hubs_payload()),
        ("neighbors_20_uniform", neighbors_payload()),
        ("traverse_20_uniform", traverse_payload()),
        ("path_10_uniform", path_payload()),
        (
            "tags_20",
            json!({
                "memlite-mcp": 46,
                "architecture": 20,
                "schema": 9,
                "feature": 11,
                "performance": 3,
                "testing": 1,
                "benchmark": 1,
                "mcp": 3,
                "db": 12,
                "graph": 2,
                "embeddings": 4,
                "config": 5,
                "refactor": 6,
                "documentation": 1,
                "rust": 1,
                "tools": 3,
                "relations": 1,
                "search": 4,
                "integration": 1,
                "optimization": 1
            }),
        ),
    ]
}

fn bench_formats(c: &mut Criterion) {
    for (name, payload) in payloads() {
        let json = serde_json::to_string(&payload).unwrap();
        let toon = serde_toon::to_string(&payload).unwrap();
        let decoded: Value = serde_toon::from_str(&toon).unwrap();
        assert_eq!(decoded, payload);

        if name == "search_20_mixed_prose" {
            let block = search_block_text(&payload);
            println!(
                "format_size case={name} json_bytes={} toon_bytes={} block_bytes={} toon_ratio={:.3} block_ratio={:.3} json_chars={} toon_chars={} block_chars={}",
                json.len(),
                toon.len(),
                block.len(),
                toon.len() as f64 / json.len() as f64,
                block.len() as f64 / json.len() as f64,
                json.chars().count(),
                toon.chars().count(),
                block.chars().count()
            );
        } else {
            println!(
                "format_size case={name} json_bytes={} toon_bytes={} toon_ratio={:.3} json_chars={} toon_chars={}",
                json.len(),
                toon.len(),
                toon.len() as f64 / json.len() as f64,
                json.chars().count(),
                toon.chars().count()
            );
        }

        let mut group = c.benchmark_group(format!("format_encode/{name}"));
        group.bench_function("json", |b| {
            b.iter(|| serde_json::to_string(black_box(&payload)).unwrap())
        });
        group.bench_function("toon", |b| {
            b.iter(|| serde_toon::to_string(black_box(&payload)).unwrap())
        });
        if name == "search_20_mixed_prose" {
            group.bench_function("block", |b| {
                b.iter(|| search_block_text(black_box(&payload)))
            });
        }
        group.finish();
    }
}

criterion_group!(benches, bench_formats);
criterion_main!(benches);
