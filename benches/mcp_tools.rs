//! End-to-end benchmarks for the memlite-mcp MCP tools.
//!
//! Spawns the server binary as a child process and drives it via JSON-RPC
//! over stdio, measuring round-trip latency per tool call at steady state.
//!
//! Each benchmark group creates its own server process so corpus state
//! does not bleed between groups.
//!
//! Run: `cargo bench`

use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::Duration;

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use serde_json::{Value, json};
use tempfile::TempDir;

// Content used for write/warm iterations.
const BENCH_CONTENT: &str = "notes on Rust async runtime design: tokio executor, task scheduling, and backpressure handling";

// Query used for search/n100 iterations.
const BENCH_QUERY: &str = "async runtime and task scheduling";

// Varied strings cycled when seeding a corpus for read benchmarks.
const SEED_CONTENT: &[&str] = &[
    "Rust ownership model and borrow checker semantics for safe concurrency",
    "database indexing strategies for high-write workloads using B-trees",
    "REST API design with versioning, pagination, and backward compatibility",
    "container orchestration: Kubernetes pod lifecycle and health probes",
    "distributed tracing with OpenTelemetry spans and context propagation",
    "authentication flows: OAuth2 PKCE, JWT validation, and token rotation",
    "compiler frontend: lexer, recursive-descent parser, and typed AST",
    "ML inference pipeline: feature normalization and model serving latency",
];

/// Drives a spawned memlite-mcp process via synchronous JSON-RPC over stdio.
///
/// Communicates using newline-delimited JSON (one complete object per line),
/// which is the MCP stdio transport format.
struct McpClient {
    _child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl McpClient {
    /// Spawns the server binary and completes the MCP initialization handshake.
    ///
    /// Uses a caller-supplied temp directory as `--db-dir` so the database is
    /// isolated from real data and is cleaned up after the benchmark group.
    /// File-based SQLite is used (not in-memory) so all connections from the
    /// spawned process share the same schema after migrations run.
    fn new(db_dir: &Path) -> Self {
        // env!("CARGO_BIN_EXE_memlite-mcp") is set by Cargo when building benches;
        // it points to the binary compiled under [profile.bench], which inherits
        // [profile.release] so the binary is release-quality.
        let binary = env!("CARGO_BIN_EXE_memlite-mcp");
        let mut child = Command::new(binary)
            .args(["--project-path", "/bench", "--db-dir"])
            .arg(db_dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            // suppressing server tracing output so it does not appear in bench output
            .stderr(Stdio::null())
            .spawn()
            .expect("failed to spawn memlite-mcp");
        let stdin = BufWriter::new(child.stdin.take().unwrap());
        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut c = McpClient {
            _child: child,
            stdin,
            stdout,
            next_id: 1,
        };
        c.handshake();
        c
    }

    /// Drives the initialize / initialized / wait-for-ready sequence.
    fn handshake(&mut self) {
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {"name": "bench", "version": "0.1.0"}
            }
        }));
        // Draining messages until the server's initialize response arrives (id == 0)
        loop {
            let msg = self.read_msg();
            if msg.get("id").and_then(Value::as_u64) == Some(0) {
                break;
            }
        }
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
            "params": {}
        }));
        self.wait_for_ready();
    }

    /// Polls memory_list until on_initialized has resolved the project path.
    ///
    /// on_initialized runs asynchronously after the initialized notification;
    /// tool calls issued before it completes return a PathNotResolved JSON-RPC
    /// error. Polls at 100 ms intervals for up to 5 s.
    fn wait_for_ready(&mut self) {
        for _ in 0..50 {
            let id = self.next_id;
            self.next_id += 1;
            self.send_raw(&json!({
                "jsonrpc": "2.0",
                "id": id,
                "method": "tools/call",
                "params": {"name": "memory_list", "arguments": {"limit": 1}}
            }));
            loop {
                let msg = self.read_msg();
                if msg.get("id").and_then(Value::as_u64) == Some(id) {
                    if msg.get("result").is_some() {
                        return; // valid result: server is ready
                    }
                    break; // JSON-RPC error (PathNotResolved); retrying after sleep
                }
                // discarding non-matching messages (server notifications, if any)
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("memlite-mcp did not become ready within 5 s");
    }

    fn send_raw(&mut self, msg: &Value) {
        let mut line = serde_json::to_string(msg).expect("serialization failed");
        line.push('\n');
        self.stdin
            .write_all(line.as_bytes())
            .expect("write to server stdin failed");
        self.stdin.flush().expect("flush to server stdin failed");
    }

    fn read_msg(&mut self) -> Value {
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("read from server stdout failed");
        serde_json::from_str(line.trim_end()).expect("invalid JSON from server")
    }

    /// Sends a tools/call request and returns the result object.
    ///
    /// Panics on JSON-RPC errors; benchmarks should not encounter errors during
    /// steady-state iteration.
    fn call_tool(&mut self, name: &str, args: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        self.send_raw(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {"name": name, "arguments": args}
        }));
        loop {
            let msg = self.read_msg();
            if msg.get("id").and_then(Value::as_u64) == Some(id) {
                if let Some(err) = msg.get("error") {
                    panic!("tool {name} returned a JSON-RPC error: {err}");
                }
                return msg["result"].clone();
            }
        }
    }

    /// Embeds `content` and inserts a memory row, returning its assigned identifier.
    fn store(&mut self, content: &str, tags: &[&str]) -> i64 {
        let result = self.call_tool(
            "memory_store",
            json!({"content": content, "tags": tags, "memory_type": "reference"}),
        );
        let text = result["content"][0]["text"].as_str().unwrap();
        serde_json::from_str::<Value>(text).unwrap()["id"]
            .as_i64()
            .unwrap()
    }

    /// Runs a semantic search; discards results (measuring latency only).
    fn search(&mut self, query: &str, limit: u32) {
        self.call_tool("memory_search", json!({"query": query, "limit": limit}));
    }

    /// Lists memories; discards results.
    fn list(&mut self, limit: u32) {
        self.call_tool("memory_list", json!({"limit": limit}));
    }

    /// Fetches tag counts; discards results.
    fn tags(&mut self) {
        self.call_tool("memory_tags", json!({}));
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self._child.kill();
    }
}

