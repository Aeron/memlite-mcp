# MemLite MCP

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

MemLite MCP takes the other approach: the DB is a per-project SQLite file on
disk, the embeddings are generated locally with a bundled ONNX model, and
nothing ever leaves the machine. Per-project isolation means agents working on
different codebases do not bleed into each other. And since only
`memory_store` actually creates the DB file, projects you never ask the agent
to remember anything about do not accumulate empty database files.

## Features

- **Local-only, no API keys**: one SQLite file per project via libsql; ONNX embeddings via fastembed; nothing leaves the machine
- **Semantic search**: cosine vector search with word-overlap re-rank as a second stage; no BM25, no full-text index required
- **Knowledge graph**: directed relations between memories backed by a normalized `memory_relations(from_id, to_id, kind)` table indexed for both forward and reverse lookups
- **Graph traversal**: BFS from any node (`memory_traverse`), shortest directed path (`memory_path`), immediate neighbor lookup (`memory_neighbors`), hub detection (`memory_hubs`)
- **Graph statistics**: `memory_stats` reports total memories, type distribution, total relation count, and all distinct relation kind labels
- **Confidence and decay**: per-memory `confidence` score (0.0-1.0) with optional per-day `decay_rate`; `effective_confidence` computed at query time and returned with each result
- **Revalidation**: `memory_revalidate` resets the decay clock on a list of ids without touching any other field
- **Typed attrs**: `status`, `severity`, `priority`, `outcome`, `reversible`, and `phase` live in the `attrs` JSON field; searchable where applicable
- **8 required memory types**: user, project, reference, task, finding, issue, decision, plan
- **Tag filtering**: normalized `memory_tags` table; AND semantics; usable as a pre-filter in both `memory_search` and `memory_list`
- **External references**: optional `refs` field (array of free-form strings) attaches file paths, symbol names, URLs, or issue IDs to a memory; `refs_contain` substring filter in search and list
- **Batch store**: up to 100 memories in a single ONNX embedding pass; atomic insert
- **Shape-aware output**: prose-heavy `memory_list` and `memory_search` results use readable block text; other successful structured tool content uses TOON v3.0; MCP transport and tool inputs remain JSON
- **Per-project isolation**: one DB per project path; all agents on the same project share the same DB
- **Three embedding models**: bge-small-en-v1.5 (384d, ~67 MB), bge-base-en-v1.5 (768d, ~210 MB), bge-large-en-v1.5 (1024d, ~620 MB)

## Build

```sh
cargo build --release
```

The embedding model is downloaded on the first `memory_store` or
`memory_search` call, not at server start. So, the very first tool call can be
slower. It is cached in the OS cache directory (`~/Library/Caches/memlite-mcp`
on macOS, `~/.cache/memlite-mcp` on Linux). Subsequent calls are fully offline.

### Output format benchmark

`memory_list` and `memory_search` use deterministic block text so multiline prose stays literal and empty fields disappear. Relations fold onto one line as `target_id:kind`, and Unix timestamps render as RFC 3339 UTC. Other successful structured outputs use TOON, which saves the most space on uniform object arrays.

The project benchmark compares byte/character size and encode latency for representative store, search, hubs, neighbors, traversal, path, and tags payloads. TOON cases also assert lossless round trips; the search case additionally compares the block renderer:

```sh
cargo bench --bench toon_format -- --noplot
```

A reported `toon_ratio` or `block_ratio` below 1.0 means that format is smaller than compact JSON for the case.

## Configuration

