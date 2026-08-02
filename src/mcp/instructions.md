Local agent memory store. Server-side embeddings from plain text.

## Tools

- `memory_store`: atomically embed + persist `items: [...]` (1-100) in one pass; each item requires `content`, `memory_type` (`user`/`project`/`reference`/`task`/`finding`/`issue`/`decision`/`plan`). Optional: `summary` (one-line recall aid), `tags`, `context` (rationale), `confidence` ([0,1], default 1.0), `decay_rate` (per-day, default 0.0), `attrs` (JSON object with type-specific fields; see attrs section), `relations` (JSON object `{"<id>": "<kind>"}`, e.g. `{"7": "caused_by"}`), `refs` (array of strings: file paths, symbol names, URLs). Returns `{"ids":[...]}` in input order
- `memory_search`: cosine + keyword re-rank; nearest-first. Compact block text by default: summary, else bounded one-line content preview. Pass `include_content: true` for full context/content; full results omit redundant summary. Includes RFC 3339 timestamps, folded `target_id:kind` relations, rounded `effective_confidence`/`distance`, optional compact `freshness`; null/empty fields omitted. `limit` default 10, max 100. Pre-filter: `tags`, `memory_type`, `status`, `severity`, `priority`, `outcome`, `phase`, `refs_contain`
- `memory_list`: newest-first block text; same readable fields as search. `include_content` default true; full results omit redundant summary. Pass `false` for compact selection output. Pagination: `limit` (default 20, max 100), `offset`. Filter: `ids` (exact, max 100) plus same fields as `memory_search`; null/empty fields omitted; decay details only when active
- `memory_tags`: tag-to-count object
- `memory_update`: update content/summary/tags/context/confidence/decay_rate/memory_type/attrs/relations/refs in-place; re-embeds on content change; resets decay clock; pass `""` to clear summary/context; pass `{}` to clear attrs or relations; pass `[]` to clear refs
- `memory_delete`: remove by id
- `memory_revalidate`: resets decay clock (`last_seen_at` = now) on a list of ids without changing any other field; returns count updated. Use when confirming one or more memories accurate in same session
- `memory_neighbors`: immediate in/out neighbors; optional `kind` filter; returns uniform `{id, kind, memory_type, summary}` selection rows
- `memory_traverse`: BFS from `start_id`; `depth` (default 2, max 5), optional `kind`, `direction`; returns uniform `{id, depth, via_kind, memory_type, summary}` node rows plus edges
- `memory_path`: shortest directed path; `from_id`, `to_id`, optional `max_hops` (default 6); returns uniform `{id, via_kind, memory_type, summary}` selection rows
- `memory_hubs`: top-N by total degree; optional `top_n` (default 10)
- `memory_stats`: graph counts; type distribution, relation kinds

List/search outputs use literal block text; other structured outputs use TOON v3.0. Inputs remain JSON. Errors remain plain text.

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

`memory_search` for concept/topic; semantic, age-agnostic, compact by default. Select identifiers from results, then fetch full records with `memory_list(ids=[...])`. `memory_list` without `ids` for chronological review (e.g., "what stored recently?"); pass `include_content: false` when only selection signals needed.

### Graph navigation

Use when you have ids and want to explore connections, not discover from scratch:

- `memory_neighbors`: immediate links from/to a node
- `memory_traverse`: BFS cluster from a seed node
- `memory_path`: how two nodes connect; trace cause/decision chains
- `memory_hubs`: most-connected nodes; good orientation on unfamiliar project
- `memory_stats`: graph shape overview

Graph rows carry identifiers and one-line selection signals, not full memories. Fetch chosen records with `memory_list(ids=[...])`.

### How `memory_search` ranks

Two stages: cosine selects candidate pool (~4x return count), keyword overlap re-ranks within it. Pre-filters (`tags`, `memory_type`, `refs_contain`, and all attrs fields) exclude before both stages; attrs filters match via `json_extract`. Result missing: drop filters first; still absent, rephrase with different vocabulary (semantic = embedding-based, not keyword-based).

### Distance scores

- 0.0: identical vector
- < 0.15: very close (likely same concept)
- < 0.40: related (same domain)
- 0.40-0.70: weak or tangential
- > 0.70: unrelated

