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

- **Local-only, no API keys**: one SQLite file per project, on-device ONNX embeddings; nothing leaves the machine
- **Semantic search**: DiskANN vector retrieval plus keyword re-rank; exact cosine scan when filters are present
- **Knowledge graph**: directed relations between memories, with traversal, shortest path, neighbor, hub, and statistics tools
- **Confidence and decay**: per-memory trust score with optional time decay and revalidation
- **Structured records**: eight memory types, typed status/severity/priority attrs, tags, and external references — all filterable
- **Batch store**: up to 100 memories in a single embedding pass; atomic insert
- **Compact output**: readable block text for prose-heavy results, [TOON](https://toonformat.dev) for the rest
- **Per-project isolation**: one DB per project path; all agents on the same project share the same DB

## Build

```sh
cargo build --release
```

The embedding model is downloaded on the first `memory_store` or
`memory_search` call, not at server start, and cached for fully offline use
afterwards. See [docs/CONFIGURATION.md](docs/CONFIGURATION.md) for cache
locations.

## Configuration

The defaults are fine for most setups; no flags are required. The one option
worth a look is `--embedding-model` (`MEMLITE_EMBEDDING_MODEL`): the bundled
models are English-first BGE variants, which matters if your memories will
not be in English.

For all flags, env vars, and the embedding models table, see
[docs/CONFIGURATION.md](docs/CONFIGURATION.md).

## Tools

The server exposes 12 `memory_*` tools: store, search, list, update, delete,
revalidate, tags, and five graph tools (neighbors, traverse, path, hubs,
stats).

For the memory model (types, typed attrs, trust and decay, relations,
references) and per-tool parameters, see [docs/TOOLS.md](docs/TOOLS.md).

## Benchmarks

The output format benchmark compares byte/character size and encode latency
of TOON and block text against compact JSON for representative tool
payloads; TOON cases also assert lossless round trips:

```sh
cargo bench --bench toon_format -- --noplot
```

A reported `toon_ratio` or `block_ratio` below 1.0 means that format is
smaller than compact JSON for the case.