All options are available as CLI flags and env vars (Clap resolves both;
flags take precedence).

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--project-path` | `MEMLITE_PROJECT_PATH` | first MCP workspace root, or CWD | Project identifier; agents sharing the same path share a DB |
| `--db-dir` | `MEMLITE_DB_DIR` | OS data dir / `memlite-mcp` | Directory for per-project DB files (`~/.local/share/…` on Linux, `~/Library/Application Support/…` on macOS) |
| `--allow-root` | `MEMLITE_ALLOW_ROOT` | `false` | Allow `/` as the project path (off by default; almost always wrong) |
| `--allow-home` | `MEMLITE_ALLOW_HOME` | `false` | Allow the home directory as the project path (off by default; spans unrelated projects) |
| `--embedding-model` | `MEMLITE_EMBEDDING_MODEL` | `bge-small-en-v1.5` | See models table below |
| `--embedding-cache-dir` | `MEMLITE_EMBEDDING_CACHE_DIR` | OS cache dir / `memlite-mcp` | Override where model files are cached |
| `--log-file` | `MEMLITE_LOG_FILE` | — | Write logs to this file instead of stderr |
| `--log-level` | `MEMLITE_LOG_LEVEL` | `info` | Minimum log level when `RUST_LOG` is not set (`trace`/`debug`/`info`/`warn`/`error`) |
| — | `RUST_LOG` | — | Standard `tracing` filter string; overrides `--log-level` when set |

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
with readable RFC 3339 timestamps and an optional compact `freshness` warning.

Set `confidence < 1.0` when storing uncertain or speculative information.
Set `decay_rate > 0.0` for time-sensitive facts that naturally become stale
(e.g. `0.01` decays ~1% per day). Calling `memory_update` on any field resets
`last_seen_at` to now, restarting the decay clock. Use `memory_revalidate` when
confirming one or more memories still accurate without changing their content.

## Memory types

The required `memory_type` field classifies a memory's intended use, enabling targeted filtering in `memory_list` and `memory_search`.

| Type | Stores |
|---|---|
| `user` | Facts about the person: preferences, identity, working style, and personal opinions on work quality or agent approach |
| `project` | Codebase knowledge: conventions, stack, decisions, constraints |
| `reference` | Domain knowledge: docs, external facts, API details, terminology |
| `task` | Things to track, follow up on, or complete in a future session |
| `finding` | Observation, evaluation, or risk; covers audit results, review findings, quality assessments, and retrospective notes; not yet a tracked task |
| `issue` | Known defect or bug |
| `decision` | Architectural or design decision record: what was chosen, what rejected, and why |
| `plan` | High-level plan or phased roadmap; use `relations` to link to spawned tasks and decisions |

Feedback disambiguation: personal opinions on agent approach or working style -> `user`; quality or correctness observations about the work itself -> `finding` or `issue`.

## Typed attrs

`attrs` is an optional JSON object for type-specific fields. It replaces the old top-level `status` and `severity` fields.

| Field | Values | Most useful for |
|---|---|---|
| `status` | `open`, `resolved`, `wontfix`, `deferred` | task, issue, finding |
| `severity` | `critical`, `high`, `medium`, `low` | issue, finding |
| `priority` | `urgent`, `high`, `medium`, `low` | task |
| `outcome` | `accepted`, `rejected`, `superseded`, `deferred` | decision |
| `reversible` | `true`, `false` | decision |
| `phase` | `draft`, `approved`, `active`, `completed` | plan |

Do not delete resolved memories — they are useful history. Set `attrs.status` to `resolved` instead and leave the record intact for future sessions. `memory_search` and `memory_list` accept `status`, `severity`, `priority`, `outcome`, and `phase` as filters.

## Relations

Memories can carry directed outgoing links via the `relations` field: a JSON object mapping a target memory id (string key) to a free-form relation kind:

```json
{"7": "caused_by", "12": "blocks"}
```

Links are one-way — the target memory is unmodified. Common kinds: `caused_by`, `causes`, `blocks`, `depends_on`, `supersedes`, `related_to`, `duplicate_of`. Kind strings are free-form; pick a label and use it consistently within a project. `memory_update` replaces the full set; pass `{}` to clear all links.

## References

Memories can carry an optional `refs` field — an array of free-form strings pointing to external resources related to the memory:

```json
["src/db/store.rs", "MemoryStore::insert", "https://github.com/foo/bar/issues/42"]
```

Useful kinds of entries: project-relative file paths, fully-qualified symbol names, URLs to docs or issues, ticket identifiers. Prefer short paths (`src/db/store.rs`) over absolute paths — absolute paths are just stored as-is and are less portable. `memory_update` replaces the full set; pass `[]` to clear.

Both `memory_search` and `memory_list` accept an optional `refs_contain` string that filters results to memories whose `refs` array contains the given substring in any entry.

## Graph traversal

Relations are stored in a normalized `memory_relations(from_id, to_id, kind)` table with an index on `to_id` for efficient reverse lookups. All write operations (`memory_store`, `memory_update`, `memory_delete`) keep this table in sync with the `relations` JSON column.

Five tools operate on this table: `memory_neighbors`, `memory_traverse`, `memory_path`, `memory_hubs`, and `memory_stats`. See the Tools table below for parameters and return shapes.

All five tools operate on the full global graph. `memory_stats` counts are also global: `total_memories`, `by_type`, `total_relations`, and `relation_kinds` all reflect the entire project DB.

## Tools

MCP requests and tool arguments use JSON. `memory_list` and `memory_search` return deterministic block text with literal prose; other successful structured tool content uses [TOON v3.0](https://toonformat.dev). Tool errors remain plain text. All output formats omit null and empty collection fields.

| Tool | Description |
|---|---|
| `memory_store` | Stores 1-100 memories atomically in a single embedding pass; accepts `items: [{...}]`; each item requires `content` and `memory_type` and optionally accepts `summary`, `tags`, `context`, `confidence`, `decay_rate`, `attrs`, `relations`, and `refs`; returns `{"ids":[...]}` in input order |
| `memory_search` | Two-stage semantic search (cosine + keyword re-rank), nearest-first; optional `tags`, `memory_type`, `status`, `severity`, `priority`, `outcome`, `phase`, `refs_contain` filters; returns block text with literal prose, RFC 3339 timestamps, folded relations, rounded `distance`/`effective_confidence`, and optional compact `freshness` per result |
| `memory_list` | Paginated list newest-first; optional exact `ids` (max 100), `tags`, `memory_type`, `status`, `severity`, `priority`, `outcome`, `phase`, `refs_contain` filters; returns block text with literal prose, RFC 3339 timestamps, folded relations, decay details only when active, rounded `effective_confidence`, and optional compact `freshness` |
| `memory_tags` | Returns a tag-to-count object |
| `memory_delete` | Deletes a memory by numeric id |
| `memory_update` | Updates any field in-place; re-embeds on content change; resets decay clock; pass `""` to clear `summary` or `context`; pass `{}` to clear `attrs` or `relations`; pass `[]` to clear `refs` |
| `memory_revalidate` | Resets the decay clock on a list of memory ids without changing any other field; accepts `ids: [...]`; returns count of memories updated |
| `memory_neighbors` | Returns immediate in-neighbors and out-neighbors of a memory; params: `id`, optional `kind` filter; returns uniform `{outgoing: [{id, kind, memory_type, summary}], incoming: [...]}` selection rows |
| `memory_traverse` | BFS from `start_id` up to `depth` (default 2, max 5); optional `kind` filter and `direction` (out/in/both); returns `{start_id, nodes: [{id, depth, via_kind, memory_type, summary}], edges}` |
| `memory_path` | Shortest directed path between two memories; params: `from_id`, `to_id`, optional `max_hops` (default 6); returns `{found, path: [{id, via_kind, memory_type, summary}]}`; use `memory_list(ids=[...])` to retrieve selected full memories |
| `memory_hubs` | Top-N memories by total degree (in + out); optional `top_n` (default 10); returns `[{id, summary, memory_type, out_degree, in_degree}]` |
| `memory_stats` | Graph-level statistics; returns `{total_memories, by_type: {type: count}, total_relations, relation_kinds}` |
