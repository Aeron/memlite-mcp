# MemLite MCP

A local-only MCP server and command-line manager for persistent, searchable agent
memory. It uses [libSQL](https://github.com/tursodatabase/libsql) for SQLite storage
with native vector search and [FastEmbed](https://github.com/Anush008/fastembed-rs)
to generate ONNX embeddings on the device.

## Motivation

AI agents are stateless by design. Without persistent memory, an agent in a new
conversation has no record of what was figured out last session, what decisions were
made, or which approaches had already failed. For quick, self-contained tasks that is
fine. For an evolving codebase, an ongoing investigation, or a project with accumulated
conventions, the repetition wastes time. The agent keeps rediscovering the same things,
and you keep re-explaining them.

A pretty pivotal problem, but is it a solved one yet? Well... there are some
attempts for sure:

- Markdown files. Those work as far as a text-file store goes: no extra attributes,
  no vector search, no memory graph or dependencies out of the box. It's the bare
  minimum. Can we do better?
- Cloud services. Somewhat featureful, but they require API keys and context
  leaving the machine. Plus, a dependency on a third-party endpoint and an internet
  connection. It feels like pricey overkill. Can we do local instead?
- Self-hosted solutions. A bit better, but still bulky. I don't think we need to
  manage a bunch of containers here. Can we do without a zoo?
- Anything simpler? Kinda. But the ones I found either require third-party runtimes
  (Node/Bun, Python) and package managers (npm/npx, uv/uvx), or separate
  storage/search services (ChromaDB, Milvus, Qdrant, MongoDB, SurrealDB, Neo4j).
  Why though? Can a single binary do it all?
- Fully self-contained binaries. I found very few, and none ticked all my boxes.

I wanted a tiny and performant local-only MCP solution. Nothing theoretical.
Something that would significantly improve my agentic workflow right after the
first draft build. I like Rust and libSQL; with native support for vector search
and indexes, libSQL was a perfect fit and an obvious choice. Everything else was
born naturally from my day-to-day personal and professional workflows. So, voila!

## Features

- **Local-only**: SQLite storage and on-device ONNX embeddings
- **Semantic search**: DiskANN retrieval plus keyword reranking; exact cosine scan with metadata filters
- **Knowledge graph**: directed relations, neighbors, traversal, shortest paths, hubs, and statistics
- **Confidence and decay**: per-memory trust scores, optional time decay, and revalidation
- **Structured records**: eight memory types, typed attributes, tags, references, and metadata filters
- **Batch storage**: up to 100 memories per MCP request; batched embeddings and atomic inserts
- **Compact output**: readable search/list results, [TOON](https://toonformat.dev) for other structured results, optional CLI JSON
- **Per-project isolation**: one database per project path, shared by agents on that project
- **Direct CLI management**: all 12 MCP tools without an MCP client
- **Shared worker election**: one worker per database directory; automatic proxy takeover on exit
- **Resource sharing**: lazy per-project database handles and shared embedding models for matching settings
- **Four embedding models**: English-first BGE v1.5 variants and multilingual `bge-m3`

## Build and install

The build requires Rust 1.92 or newer and a native build toolchain. MemLite uses Unix
sockets and file locks for its shared worker.

Clone the repository, then build or install from the checkout:

```sh
git clone https://github.com/Aeron/memlite-mcp.git
cd memlite-mcp
cargo build --release --locked
```

The build produces `target/release/memlite`. To install it into Cargo's
binary directory instead:

```sh
cargo install --path . --locked
```

On Apple Silicon macOS, the bundled ONNX Runtime objects target macOS 13.4. Set
`MACOSX_DEPLOYMENT_TARGET` to `13.4` or newer for either command so Rust and native
dependencies use the same minimum version:

```sh
MACOSX_DEPLOYMENT_TARGET=13.4 cargo build --release --locked
```

Cargo's binary directory must be on your `PATH` to run the installed command.

## Development

With [just](https://just.systems/) installed, run `just` to list available commands.
The recipes default `MACOSX_DEPLOYMENT_TARGET` to `13.4` while preserving an existing
environment override.

```sh
just check
just release
just install
just run --help
just test -- --test-threads=1
just bench --bench toon_format -- --noplot
```

`just check` checks formatting, runs Clippy on all targets, and runs tests. Build,
install, lint, test, and benchmark recipes forward extra arguments to Cargo;
`just run` forwards them to MemLite.

## Use

Configure your MCP client to run `memlite serve`. MemLite uses the first
usable local MCP root, then the server's working directory. Run direct CLI
commands from the project directory to use its memories.

The first embedding operation downloads the model, then uses a local cache for
offline use. Memory storage and inference stay on your machine. Your MCP client
still controls what it sends to its language-model provider.

```sh
memlite serve
memlite tool list --limit 10 --include-content false
memlite tool --format json search "project conventions" --limit 5
```

See [client setup](docs/clients.md) for Claude Code, Codex, Pi, OpenCode, Kilo,
Hermes, and Zed. See [configuration and CLI reference](docs/configuration.md)
for all arguments, environment variables, model choices, and direct commands.

## Documentation

- [Memory model](docs/memory.md)
- [MCP tools](docs/tools.md)
- [Benchmarks and measured results](docs/benchmarks.md)

## Acknowledgments

- **[RMCP](https://github.com/modelcontextprotocol/rust-sdk)** by [Model Context Protocol](https://github.com/modelcontextprotocol): the official Rust MCP SDK
- **[libSQL](https://github.com/tursodatabase/libsql)** by [Turso](https://github.com/tursodatabase): SQLite storage with native vector search
- **[FastEmbed](https://github.com/Anush008/fastembed-rs)** by [Anush](https://github.com/Anush008) and contributors: local ONNX embedding inference
- **[BGE models](https://huggingface.co/BAAI)** by [BAAI](https://www.baai.ac.cn/en): the embedding models behind semantic search
