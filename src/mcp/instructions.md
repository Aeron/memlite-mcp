Local agent memory store. Server-side embeddings from plain text.

## Tools

- `memory_store`: embed + persist; returns id. Optional: `confidence` ([0,1], default 1.0), `decay_rate` (per-day, default 0.0), `memory_type` (`user`/`project`/`reference`/`task`/`finding`/`issue`/`decision`/`plan`), `status` (`open`/`resolved`/`wontfix`/`deferred`), `severity` (`critical`/`high`/`medium`/`low`), `relations` (JSON object `{"<id>": "<kind>"}`, e.g. `{"7": "caused_by"}`)
- `memory_store_batch`: batch store; `items: [...]` (max 100), same fields as `memory_store`; single-pass embed, atomic insert; returns `[{"id": ...}]` in input order. Prefer over repeated `memory_store` for 3+ memories
- `memory_search`: cosine + keyword re-rank; nearest-first with `effective_confidence` and `age_days`. Pre-filter: `tags`, `memory_type`, `status`, `severity`
- `memory_list`: newest-first with pagination. Filter: `tags`, `memory_type`, `status`, `severity`; returns raw `confidence`, `decay_rate`, `last_seen_at`
- `memory_tags`: all tags with occurrence counts, frequency-ordered
- `memory_update`: update content/summary/tags/context/confidence/decay_rate/memory_type/status/severity/relations in-place; re-embeds on content change; resets decay clock; pass `""` to clear summary/context/memory_type/status/severity; pass `{}` to clear relations
- `memory_delete`: remove by id
- `memory_revalidate`: resets decay clock (`last_seen_at` = now) on a list of ids without changing any other field; returns count updated. Use when confirming one or more memories accurate in same session
- `memory_neighbors`: returns immediate in-neighbors and out-neighbors of a memory by id; optional `kind` filter restricts to one edge type; returns `{outgoing: [{id, kind, memory?}], incoming: [...]}`
- `memory_traverse`: BFS from a memory node; `depth` (default 2, max 5), optional `kind` filter, `direction` ("out" default / "in" / "both"); returns `{start_id, nodes: [{id, depth, via_kind, ...}], edges: [{from_id, to_id, kind}]}`
- `memory_path`: shortest directed path between two memories; `from_id`, `to_id`, optional `max_hops` (default 6); returns `{found, path: [{id, via_kind, ...}]}`
- `memory_hubs`: top-N most-connected memories by total degree; optional `top_n` (default 10); returns `[{id, summary, memory_type, out_degree, in_degree, total_degree}]`
- `memory_stats`: graph-level statistics; returns `{total_memories, by_type, total_relations, relation_kinds}`

## Workflow

### Before investigating

Call `memory_search` before any investigation, debugging, or implementation. Query: symptom, error, component, or concept. May resolve immediately or surface prior decisions. Skipping risks full-cost rediscovery.

### After completing work

Before finishing, store anything saving meaningful future effort. Don't wait to be asked. Good candidates:

- Decision: what chosen, what rejected, why
- Non-obvious fact: quirk, undocumented behavior, workaround, env detail
- Bug diagnosed: root cause + fix, not just symptom
- Constraint: anything ruling out or shaping future approaches
- User preference or project convention: naming, tooling, process
- Significant research: if it cost time once, store it

Plain mechanical steps don't need storing (e.g., "ran tests"). Test: would future session benefit without rediscovery?

### Before storing

Call `memory_tags` to reuse existing tags; inventing synonyms fragments taxonomy. Then `memory_search`. Distance < 0.15: update, not create. Store only when no close match found.

### Searching vs listing

`memory_search` for concept/topic; semantic, age-agnostic. `memory_list` only for chronological review (e.g., "what stored recently?").

### Graph navigation

Use graph tools when you already have memory ids and want to explore connections, not discover them from scratch:

- `memory_neighbors`: immediate links from/to a known node; use to see what a memory causes, blocks, or is caused by
- `memory_traverse`: cluster exploration from a seed node; use when a concept fans out into related memories
- `memory_path`: how two memories relate; use when tracing a chain of causes or decisions
- `memory_hubs`: most-connected memories; run at session start on an unfamiliar project for quick orientation
- `memory_stats`: graph shape overview; total counts, type distribution, relation kinds present

### How `memory_search` ranks

Two stages: cosine selects candidate pool (~4x return count), keyword overlap re-ranks within it. Pre-filters (`tags`, `memory_type`, `status`, `severity`) exclude before both stages. Result missing: drop filters first; still absent, rephrase with different vocabulary (semantic = embedding-based, not keyword-based).

### Distance scores

- 0.0: identical vector
- < 0.15: very close (likely same concept)
- < 0.40: related (same domain)
- > 0.70: unrelated

### `effective_confidence` and `age_days`

`effective_confidence` = base confidence - accumulated decay since last validation, clamped [0,1].

- >= 0.8: reliable
- 0.5-0.8: mild caution; re-verify if decision significant
- < 0.5: potentially stale; confirm before use

