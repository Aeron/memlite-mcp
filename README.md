# MemLite MCP

A local-only MCP server and command-line manager for persistent, searchable
agent memory. It uses [libSQL](https://github.com/tursodatabase/libsql) for
SQLite storage with native vector search and
[FastEmbed](https://github.com/Anush008/fastembed-rs) to generate ONNX
embeddings on the device.

## Motivation

AI agents are stateless by design. Without persistent memory, an agent in a new
conversation has no recollection of what was figured out last session, what
decisions were made, or which approaches had already failed. For quick,
self-contained tasks that is fine. For an evolving codebase, an ongoing
investigation, or a project with accumulated conventions, the repetition wastes
time. The agent keeps rediscovering the same things, and you keep re-explaining
them.

Thus, the memory problem. A pretty essential one, but is it solved? Well… there
are some attempts for sure:

- Markdown files. Those work as far as a text-file store goes: no meta
  attributes, no vector search, no memory relations out of the box, no reranking
  and decay, etc. It’s the barest minimum. Can we do better?
- Cloud services. Somewhat featureful, but they require authentication and
  context leaving the machine. Plus, an extra dependency on a third-party
  endpoint and an internet connection. It feels like pricey overkill. Can we do
  local instead?
- Self-hosted variants. A bit better, but still bulky. I don’t think we need to
  manage a bunch of containers here. Can we do without a zoo?
- Local-first tools. Usually, those either require third-party runtimes
  (Node/Bun, Python) and package managers (npm/npx, uv/uvx), or separate
  storage/search solutions (ChromaDB, Milvus, Qdrant, MongoDB, SurrealDB,
  Neo4j). But why though? Can a single binary do it all?
- Self-contained binaries. They exist, but none ticked all my hypothetical
  boxes. Why rely on Markdown files still? Why a full-text search instead of a
  vector one?
- Harness-specific plugins and wrappers. Also exist and have good candidates,
  yet we’re tying ourselves to a single tool; suboptimal. Can we simply stick to
  MCP?

All I wanted is a tiny and performant harness-agnostic local MCP solution.
Nothing theoretical. Something that would solve the problem and significantly
improve my agentic workflow right after the first draft build. And not a whole
knowledge graph super monstrosity. Just something that does one thing and does
it very well.

What can we do about it? For local storage approaches, I like libSQL and it
already has native support for vector search/indexes. It also suits the idea of
having a single, portable SQLite database file per project, which is nice. Looks
like a perfect fit and an obvious choice. And Rust as a backend, of course.
Plus, fast on-CPU text embedding generation with suitable models already exist.
Everything else was born naturally from my day-to-day personal and professional
workflows.

So, voilà! I hope you’ll find it useful and enjoyable for your developer
experience.

## Features

- **Local-only**: SQLite storage and on-device ONNX embeddings
- **Semantic search**: DiskANN retrieval plus keyword reranking; exact cosine
  scan with metadata filters
- **Knowledge graph**: directed relations, neighbors, traversal, shortest paths,
  hubs, and statistics
- **Confidence and decay**: per-memory trust scores, optional time decay, and
  revalidation
- **Structured records**: eight memory types, typed attributes, tags,
  references, and metadata filters
- **Batch storage**: up to 100 memories per MCP request; batched embeddings and
  atomic inserts
- **Compact output**: readable search/list results,
  [TOON](https://toonformat.dev) for other structured results, optional CLI JSON
- **Per-project isolation**: one database per project path, shared by agents on
  that project
- **Direct CLI management**: all 12 MCP tools without an MCP client
- **Shared worker election**: one worker per database directory; automatic proxy
  takeover on exit
- **Resource sharing**: lazy per-project database handles and shared embedding
  models for matching settings
- **Four embedding models**: English-first BGE v1.5 variants and multilingual
  `bge-m3`

## Usage

MemLite runs as a standalone `memlite` binary. Download it from the
[releases page](https://github.com/Aeron/memlite-mcp/releases) or install with
Homebrew.

For Cargo installation, source builds, and development commands, see the
[building guide](docs/building.md).

Install with Homebrew:

```sh
brew install aeron/tap/memlite-mcp
```

Configure your MCP client to run `memlite serve`. No special `AGENTS.md`
instructions are required. You can optionally add `Use MemLite MCP for memory.`

See [client setup](docs/clients.md) for Claude Code, Codex, Pi, OpenCode, Kilo,
Hermes, and Zed.

You can also manage memories directly from the CLI:

```sh
memlite tool list --limit 10 --include-content false
memlite tool --format json search "project conventions" --limit 5
```

The [CLI reference](docs/cli.md) covers commands, flags, and
environment variables. See [configuration](docs/configuration.md) for storage,
permissions, project paths, shared workers, and embedding models.

### Other documentation

- [Memory model](docs/memory.md)
- [MCP tools](docs/tools.md)
- [Benchmarks and measured results](docs/benchmarks.md)

## Contributing

You’re welcome to fork MemLite, report bugs, suggest improvements, and submit
pull requests. Please be kind and respectful of everyone’s time. Maintainers
and authors don’t owe you a response, support, a fix, or acceptance of your
contribution. Participation is an invitation to collaborate, not an obligation
for anyone else.

AI-assisted contributions are welcome, but you remain responsible for what you
submit. Read and follow the [AI usage policy](AI_POLICY.md) before contributing.

## Acknowledgments

- **[RMCP](https://github.com/modelcontextprotocol/rust-sdk)** by [Model Context
  Protocol](https://github.com/modelcontextprotocol): the official Rust MCP SDK
- **[libSQL](https://github.com/tursodatabase/libsql)** by
  [Turso](https://github.com/tursodatabase): SQLite storage with native vector
  search
- **[FastEmbed](https://github.com/Anush008/fastembed-rs)** by
  [Anush](https://github.com/Anush008) and contributors: local ONNX embedding
  inference
- **[BGE models](https://huggingface.co/BAAI)** by
  [BAAI](https://www.baai.ac.cn/en): the embedding models behind semantic search
