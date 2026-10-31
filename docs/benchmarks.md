# Benchmarks

MemLite has three Criterion suites for embedding models, MCP tool round trips,
and result encoding. Run commands from the repository root with the selected
models already cached.

Reference results below come from a sequential run on November 3, 2026, on an
Apple M1 Max with 10 cores and 64 GiB RAM, macOS 27.2 arm64, and Rust 1.99.1.
All four models were cached before the run. Timings are Criterion's central
estimates from one run, not fixed performance guarantees.

## Embedding models

Source: [embedding_models.rs](../benches/embedding_models.rs). The suite compares
local FastEmbed inference across the four supported models.

### Running it

The default runs `bge-small-en-v1.5`:

```sh
cargo bench --bench embedding_models -- --noplot
```

Select models or compare all four:

```sh
EMBEDDING_BENCH_MODELS=bge-small-en-v1.5,bge-base-en-v1.5 cargo bench --bench embedding_models -- --noplot
EMBEDDING_BENCH_MODELS=all cargo bench --bench embedding_models -- --noplot
```

`EMBEDDING_BENCH_CACHE_DIR` overrides the default OS cache directory under
`memlite-mcp`. The suite loads one model at a time.

### Workload and metrics

The quality corpus has 16 synthetic memories and 11 labeled queries. Four queries
test English paraphrases, four test cross-lingual retrieval, and three test details
placed after repeated unrelated context. Each query has one relevant memory.

Recall@1 and recall@3 report how often that memory appears in the first one or
three results. MRR is its mean reciprocal rank. The cosine margin is the mean
relevant-memory score minus the strongest non-relevant score. Quality scores use
raw cosine ranking, without MemLite's database retrieval or keyword rerank.

Criterion measures one short query and a batch of 32 short memories, with ten
samples, a two-second warmup, and a ten-second measurement target. Model loading
and initial quality embeddings happen before timing. The suite calls FastEmbed
with `embed(inputs, None)`. Production caps batches by model, so M3's batch-32
result does not represent its production batches of eight.

All models use FastEmbed's default 512-token limit. The long-context fixture tests
that limit, not M3's advertised 8192-token window.

### Results

#### Latency and throughput

| Model | Single query | Queries/s | Batch of 32 | Memories/s |
|---|---:|---:|---:|---:|
| `bge-small-en-v1.5` | 7.1193 ms | 140.46 | 152.80 ms | 209.42 |
| `bge-base-en-v1.5` | 13.824 ms | 72.336 | 497.08 ms | 64.376 |
| `bge-large-en-v1.5` | 38.047 ms | 26.284 | 1.5951 s | 20.062 |
| `bge-m3` | 36.779 ms | 27.189 | 1.2770 s | 25.059 |

#### Raw vector footprint

Raw `f32` payload only, without database or vector-index overhead.

| Model | Dimensions | Bytes/vector | MiB/100,000 vectors |
|---|---:|---:|---:|
| `bge-small-en-v1.5` | 384 | 1536 | 146.48 |
| `bge-base-en-v1.5` | 768 | 3072 | 292.97 |
| `bge-large-en-v1.5` | 1024 | 4096 | 390.62 |
| `bge-m3` | 1024 | 4096 | 390.62 |

#### English semantic paraphrases

| Model | Recall@1 | Recall@3 | MRR | Cosine margin |
|---|---:|---:|---:|---:|
| `bge-small-en-v1.5` | 1.000 | 1.000 | 1.000 | 0.1384 |
| `bge-base-en-v1.5` | 1.000 | 1.000 | 1.000 | 0.1519 |
| `bge-large-en-v1.5` | 1.000 | 1.000 | 1.000 | 0.1498 |
| `bge-m3` | 1.000 | 1.000 | 1.000 | 0.1365 |

#### Multilingual cross-lingual retrieval

