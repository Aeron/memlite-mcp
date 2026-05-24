# memlite-mcp

A local-only MCP server that gives AI agents persistent, searchable memory,
backed by **libsql** (SQLite with native vector search) and **fastembed**
(on-device ONNX embeddings — no API keys, no cloud).

## Motivation

AI agents are stateless by design. Every new conversation starts cold: no
memory of what was figured out last session, what decisions were made, what
dead ends were already explored. For quick, self-contained tasks that is fine.
For anything longer — an evolving codebase, an ongoing investigation, a project
with accumulated conventions — it becomes a real friction point. The agent keeps
rediscovering the same things, and you keep re-explaining them.

The standard answer is some cloud memory service. That means API keys, a
dependency on a third-party endpoint, and context leaving the machine. None of
that felt right for a tool I wanted to run silently in the background across
all my local projects.

memlite-mcp takes the other approach: the DB is a per-project SQLite file on
disk, the embeddings are generated locally with a bundled ONNX model, and
nothing ever leaves the machine. Per-project isolation means agents working on
different codebases do not bleed into each other. Per-agent filtering means
multiple agents sharing a project can coexist without noise. And since only
`memory_store` actually creates the DB file, projects you never ask the agent
to remember anything about do not accumulate empty database files.

## Build

```sh
cargo build --release
```

The first run downloads the default embedding model and caches it in the OS
cache directory (`~/Library/Caches/memlite-mcp` on macOS,
`~/.cache/memlite-mcp` on Linux). Subsequent runs are fully offline.

## Configuration

All options are available as CLI flags and env vars (Clap resolves both;
flags take precedence).

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--project-path` | `MEMLITE_PROJECT_PATH` | first MCP workspace root, or CWD | Project identifier; agents sharing the same path share a DB |
| `--agent` | `MEMLITE_AGENT` | `""` | Filters memories to this agent (empty = see all) |
| `--db-dir` | `MEMLITE_DB_DIR` | `~/.local/share/memlite-mcp` | Directory for per-project DB files |
| `--in-memory` | `MEMLITE_IN_MEMORY` | `false` | `true`/`1`/`yes` for ephemeral in-memory DB |
| `--embedding-model` | `MEMLITE_EMBEDDING_MODEL` | `bge-small-en-v1.5` | See models table below |
| `--log-file` | `MEMLITE_LOG_FILE` | — | Write logs to this file instead of stderr |
| — | `RUST_LOG` | `info` | Log level (written to stderr) |

Run `memlite-mcp --help` for the full usage listing.

### Embedding models

| Model | Dimensions | Download size |
|---|---|---|
| `bge-small-en-v1.5` (default) | 384 | ~67 MB |
| `bge-base-en-v1.5` | 768 | ~210 MB |
| `bge-large-en-v1.5` | 1024 | ~620 MB |

## Trust and decay

Each memory carries a `confidence` score (base trust, 0.0–1.0) and an optional
`decay_rate` (per-day reduction, 0.0–1.0). At query time the server computes
`effective_confidence = confidence - decay_rate * days_since_last_seen`,
clamped to [0, 1]. `memory_search` results include this derived value together
with `age_days` so agents can judge freshness without parsing timestamps.

Set `confidence < 1.0` when storing uncertain or speculative information.
Set `decay_rate > 0.0` for time-sensitive facts that naturally become stale
(e.g. `0.01` decays ~1% per day). Calling `memory_update` on any field resets
`last_seen_at` to now, restarting the decay clock — use this to revalidate a
memory confirmed still accurate without changing its content.

## Memory types

The optional `memory_type` field classifies a memory's intended use, enabling targeted filtering in `memory_list` and `memory_search`.

| Type | Stores |
|---|---|
| `user` | Facts about the person: preferences, identity, working style |
| `project` | Codebase knowledge: conventions, stack, decisions, constraints |
| `reference` | Domain knowledge: docs, external facts, API details, terminology |
| `task` | Things to track, follow up on, or complete in a future session |
| `finding` | Discovered problem, audit result, or identified risk; not yet a tracked task |
| `issue` | Known defect or bug |
| `decision` | Architectural or design decision record: what was chosen, what rejected, and why |

## Lifecycle and severity

Tasks, issues, and findings carry an optional `status` field:

| Value | Meaning |
|---|---|
| `open` | Active and unresolved |
| `resolved` | Completed or fixed |
| `wontfix` | Acknowledged, will not be addressed |
| `deferred` | Postponed for later |

Do not delete resolved memories — they are useful history. Update `status` to `resolved` instead and leave the record intact for future sessions. Both `memory_search` and `memory_list` accept `status` as a filter.

The optional `severity` field (`critical` / `high` / `medium` / `low`) captures urgency for findings, issues, and tasks. Most meaningful combined with `status`; both are independently filterable.

## Relations

Memories can carry directed outgoing links via the `relations` field: a JSON object mapping a target memory id (string key) to a free-form relation kind:

```json
{"7": "caused_by", "12": "blocks"}
```

Links are one-way — the target memory is unmodified. Common kinds: `caused_by`, `causes`, `blocks`, `depends_on`, `supersedes`, `related_to`, `duplicate_of`. Kind strings are free-form; pick a label and use it consistently within a project. `memory_update` replaces the full set; pass `{}` to clear all links.

## Tools

| Tool | Description |
|---|---|
| `memory_store` | Embeds and persists a memory; returns `{"id"}`. Required: `content`, `memory_type`. Optional: `summary`, `tags`, `context`, `confidence` ([0, 1], default 1.0), `decay_rate` (per-day decay, default 0.0), `status`, `severity`, `relations` |
| `memory_store_batch` | Stores up to 100 memories atomically in a single embedding pass; accepts `items: [{...}]` with the same fields as `memory_store`; returns `[{"id"}]` in input order |
| `memory_search` | Two-stage semantic search (cosine + keyword re-rank), nearest-first; optional `tags`, `memory_type`, `status`, `severity` filters; returns `effective_confidence` and `age_days` per result |
| `memory_list` | Paginated list newest-first; optional `tags`, `memory_type`, `status`, `severity` filters; returns raw `confidence`, `decay_rate`, `last_seen_at` |
| `memory_tags` | Lists all distinct tags with occurrence counts, ordered by frequency |
| `memory_delete` | Deletes a memory by numeric id |
| `memory_update` | Updates any field in-place; re-embeds on content change; resets decay clock; pass `""` to clear text fields, `{}` to clear relations |
