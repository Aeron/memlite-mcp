# Tools reference

The server exposes the MCP tools below. See [memory.md](memory.md) for memory
types, typed attributes, trust and decay, relations, and references. See
[configuration.md](configuration.md) for the full CLI reference and
configuration, or the [README](../README.md#build-and-install) for installation.

MCP requests and tool arguments use JSON. `memory_list` and `memory_search`
return deterministic block text with literal prose. Other successful
structured tool content uses [TOON v3.0](https://toonformat.dev). Tools return
errors as plain text. All output formats omit null and empty collection fields.

Memory types, typed-attribute filters, and traversal directions use enum
schemas. MCP rejects invalid enum values with `invalid_params` errors before
running the tool. The CLI rejects them during argument parsing. Relation
kinds remain free-form.

MCP tool operations may pass through an elected shared worker. Proxying does
not change tool schemas or output. A request that overlaps worker failover
can return one transport error. The worker never replays in-flight requests
because a write may already have committed. Check whether a write took
effect before retrying it.

## Direct CLI

Every tool below is also available as `memlite-mcp tool <name>`. CLI names
omit the `memory_` prefix. The store command differs: `tool store` stores one
memory per invocation, while MCP `memory_store` accepts 1 to 100 atomic
inserts. See [configuration.md](configuration.md) for the full command,
option, and flag reference.

## memory_store

The server embeds 1 to 100 memories in one batch request and inserts them
in one transaction. Inference can split that request into smaller batches
according to the [model batch cap](configuration.md#embedding-models).
All inserts succeed or all fail. The tool returns `{"ids": [...]}` in input
order.

| Parameter | Required | Description |
|---|---|---|
| `items` | yes | Provide an array of 1 to 100 memory objects. |

Each item accepts the following fields.

| Field | Required | Description |
|---|---|---|
| `content` | yes | Provide the full text to embed and store. |
| `memory_type` | yes | Choose one of the eight [memory types](memory.md#memory-types). |
| `summary` | yes | Describe the memory in one sentence for quick recall. The server rejects blank values. |
| `tags` | no | Group memories with category labels, such as `["architecture", "auth"]`. |
| `context` | no | Explain the rationale for this memory. |
| `confidence` | no | Set the initial confidence score in [0, 1]. The default is 1.0. |
| `decay_rate` | no | Set the per-day confidence decay rate in [0, 1]. The default is 0.0. |
| `attrs` | no | Provide type-specific fields. See [typed attrs](memory.md#typed-attrs). |
| `relations` | no | Provide outgoing directed links. See [relations](memory.md#relations). |
| `refs` | no | Provide external references. See [references](memory.md#references). |

## memory_search

Search first selects candidates by cosine similarity, then reorders them by
whole-word overlap between the query and memory content. Word overlap
determines the final order; cosine distance breaks ties. Unfiltered queries
use the DiskANN vector index. Filtered queries use an exact cosine scan so
filters apply before candidate selection.

Lower `distance` values indicate closer cosine matches, but results need not
follow distance order after word-overlap ranking. Search does not apply a
distance cutoff. Results include rounded `distance` and
`effective_confidence` values, RFC 3339 timestamps, folded relations, and an
optional compact `freshness` warning.

Each result includes its required summary by default. Pass
`include_content: true` for full context and content. Full results omit the
summary.

| Parameter | Required | Description |
|---|---|---|
| `query` | yes | Provide a natural-language query. |
| `limit` | no | Return at most this many results. The default is 10; the maximum is 100. |
| `include_content` | no | Include full context and content in each result. The default is false. |
| `tags` | no | Return only memories that carry every given tag. |
| `memory_type` | no | Return only memories of this type. |
| `status`, `severity`, `priority`, `outcome`, `phase` | no | Filter by [typed attrs](memory.md#typed-attrs). |
| `refs_contain` | no | Return only memories with a `refs` entry that contains this literal substring. |

## memory_list

The tool lists memories newest first and supports pagination. It returns
block text with RFC 3339 timestamps, folded relations, decay details only
when active, rounded `effective_confidence`, and an optional compact
`freshness` warning.

`include_content` defaults to true. Full output omits the summary. Pass false
for compact selection output, then fetch selected full memories with
`memory_list(ids=[...])`.

| Parameter | Required | Description |
|---|---|---|
| `limit` | no | Return at most this many results per page. The default is 20; the maximum is 100. |
| `offset` | no | Set the zero-based pagination offset. The default is 0. |
| `ids` | no | Provide up to 100 exact numeric memory identifiers. The tool omits missing identifiers and returns results newest first, not in input order. |
| `include_content` | no | Include full context and content in each result. The default is true. |
| `tags` | no | Return only memories that carry every given tag. |
| `memory_type` | no | Return only memories of this type. |
| `status`, `severity`, `priority`, `outcome`, `phase` | no | Filter by [typed attrs](memory.md#typed-attrs). |
| `refs_contain` | no | Return only memories with a `refs` entry that contains this literal substring. |

## memory_update

The tool updates any field of a memory in place. Providing `content` triggers
re-embedding. Any update resets the decay clock by setting `last_seen_at` to now.

Pass `""` to clear `context`, `{}` to clear `attrs` or `relations`, and `[]`
to clear `refs`. The server rejects blank summaries. Updates replace the
full `attrs`, `relations`, and `refs` sets rather than merging them.

| Parameter | Required | Description |
|---|---|---|
| `id` | yes | Provide the numeric identifier of the memory to update. |
| `content` | no | Replace the content and trigger re-embedding. |
| `summary` | no | Replace the summary. The server rejects blank values. |
| `tags` | no | Replace the tags. |
| `context` | no | Replace the context. |
| `confidence` | no | Replace the confidence score. The server clamps it to [0, 1]. |
| `decay_rate` | no | Replace the decay rate. The server clamps it to [0, 1]. |
| `memory_type` | no | Replace the type. The server rejects an empty string. |
| `attrs` | no | Replace the full set of typed attributes. |
| `relations` | no | Replace the full set of outgoing relations. |
| `refs` | no | Replace the full set of references. |

## memory_delete

The tool deletes a memory by numeric identifier. It returns
`{deleted: true, id}` when it removes the memory or `{deleted: false, id}`
when the memory does not exist. Set `attrs.status` to `resolved` instead of
deleting memories that provide useful history.

| Parameter | Required | Description |
|---|---|---|
| `id` | yes | Provide the numeric identifier of the memory to delete. |

## memory_revalidate

The tool resets the decay clock on a list of memories by setting
`last_seen_at = now`. It changes no other field and returns the count of
memories it updated. Use it after confirming that multiple memories are
still accurate in the same session.

| Parameter | Required | Description |
|---|---|---|
| `ids` | yes | Provide 1 to 100 numeric identifiers of memories to revalidate. |

## memory_tags

The tool returns all tags as a compact tag-to-count object and takes no
parameters. Call it before `memory_store` to reuse existing tags and keep
tag names consistent.

## memory_neighbors

The tool returns a memory's immediate incoming and outgoing neighbors as
compact selection rows: `{outgoing: [{id, kind, memory_type, summary}], incoming:
[...]}`. Fetch selected full memories with `memory_list(ids=[...])`.

| Parameter | Required | Description |
|---|---|---|
| `id` | yes | Provide the numeric identifier of the memory to inspect. |
| `kind` | no | Filter by a relation kind, such as `blocks`. Omit it to include all kinds. |

## memory_traverse

The tool traverses the graph breadth first from a start memory and returns
`{start_id, nodes: [{id, depth, via_kind, memory_type, summary}], edges}`.

| Parameter | Required | Description |
|---|---|---|
| `start_id` | yes | Provide the numeric identifier of the start memory. |
| `depth` | no | Limit the breadth-first traversal depth. The default is 2; the maximum is 5. |
| `kind` | no | Traverse only edges with this exact kind. |
| `direction` | no | Choose `out`, `in`, or `both`. The default is `out`. |

## memory_path

The tool finds the shortest directed path between two memories by following
outgoing relations. It returns
`{found, path: [{id, via_kind, memory_type, summary}]}`. The path starts at
the source. Each step carries the relation kind used to reach it.

| Parameter | Required | Description |
|---|---|---|
| `from_id` | yes | Provide the numeric identifier of the start memory. |
| `to_id` | yes | Provide the numeric identifier of the destination memory. |
| `max_hops` | no | Limit how many hops to explore. The default is 6; the maximum is 10. |

## memory_hubs

The tool returns the most connected memories as
`[{id, summary, memory_type, out_degree, in_degree}]`. It adds the incoming
and outgoing relation counts to get each memory's total degree, then orders
memories by total degree from highest to lowest. Use these memories to
orient yourself at the start of a session.

| Parameter | Required | Description |
|---|---|---|
| `top_n` | no | Set how many top hubs to return. The default is 10; the maximum is 50. |

## memory_stats

The tool returns graph-level statistics as
`{total_memories, by_type: {type: count}, total_relations, relation_kinds}`.
It takes no parameters. All counts are global and cover the entire project
database. Use them to check the size and structure of the memory graph.
