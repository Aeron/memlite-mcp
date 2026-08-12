# Tools Reference

This document describes the memory model and every MCP tool the server
exposes. For installation and configuration, see
[CONFIGURATION.md](CONFIGURATION.md).

MCP requests and tool arguments use JSON. `memory_list` and `memory_search`
return deterministic block text with literal prose; other successful
structured tool content uses [TOON v3.0](https://toonformat.dev). Tool errors
remain plain text. All output formats omit null and empty collection fields.

## Memory model

Each memory is a record with required `content` and `memory_type` fields and
a set of optional fields: `summary`, `tags`, `context`, `confidence`,
`decay_rate`, `attrs`, `relations`, and `refs`. The sections below describe
the fields that carry structure.

### Memory types

The required `memory_type` field classifies a memory's intended use, enabling
targeted filtering in `memory_list` and `memory_search`.

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

Feedback disambiguation: personal opinions on agent approach or working style
go to `user`; quality or correctness observations about the work itself go to
`finding` or `issue`.

### Typed attrs

`attrs` is an optional JSON object for type-specific fields.

| Field | Values | Most useful for |
|---|---|---|
| `status` | `open`, `resolved`, `wontfix`, `deferred` | task, issue, finding |
| `severity` | `critical`, `high`, `medium`, `low` | issue, finding |
| `priority` | `urgent`, `high`, `medium`, `low` | task |
| `outcome` | `accepted`, `rejected`, `superseded`, `deferred` | decision |
| `reversible` | `true`, `false` | decision |
| `phase` | `draft`, `approved`, `active`, `completed` | plan |

Do not delete resolved memories — they are useful history. Set `attrs.status`
to `resolved` instead and leave the record intact for future sessions.
`memory_search` and `memory_list` accept `status`, `severity`, `priority`,
`outcome`, and `phase` as filters.

### Trust and decay

Each memory carries a `confidence` score (base trust, 0.0–1.0) and an
optional `decay_rate` (per-day reduction, 0.0–1.0). At query time the server
computes `effective_confidence = confidence - decay_rate * days_since_last_seen`,
clamped to [0, 1]. `memory_search` results include this derived value
together with readable RFC 3339 timestamps and an optional compact
`freshness` warning.

Set `confidence < 1.0` when storing uncertain or speculative information.
Set `decay_rate > 0.0` for time-sensitive facts that naturally become stale
(e.g. `0.01` decays ~1% per day). Calling `memory_update` on any field resets
`last_seen_at` to now, restarting the decay clock. Use `memory_revalidate`
when confirming one or more memories still accurate without changing their
content.

### Relations

Memories can carry directed outgoing links via the `relations` field: a JSON
object mapping a target memory id (string key) to a free-form relation kind:

```json
{"7": "caused_by", "12": "blocks"}
```

Links are one-way; the target memory is unmodified. Every target identifier
must name an existing memory. Common kinds: `caused_by`, `causes`, `blocks`,
`depends_on`, `supersedes`, `related_to`, `duplicate_of`. Kind strings are
free-form; pick a label and use it consistently within a project.
`memory_update` replaces the full set; pass `{}` to clear all links.

### References

Memories can carry an optional `refs` field — an array of free-form strings
pointing to external resources related to the memory:

```json
["src/db/store.rs", "MemoryStore::insert", "https://github.com/foo/bar/issues/42"]
```

Useful kinds of entries: project-relative file paths, fully-qualified symbol
names, URLs to docs or issues, ticket identifiers. Prefer short paths
(`src/db/store.rs`) over absolute paths — absolute paths are just stored
as-is and are less portable. `memory_update` replaces the full set; pass `[]`
to clear.

Both `memory_search` and `memory_list` accept an optional `refs_contain`
string that filters results to memories whose `refs` array contains the
given substring in any entry.

### Knowledge graph

Relations are stored in a normalized `memory_relations(from_id, to_id, kind)`
table with an index on `to_id` for efficient reverse lookups. All write
operations (`memory_store`, `memory_update`, `memory_delete`) keep this
table in sync with the `relations` JSON column.

Five tools operate on this table: `memory_neighbors`, `memory_traverse`,
`memory_path`, `memory_hubs`, and `memory_stats`. All five operate on the
full global graph of the project DB.

## memory_store

Stores 1–100 memories atomically: all items are embedded in a single ONNX
pass and inserted in one transaction — all succeed or all fail. Returns
`{"ids": [...]}` in input order.

| Parameter | Required | Description |
|---|---|---|
| `items` | yes | Array of 1–100 memory objects |

Each item:

| Field | Required | Description |
|---|---|---|
| `content` | yes | Full text to embed and store |
| `memory_type` | yes | One of the eight [memory types](#memory-types) |
| `summary` | no | One-sentence description for quick recall; provide when content is long, complex, or not self-describing |
| `tags` | no | Category labels for grouping (e.g. `["architecture", "auth"]`) |
| `context` | no | Background rationale for this memory |
| `confidence` | no | Initial confidence score in [0, 1] (default 1.0) |
| `decay_rate` | no | Per-day confidence decay rate in [0, 1] (default 0.0) |
| `attrs` | no | Type-specific fields; see [Typed attrs](#typed-attrs) |
| `relations` | no | Outgoing directed links; see [Relations](#relations) |
| `refs` | no | External reference pointers; see [References](#references) |

## memory_search

Two-stage semantic search: cosine vector search first, word-overlap re-rank
second. Unfiltered queries use the DiskANN vector index. Filtered queries use
an exact cosine scan so filters apply before candidate selection. Results are
ordered nearest first and include a rounded `distance` and
`effective_confidence`, RFC 3339 timestamps, folded relations, and an optional
compact `freshness` warning.

Compact by default: each result carries its summary, or a bounded content
preview as fallback. Pass `include_content: true` for full context and
content; full results omit the then-redundant summary.

| Parameter | Required | Description |
|---|---|---|
| `query` | yes | Natural-language query; ranked by cosine similarity |
| `limit` | no | Maximum results (default 10, max 100) |
| `include_content` | no | Full context and content per result (default false) |
| `tags` | no | Only memories carrying all given tags (AND semantics) |
| `memory_type` | no | Only memories of the given type |
| `status`, `severity`, `priority`, `outcome`, `phase` | no | [Typed attrs](#typed-attrs) filters |
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
| `status`, `severity`, `priority`, `outcome`, `phase` | no | [Typed attrs](#typed-attrs) filters |
| `refs_contain` | no | Only memories whose `refs` contain this substring |

## memory_update

Updates any field of a memory in place. Providing `content` triggers
re-embedding. Any update resets the decay clock (`last_seen_at`).

Clearing semantics: pass `""` to clear `summary` or `context`; pass `{}` to
clear `attrs` or `relations`; pass `[]` to clear `refs`. `attrs`,
`relations`, and `refs` are full-set replacements, not merges.

| Parameter | Required | Description |
|---|---|---|
| `id` | yes | Numeric identifier of the memory to update |
| `content` | no | Replacement content; triggers re-embedding |
| `summary` | no | Replacement summary |
| `tags` | no | Replacement tags |
| `context` | no | Replacement context |
| `confidence` | no | Replacement confidence; clamped to [0, 1] |
| `decay_rate` | no | Replacement decay rate; clamped to [0, 1] |
| `memory_type` | no | Replacement type; empty string is rejected |
| `attrs` | no | Replacement attrs set |
| `relations` | no | Replacement relations set |
| `refs` | no | Replacement refs set |

## memory_delete

Deletes a memory by numeric id and returns whether a record was actually
removed. Prefer setting `attrs.status` to `resolved` over deletion for
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
| `ids` | yes | 1-100 numeric identifiers of memories to revalidate |

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
| `id` | yes | Numeric id of the memory to get neighbors for |
| `kind` | no | Relation kind filter (e.g. `blocks`); all kinds when omitted |

## memory_traverse

BFS traversal from a start memory. Returns
`{start_id, nodes: [{id, depth, via_kind, memory_type, summary}], edges}`.

| Parameter | Required | Description |
|---|---|---|
| `start_id` | yes | Numeric id of the start memory |
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
| `from_id` | yes | Numeric id of the start memory |
| `to_id` | yes | Numeric id of the destination memory |
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