/// Seeds `count` memories via `memory_store_batch` (100 items per call), cycling through `SEED_CONTENT`.
///
/// Batching keeps seeding time manageable for large corpora: ONNX runs one
/// inference pass per batch instead of per item. Also warms the ONNX runtime
/// as a side effect; subsequent calls reflect steady-state latency.
///
/// Approximate seeding times (bge-small, M-series Mac): 100 ~1s, 1k ~5s,
/// 10k ~30s, 100k ~5min.
fn seed(client: &mut McpClient, count: usize) {
    const BATCH: usize = 100;
    let mut i = 0;
    while i < count {
        let n = (count - i).min(BATCH);
        let items: Vec<Value> = (0..n)
            .map(|j| {
                json!({
                    "content": SEED_CONTENT[(i + j) % SEED_CONTENT.len()],
                    "tags": ["seeded"],
                    "memory_type": "reference"
                })
            })
            .collect();
        client.call_tool("memory_store_batch", json!({"items": items}));
        i += n;
    }
}

fn bench_write(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut client = McpClient::new(tmp.path());
    // Priming the ONNX runtime before the timed section; first call includes
    // JIT compilation of the ONNX graph which is a one-time cost
    client.store("warmup", &[]);

    let mut group = c.benchmark_group("write");
    group.measurement_time(Duration::from_secs(20));
    group.sample_size(30);
    group.bench_function("warm", |b| {
        b.iter(|| client.store(BENCH_CONTENT, &["bench"]));
    });
    group.finish();
}

fn bench_search(c: &mut Criterion) {
    let mut group = c.benchmark_group("search");
    group.measurement_time(Duration::from_secs(30));
    group.sample_size(20);

    // Each corpus size gets its own isolated server process and DB.
    // Seeding happens before the timed section; see seed() for time estimates.
    for &n in &[100usize, 1_000, 10_000, 100_000] {
        let tmp = TempDir::new().unwrap();
        let mut client = McpClient::new(tmp.path());
        seed(&mut client, n);
        group.bench_with_input(BenchmarkId::new("n", n), &n, |b, _| {
            b.iter(|| client.search(BENCH_QUERY, 10));
        });
    }

    group.finish();
}

fn bench_list(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut client = McpClient::new(tmp.path());
    seed(&mut client, 100);

    let mut group = c.benchmark_group("list");
    group.bench_function("n100", |b| {
        b.iter(|| client.list(20));
    });
    group.finish();
}

fn bench_tags(c: &mut Criterion) {
    let tmp = TempDir::new().unwrap();
    let mut client = McpClient::new(tmp.path());
    seed(&mut client, 100);

    let mut group = c.benchmark_group("tags");
    group.bench_function("n100", |b| {
        b.iter(|| client.tags());
    });
    group.finish();
}

criterion_group!(benches, bench_write, bench_search, bench_list, bench_tags);
criterion_main!(benches);
