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

## Tools

| Tool | Description |
|---|---|
| `memory_store` | Embeds and persists a memory; returns its numeric id. Accepts optional `summary`, `tags`, and `context` |
| `memory_search` | Semantic search by cosine similarity, ordered nearest-first; supports tag filtering |
| `memory_list` | Paginated list newest-first; supports tag filtering |
| `memory_tags` | Lists all distinct tags with occurrence counts, ordered by frequency |
| `memory_delete` | Deletes a memory by numeric id |
| `memory_update` | Updates content (re-embeds), summary, tags, or context; pass `""` to clear optional fields |
