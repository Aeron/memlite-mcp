# MemLite MCP

A local-only MCP server and command-line manager for persistent, searchable
agent memory, backed by **libsql** (SQLite with native vector search) and
**fastembed** (on-device ONNX embeddings; no API keys, no cloud).

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
- **Direct CLI management**: inspect, search, update, and remove memories without an MCP client

## Build

```sh
cargo build --release
```

The embedding model is downloaded on the first `memory_store` or
`memory_search` call, not at server start, and cached for fully offline use
afterwards. See [docs/configuration.md](docs/configuration.md) for cache
locations.

## Configuration

The defaults are fine for most setups. Run the MCP server with
`memlite-mcp serve`. The one option worth a look is `--embedding-model`
(`MEMLITE_EMBEDDING_MODEL`): English-first BGE v1.5 models are available,
along with multilingual `bge-m3` for memories spanning 100+ languages.

For all flags, env vars, and the embedding models table, see
[docs/configuration.md](docs/configuration.md).

## MCP server

MCP host configurations must invoke the `serve` subcommand:

```sh
memlite-mcp serve
```

The server exposes 12 `memory_*` tools: store, search, list, update, delete,
revalidate, tags, and five graph tools (neighbors, traverse, path, hubs,
stats).

## Direct CLI

The same operations are available under the singular `tool` namespace. Tool
names omit the redundant `memory_` prefix:

```sh
memlite-mcp tool list --limit 10 --include-content false
memlite-mcp tool list 55 --format json
memlite-mcp tool search "CLI interface plan"
memlite-mcp tool update 55 --phase completed
memlite-mcp tool delete 55
```

Text output is the default. Add `--format json` for scripts. Global project,
storage, embedding, and logging flags work before or after nested commands.
Run `memlite-mcp tool <command> --help` for exact arguments.

For the full command and flag reference, see
[docs/configuration.md](docs/configuration.md). For the memory model (types,
typed attrs, trust and decay, relations, references), see
[docs/memory.md](docs/memory.md). For per-tool parameters, see
[docs/tools.md](docs/tools.md).

## Benchmarks

The output format benchmark compares byte/character size and encode latency
of TOON and block text against compact JSON for representative tool
payloads; TOON cases also assert lossless round trips:

```sh
cargo bench --bench toon_format -- --noplot
```

A reported `toon_ratio` or `block_ratio` below 1.0 means that format is
smaller than compact JSON for the case.
