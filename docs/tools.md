# Tools Reference

This document describes every MCP tool the server exposes. For the shape of
a memory record (types, typed attrs, trust and decay, relations,
references), see [memory.md](memory.md). For installation, the
full CLI shape, and configuration, see [configuration.md](configuration.md).

MCP requests and tool arguments use JSON. `memory_list` and `memory_search`
return deterministic block text with literal prose; other successful
structured tool content uses [TOON v3.0](https://toonformat.dev). Tool errors
remain plain text. All output formats omit null and empty collection fields.

MCP tool operations may pass through elected shared worker. Proxying is
transparent to tool schemas and output. A request overlapping worker failover can
return one transport error; retry it because in-flight writes are never replayed.

## Direct CLI

Every tool below is also available as `memlite-mcp tool <name>` (names omit
the `memory_` prefix), with one exception: `tool store` stores a single
memory per invocation, while MCP `memory_store` keeps the batch interface for
1 to 100 atomic inserts. See [configuration.md](configuration.md) for the
full command, option, and flag reference.

## memory_store

Stores 1–100 memories atomically: all items are embedded in a single ONNX
pass and inserted in one transaction (all succeed or all fail). Returns
`{"ids": [...]}` in input order.

| Parameter | Required | Description |
|---|---|---|
| `items` | yes | Array of 1–100 memory objects |

Each item:

| Field | Required | Description |
|---|---|---|
| `content` | yes | Full text to embed and store |
| `memory_type` | yes | One of the eight [memory types](memory.md#memory-types) |
| `summary` | yes | One-sentence description for quick recall; blank values are rejected |
| `tags` | no | Category labels for grouping (e.g. `["architecture", "auth"]`) |
| `context` | no | Background rationale for this memory |
| `confidence` | no | Initial confidence score in [0, 1] (default 1.0) |
| `decay_rate` | no | Per-day confidence decay rate in [0, 1] (default 0.0) |
| `attrs` | no | Type-specific fields; see [typed attrs](memory.md#typed-attrs) |
| `relations` | no | Outgoing directed links; see [relations](memory.md#relations) |
| `refs` | no | External reference pointers; see [references](memory.md#references) |

## memory_search

Two-stage semantic search: cosine vector search first, word-overlap re-rank
second. Unfiltered queries use the DiskANN vector index. Filtered queries use
an exact cosine scan so filters apply before candidate selection. Results are
ordered nearest first and include a rounded `distance` and
`effective_confidence`, RFC 3339 timestamps, folded relations, and an optional
compact `freshness` warning.

Compact by default: each result carries its required summary. Pass
`include_content: true` for full context and content; full results omit the
then-redundant summary.

| Parameter | Required | Description |
|---|---|---|
| `query` | yes | Natural-language query; ranked by cosine similarity |
| `limit` | no | Maximum results (default 10, max 100) |
| `include_content` | no | Full context and content per result (default false) |
| `tags` | no | Only memories carrying all given tags (AND semantics) |
| `memory_type` | no | Only memories of the given type |
| `status`, `severity`, `priority`, `outcome`, `phase` | no | [Typed attrs](memory.md#typed-attrs) filters |
| `refs_contain` | no | Only memories whose `refs` contain this substring |

## memory_list

Paginated list, newest first. Returns block text with RFC 3339 timestamps,
folded relations, decay details only when active, rounded
`effective_confidence`, and an optional compact `freshness` warning.

`include_content` defaults to true; full output omits the redundant summary.
Pass false for compact selection output, then fetch selected full memories
with `memory_list(ids=[...])`.

| Parameter | Required | Description |
|---|---|---|
| `limit` | no | Maximum results per page (default 20, max 100) |
| `offset` | no | Zero-based pagination offset (default 0) |
| `ids` | no | Exact numeric identifiers to fetch (max 100); missing identifiers are omitted; results stay ordered newest first, not in input order |
| `include_content` | no | Full context and content per result (default true) |
| `tags` | no | Only memories carrying all given tags (AND semantics) |
| `memory_type` | no | Only memories of the given type |
| `status`, `severity`, `priority`, `outcome`, `phase` | no | [Typed attrs](memory.md#typed-attrs) filters |
| `refs_contain` | no | Only memories whose `refs` contain this substring |

## memory_update

Updates any field of a memory in place. Providing `content` triggers
re-embedding. Any update resets the decay clock (`last_seen_at`).

Clearing semantics: pass `""` to clear `context`; pass `{}` to clear `attrs`
or `relations`; pass `[]` to clear `refs`. Blank summaries are rejected.
`attrs`, `relations`, and `refs` are full-set replacements, not merges.

| Parameter | Required | Description |
|---|---|---|
| `id` | yes | Numeric identifier of the memory to update |
| `content` | no | Replacement content; triggers re-embedding |
| `summary` | no | Replacement summary; blank values are rejected |
| `tags` | no | Replacement tags |
| `context` | no | Replacement context |
| `confidence` | no | Replacement confidence; clamped to [0, 1] |
| `decay_rate` | no | Replacement decay rate; clamped to [0, 1] |
| `memory_type` | no | Replacement type; empty string is rejected |
| `attrs` | no | Replacement attrs set |
| `relations` | no | Replacement relations set |
| `refs` | no | Replacement refs set |

## memory_delete

Deletes a memory by numeric identifier and returns whether a record was
actually removed. Prefer setting `attrs.status` to `resolved` over deletion for
anything with history value.

| Parameter | Required | Description |
|---|---|---|
| `id` | yes | Numeric identifier of the memory to delete |

## memory_revalidate

Resets the decay clock (`last_seen_at = now`) on a list of memories without
changing any other field. Returns the count of memories actually updated.
Use after confirming multiple memories are still accurate in the same
session.

| Parameter | Required | Description |
|---|---|---|
| `ids` | yes | 1–100 numeric identifiers of memories to revalidate |

## memory_tags

Returns all tags as a compact tag-to-count object. Takes no parameters.
Useful before `memory_store` to reuse existing tags and keep the taxonomy
consistent.

## memory_neighbors

Returns immediate in-neighbors and out-neighbors of a memory as compact
selection rows: `{outgoing: [{id, kind, memory_type, summary}], incoming:
[...]}`. Fetch selected full memories with `memory_list(ids=[...])`.

| Parameter | Required | Description |
|---|---|---|
| `id` | yes | Numeric identifier of the memory to get neighbors for |
| `kind` | no | Relation kind filter (e.g. `blocks`); all kinds when omitted |

## memory_traverse

BFS traversal from a start memory. Returns
`{start_id, nodes: [{id, depth, via_kind, memory_type, summary}], edges}`.

| Parameter | Required | Description |
|---|---|---|
| `start_id` | yes | Numeric identifier of the start memory |
| `depth` | no | BFS depth limit (default 2, max 5) |
| `kind` | no | Only traverse edges with this exact kind |
| `direction` | no | `out` (default), `in`, or `both` |

## memory_path

Finds the shortest directed path between two memories via outgoing
relations. Returns `{found, path: [{id, via_kind, memory_type, summary}]}`;
the path starts at the source and each step carries the relation kind used
to arrive.

| Parameter | Required | Description |
|---|---|---|
| `from_id` | yes | Numeric identifier of the start memory |
| `to_id` | yes | Numeric identifier of the destination memory |
| `max_hops` | no | Maximum hops to explore (default 6, max 10) |

## memory_hubs

Returns the top-N memories by total relation degree (in + out) as
`[{id, summary, memory_type, out_degree, in_degree}]`, ordered by total
degree descending. These are the hub memories the rest of the graph revolves
around; useful for orientation at the start of a session.

| Parameter | Required | Description |
|---|---|---|
| `top_n` | no | Number of top hubs to return (default 10, max 50) |

## memory_stats

Returns graph-level statistics:
`{total_memories, by_type: {type: count}, total_relations, relation_kinds}`.
Takes no parameters. All counts are global and reflect the entire project
DB. Good starting point for understanding the shape of the memory graph.
