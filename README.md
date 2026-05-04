# memlite-mcp

A local-only MCP (Model Context Protocol) server that gives AI agents persistent,
searchable memory backed by **libsql** (SQLite + native vector search) and
**fastembed** (local ONNX embeddings — no API keys, no cloud calls).

## Features

- **Semantic search** via cosine similarity using `vector_distance_cos`
- **Local-only**: all embeddings generated on-device with `fastembed`
- **Per-project isolation**: each project gets its own SQLite file
- **Per-agent filtering**: memories are tagged with the agent identifier
- **Stdio transport**: standard MCP wire protocol, works with any MCP client

## Tools

| Tool | Description |
|---|---|
| `memory_store` | Embed and persist a memory; returns its numeric `id` |
| `memory_search` | Semantic search over stored memories (cosine distance) |
| `memory_list` | Paginated list of memories, newest first |
| `memory_delete` | Delete a memory by its `id` |
| `memory_update` | Update content (re-embeds), summary, or tags |

## Build

```sh
cargo build --release
```

The first run downloads the default embedding model (~67 MB) and caches it in
`~/.cache/huggingface`. Subsequent runs are fully offline.

## Configuration

All options are available as CLI flags **and** env vars (Clap resolves both; flags take precedence).

| Flag | Env var | Default | Description |
|---|---|---|---|
| `--project-path` | `MEMLITE_PROJECT_PATH` | current working directory | Project identifier; agents sharing the same path share a DB |
| `--agent` | `MEMLITE_AGENT` | `""` | Filters memories to this agent (empty = see all) |
| `--db-dir` | `MEMLITE_DB_DIR` | `~/.local/share/memlite-mcp` | Directory for per-project DB files |
| `--in-memory` | `MEMLITE_IN_MEMORY` | `false` | `true`/`1`/`yes` for ephemeral in-memory DB |
| `--embedding-model` | `MEMLITE_EMBEDDING_MODEL` | `bge-small-en-v1.5` | Options: `bge-small-en-v1.5`, `bge-base-en-v1.5`, `bge-large-en-v1.5` |
| — | `RUST_LOG` | `info` | Log level (written to stderr) |

Run `memlite-mcp --help` for the full usage listing.

## MCP Client Configuration

### Claude Desktop (`~/Library/Application Support/Claude/claude_desktop_config.json`)

```json
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/memlite-mcp",
      "env": {
        "MEMLITE_PROJECT_PATH": "/my/project",
        "MEMLITE_AGENT": "claude"
      }
    }
  }
}
```

### Zed (`settings.json`)

```json
{
  "context_servers": {
    "memory": {
      "command": {
        "path": "/path/to/memlite-mcp",
        "env": {
          "MEMLITE_PROJECT_PATH": "/my/project",
          "MEMLITE_AGENT": "claude"
        }
      }
    }
  }
}
```

## Schema

```sql
CREATE TABLE memories (
    content    TEXT    NOT NULL,
    summary    TEXT,
    tags       TEXT    NOT NULL DEFAULT '[]',
    agent      TEXT    NOT NULL DEFAULT '',
    embedding  BLOB,                           -- vector32 via libsql
    created_at INTEGER NOT NULL,               -- Unix seconds
    updated_at INTEGER NOT NULL
);
```

Memories are identified by their SQLite `rowid` (exposed as `id` in all tool responses).
