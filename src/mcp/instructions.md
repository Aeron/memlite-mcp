Local agent memory store. Embeddings generated server-side from plain text.

## Tools

- `memory_store`: embed + persist new memory; returns numeric id. Optional: `confidence` ([0,1], default 1.0), `decay_rate` (per-day, default 0.0), `memory_type` (`user`/`project`/`reference`/`task`)
- `memory_search`: cosine similarity + keyword re-rank; nearest-first with `effective_confidence` and `age_days`. Optional: `tags`, `memory_type` pre-filter
- `memory_list`: newest-first with pagination. Optional: `tags`, `memory_type` filter; returns raw `confidence`, `decay_rate`, `last_seen_at`
- `memory_tags`: all tags with occurrence counts, frequency-ordered
- `memory_update`: update content/summary/tags/context/confidence/decay_rate/memory_type in-place; re-embeds on content change; resets decay clock
- `memory_delete`: remove by numeric id

## Workflow

### Before investigating

Call `memory_search` before any investigation, debugging, or implementation. Use symptom, error, component, or concept as query. Stored memory may resolve immediately or surface prior decisions. Skipping risks full-cost rediscovery.

### After completing work

Before finishing, store anything that would save meaningful future effort. Don't wait to be asked. Good candidates:

- Decision made or confirmed: what was chosen, what rejected, and why
- Non-obvious fact: quirk, undocumented behavior, workaround, env-specific detail
- Bug diagnosed: root cause + fix, not just symptom
- Constraint identified: anything ruling out or shaping future approaches
- User preference or project convention: naming style, tooling choice, process rule
- Significant research: if it cost time once, store it

Plain mechanical steps don't need storing (e.g., "ran the tests"). Test: would a future session benefit without rediscovery?

### Before storing

Call `memory_tags` to reuse existing tags; inventing synonyms fragments taxonomy. Then call `memory_search`. Distance < 0.15: update instead of creating duplicate. Store only when no close match found.

### Searching vs listing

Use `memory_search` for concept/topic; semantic results regardless of age. Use `memory_list` only for chronological review (e.g., "what did I store recently?").

### Distance scores

- 0.0: identical vector
- < 0.15: very close (likely same concept)
- < 0.40: related (same domain)
- > 0.70: unrelated

### `effective_confidence` and `age_days`

`effective_confidence` = base confidence minus accumulated decay since last validation, clamped [0,1].

- >= 0.8: reliable
- 0.5-0.8: mild caution; re-verify if decision is significant
- < 0.5: potentially stale; confirm before relying on it

Low-confidence memory confirmed accurate? Call `memory_update` (any field, including setting `confidence` to current value) to reset `last_seen_at` and restart decay clock.

## Fields

### `summary`

Provide when content is longer than 2-3 sentences, technical jargon, or structured data. Shown in list views; used to decide whether to read full content. Skip for short, self-describing facts. Pass `""` to clear.

### `context`

Record the *why*: reasoning, constraints, trade-offs, broader situation. Not a summary; answers:
- "Why this approach over alternatives?"
- "What prompted this?"
- "Who or what does this affect, and how?"

Skip for plain self-contained facts. Use for decisions, architectural choices, anything where rationale will matter later. Pass `""` to clear.

### `confidence`

Float [0.0, 1.0]. Provide when info is not fully certain:
- Speculative/inferred: 0.5-0.7
- Believed correct, unverified: 0.8-0.9
- Confirmed: 1.0 (default)

### `decay_rate`

Float [0.0, 1.0]. Reduces `effective_confidence` by that amount per day since last validation:
- 0.0 (default): no decay; stable facts, decisions, conventions
- 0.005-0.01: slow; implementation details drifting over months
- 0.02-0.05: moderate; env state, dependency versions, workarounds
- 0.1+: fast; time-sensitive (e.g., "build currently broken")

### `tags`

Grouping and filtering. Lowercase, singular where natural: `"auth"` not `"Authentication"`. Examples: `["architecture", "database"]`, `["bug", "auth"]`. Skip for one-off facts with no named domain.

Both `memory_search` and `memory_list` accept optional `tags` array (all must match). Combine with query in `memory_search` to narrow to a category.

### `memory_type`

| Value | Use for |
|---|---|
| `user` | Preferences, identity, working style |
| `project` | Conventions, tech stack, architecture, constraints |
| `reference` | Domain knowledge, docs, external facts, API details |
| `task` | Things to track, follow up, or complete |

Set when memory clearly fits one category. Skip for multi-category or unclear fit. Both `memory_search` and `memory_list` accept `memory_type` as filter; combine with `tags` for precise retrieval.

## Updating vs deleting

`memory_update` resets decay clock (`last_seen_at` = now). Use as revalidation: if memory confirmed accurate during session, update it (even just set `confidence` to current value) to signal verification.

Prefer update when core identity same but details changed: decision revised, fix turned incomplete, constraint relaxed/tightened, new info extends existing. Update proactively when current work makes stored memory stale or incomplete.

Delete only when fully obsolete or factually wrong with no salvageable parts. Content update triggers re-embedding automatically.