### `effective_confidence` and freshness

`effective_confidence` = base confidence - accumulated decay since last validation, clamped [0,1]. List/search return readable timestamps instead of raw Unix seconds; optional `freshness` summarizes meaningful decay or staleness.

- >= 0.8: reliable
- 0.5-0.8: mild caution; re-verify if decision significant
- < 0.5: potentially stale; confirm before use

Confirmed accurate? Call `memory_revalidate([id])`. Multiple: pass all ids in one call. If also changing a field, `memory_update` resets clock too.

## Fields

### `summary`

Provide when content > 2-3 sentences, uses jargon, or is structured data. Shown in compact search/list output; helps decide whether to fetch full content. Compact output falls back to bounded one-line content preview when summary absent. Full output omits summary; content already present. Skip for short, self-describing facts. Pass `""` to clear.

### `context`

Record the *why*: reasoning, constraints, trade-offs, broader situation. Not a summary. Use for decisions, architectural choices, anything where rationale matters later. Skip for plain self-contained facts. Pass `""` to clear.

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
| `user` | Preferences, identity, working style; personal opinions on work quality or agent approach |
| `project` | Conventions, tech stack, architecture, constraints |
| `reference` | Domain knowledge, docs, external facts, API details |
| `task` | Things to track, follow up, or complete |
| `finding` | Observation, evaluation, or risk; covers audit results, review findings, quality assessments, and retrospective notes; not yet a tracked task |
| `issue` | Known defect or bug |
| `decision` | Architectural or design decision record: what was chosen, what rejected, and why |
| `plan` | High-level plan or phased roadmap; use `relations` to link to spawned tasks and decisions; apply moderate `decay_rate` (0.01-0.02) |

Required for every stored memory. Pick closest category; use `project` for general codebase facts when unclear. Both tools accept `memory_type` as filter; combine with `tags`, `status`, and `severity` (attrs key filters) for precise retrieval.

Feedback disambiguation: personal opinions on agent approach or working style -> `user`; quality or correctness observations about the work itself -> `finding` or `issue`.

### `attrs`

Typed extra fields stored as a JSON object. All fields are optional; omit to exclude. Pass `{}` to clear all. Available fields and valid values:

| Field | Type | Use for |
|---|---|---|
| `status` | `open`/`resolved`/`wontfix`/`deferred` | task, issue, finding |
| `severity` | `critical`/`high`/`medium`/`low` | issue, finding |
| `priority` | `urgent`/`high`/`medium`/`low` | task |
| `outcome` | `accepted`/`rejected`/`superseded`/`deferred` | decision |
| `reversible` | `true`/`false` | decision |
| `phase` | `draft`/`approved`/`active`/`completed` | plan |

Don't delete resolved memories; set `attrs: {"status": "resolved"}` instead. Use `status` filter in search/list to narrow to e.g. open issues. Combine `status` + `severity` filters to find open critical issues.

### `refs`

External reference pointers. Free-form array of strings: file paths, symbol names, URLs, or issue IDs. Examples: `["src/db/store.rs", "MemoryStore::insert"]`, `["https://github.com/foo/issues/42"]`. Use short project-relative paths (`src/db/store.rs`) not absolute paths. `memory_update` replaces the full set; pass `[]` to clear. `memory_search` and `memory_list` accept `refs_contain` for substring filtering.

### `relations`

Outgoing directed links: JSON object `{"<target_id>": "<kind>"}`. Kind is free-form; common: `caused_by`, `causes`, `blocks`, `depends_on`, `supersedes`, `related_to`, `duplicate_of`. Links one-way; reverse traversal via `memory_neighbors`. `memory_update` replaces full set; pass `{}` to clear.

## Updating vs deleting

Revalidating without field changes: `memory_revalidate` (single or list of ids). `memory_update` also resets clock when bundling a field change.

Prefer update when core identity same but details changed: decision revised, fix turned incomplete, constraint shifted, new info extends. Update proactively when current work makes memory stale.

Delete only when fully obsolete or factually wrong with nothing salvageable. Content update triggers re-embedding automatically.