Confirmed accurate? Call `memory_revalidate([id])`. Multiple: pass all ids in one call. If also changing a field, `memory_update` resets clock too.

## Fields

### `summary`

Provide when content > 2-3 sentences, uses jargon, or is structured data. Shown in list views; helps decide whether to read full content. Skip for short, self-describing facts. Pass `""` to clear.

### `context`

Record the *why*: reasoning, constraints, trade-offs, broader situation. Not a summary; answers:
- "Why this approach over alternatives?"
- "What prompted this?"
- "Who or what does this affect, and how?"

Skip for plain self-contained facts. Use for decisions, architectural choices, anything where rationale matters later. Pass `""` to clear.

### `confidence`

Float [0.0, 1.0]. Provide when info uncertain:
- Speculative/inferred: 0.5-0.7
- Believed correct, unverified: 0.8-0.9
- Confirmed: 1.0 (default)

### `decay_rate`

Float [0.0, 1.0]. Reduces `effective_confidence` by amount per day since last validation:
- 0.0 (default): no decay; stable facts, decisions, conventions
- 0.005-0.01: slow; implementation details drifting over months
- 0.02-0.05: moderate; env state, dependency versions, workarounds
- 0.1+: fast; time-sensitive (e.g., "build currently broken")

### `tags`

Grouping and filtering. Lowercase, singular where natural: `"auth"` not `"Authentication"`. Examples: `["architecture", "database"]`, `["bug", "auth"]`. Skip for one-off facts with no named domain.

Both `memory_search` and `memory_list` accept optional `tags`. All must match (hard AND pre-filter; not a relevance boost). Fewer tags = broader pool; result missing, drop tags first. Combine with query to narrow domain.

### `memory_type`

| Value | Use for |
|---|---|
| `user` | Preferences, identity, working style |
| `project` | Conventions, tech stack, architecture, constraints |
| `reference` | Domain knowledge, docs, external facts, API details |
| `task` | Things to track, follow up, or complete |
| `finding` | Discovered problem, audit result, or identified risk; not yet tracked task |
| `issue` | Known defect or bug |
| `decision` | Architectural or design decision record: what was chosen, what rejected, and why |
| `plan` | High-level plan or phased roadmap; use `relations` to link to spawned tasks and decisions; apply moderate `decay_rate` (0.01-0.02) |

Set when memory fits one category; skip if multi-category or unclear. Pass `""` to `memory_update` to clear. Both tools accept `memory_type` as filter; combine with `tags` and `status` for precise retrieval.

### `status`

Lifecycle state for task and issue memories. Omit for general knowledge or anything without discrete resolution.

| Value | Use for |
|---|---|
| `open` | Active and unresolved |
| `resolved` | Completed or fixed |
| `wontfix` | Acknowledged, will not be addressed |
| `deferred` | Postponed for later |

Set when storing bug, task, or finding. Update to `resolved` when done; don't delete resolved memories (useful history). Pass `""` to `memory_update` to clear. Both tools accept `status` as filter; combine with `memory_type` to query e.g. all open tasks.

Note: `decay_rate` is wrong model for "is this still open?"; bug is fixed or not. Use `status` for lifecycle, `decay_rate` for time-based staleness.

### `severity`

Urgency/impact level for finding, issue, or task. Omit for general knowledge or anything without meaningful priority.

| Value | Meaning |
|---|---|
| `critical` | Requires immediate attention; may block work or cause data loss |
| `high` | Significant impact; should be addressed soon |
| `medium` | Noticeable impact; can be scheduled |
| `low` | Minor impact; address when convenient |

Set when storing finding, issue, or task where priority matters. Pass `""` to `memory_update` to clear. Both tools accept `severity` as filter; combine with `status` to query e.g. all open critical issues.

### `relations`

Outgoing directed links to other memory identifiers. JSON object mapping target id (string key) to free-form relation kind:

```json
{"7": "caused_by", "12": "blocks"}
```

Kind is any non-empty string. Suggested labels for consistency:

| Kind | Meaning |
|---|---|
| `caused_by` | caused by target (finding -> root cause) |
| `causes` | causes target (root cause -> downstream effect) |
| `blocks` | blocks target task or fix |
| `depends_on` | depends on target |
| `supersedes` | replaces target (new decision -> old decision) |
| `related_to` | generic association |
| `duplicate_of` | duplicate of target |

Links are one-way: memory A having `{"7": "caused_by"}` adds no link to memory 7. Reverse traversal: use `memory_neighbors` (returns both directions) or store an explicit back-link.

`memory_update` replaces full set when `relations` provided. Pass `{}` to clear all outgoing links.

Returned in both tools as JSON object (empty when no links stored).

## Updating vs deleting

Revalidating without field changes: `memory_revalidate` (single or list of ids). `memory_update` also resets clock when bundling a field change.

Prefer update when core identity same but details changed: decision revised, fix turned incomplete, constraint shifted, new info extends. Update proactively when current work makes memory stale.

Delete only when fully obsolete or factually wrong with nothing salvageable. Content update triggers re-embedding automatically.