| Model | Recall@1 | Recall@3 | MRR | Cosine margin |
|---|---:|---:|---:|---:|
| `bge-small-en-v1.5` | 0.250 | 0.500 | 0.424 | -0.0606 |
| `bge-base-en-v1.5` | 0.500 | 0.750 | 0.615 | -0.0401 |
| `bge-large-en-v1.5` | 0.500 | 0.750 | 0.661 | -0.0311 |
| `bge-m3` | 1.000 | 1.000 | 1.000 | 0.0977 |

#### Long-context retrieval

| Model | Recall@1 | Recall@3 | MRR | Cosine margin |
|---|---:|---:|---:|---:|
| `bge-small-en-v1.5` | 0.000 | 0.000 | 0.080 | -0.1536 |
| `bge-base-en-v1.5` | 0.000 | 0.333 | 0.200 | -0.0604 |
| `bge-large-en-v1.5` | 0.000 | 0.000 | 0.111 | -0.0913 |
| `bge-m3` | 0.000 | 0.000 | 0.091 | -0.1014 |

Small was fastest on this workload. M3 retrieved all four multilingual targets at
rank one. No model retrieved a long-context target at rank one. Check models
against representative memories before changing the production default.

## MCP tool round trips

Source: [mcp_tools.rs](../benches/mcp_tools.rs). Each case uses a server subprocess,
stdio JSON-RPC, and a temporary file-backed database. Startup, the MCP handshake,
and corpus seeding happen before timing.

### Running it

The full suite includes search corpora of 100, 1,000, 10,000, and 100,000 memories:

```sh
cargo bench --bench mcp_tools -- --noplot
```

The reference run skips the 100,000-memory case:

```sh
cargo bench --bench mcp_tools -- '^(write/warm|search/n/(100|1000|10000)|list/n100/(full|compact)|tags/n100)$' --noplot
```

The spawned server uses `MEMLITE_EMBEDDING_MODEL`, `MEMLITE_EMBEDDING_CACHE_DIR`,
and `MEMLITE_EMBEDDING_MAX_BATCH_SIZE`. These results use the default Small model.

### Results

| Case | Round-trip latency |
|---|---:|
| `write/warm` | 13.236 ms |
| `search/n/100` | 6.4574 ms |
| `search/n/1000` | 7.2160 ms |
| `search/n/10000` | 7.1155 ms |
| `list/n100/full` | 191.92 us |
| `list/n100/compact` | 181.12 us |
| `tags/n100` | 116.11 us |

Writes store one memory per call. Searches return up to ten results. Lists return
20 rows from 100 memories, with or without full content. Tags aggregate over 100
memories. The seeded corpus repeats eight short texts. These are latency results,
not retrieval-quality scores. The 100,000-memory case remains unmeasured.

## Result encoding

Source: [toon_format.rs](../benches/toon_format.rs). The suite compares compact
JSON and TOON encoding of seven fixed payloads, plus block text for mixed-prose
search results. Payload construction happens before timing.

### Running it

```sh
cargo bench --bench toon_format -- --noplot
```

### Results

Encoding times below use microseconds, written as `us`.

| Payload | JSON bytes | TOON bytes | TOON/JSON size | JSON encoding, us | TOON encoding, us |
|---|---:|---:|---:|---:|---:|
| `store_100` | 301 | 301 | 1.000 | 0.65339 | 3.5363 |
| `search_20_mixed_prose` | 11963 | 12418 | 1.038 | 9.4207 | 38.424 |
| `hubs_20_uniform` | 1915 | 804 | 0.420 | 2.0742 | 17.369 |
| `neighbors_20_uniform` | 2668 | 1926 | 0.722 | 2.2280 | 15.152 |
| `traverse_20_uniform` | 3802 | 2309 | 0.607 | 3.5484 | 26.554 |
| `path_10_uniform` | 1364 | 957 | 0.702 | 1.1918 | 7.7395 |
| `tags_20` | 262 | 242 | 0.924 | 0.40064 | 2.3292 |

Mixed-prose block output was 10898 bytes and took 16.920 us to encode. TOON was
smaller than JSON for uniform graph payloads but larger for mixed prose. JSON
encoding was faster in every case.
