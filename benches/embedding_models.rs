use std::{
    env,
    hint::black_box,
    mem::size_of,
    path::PathBuf,
    time::{Duration, Instant},
};

use criterion::{Criterion, SamplingMode, Throughput, criterion_group, criterion_main};
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};

const DEFAULT_MODEL: &str = "bge-small-en-v1.5";
const SUITES: [&str; 3] = [
    "english_semantic_paraphrases",
    "multilingual_cross_lingual",
    "long_context",
];

struct ModelSpec {
    name: &'static str,
    model: EmbeddingModel,
}

struct CorpusItem {
    name: &'static str,
    text: String,
}

struct LabeledQuery {
    suite: &'static str,
    text: &'static str,
    relevant: &'static str,
}

fn all_models() -> Vec<ModelSpec> {
    vec![
        ModelSpec {
            name: "bge-small-en-v1.5",
            model: EmbeddingModel::BGESmallENV15,
        },
        ModelSpec {
            name: "bge-base-en-v1.5",
            model: EmbeddingModel::BGEBaseENV15,
        },
        ModelSpec {
            name: "bge-large-en-v1.5",
            model: EmbeddingModel::BGELargeENV15,
        },
        ModelSpec {
            name: "bge-m3",
            model: EmbeddingModel::BGEM3,
        },
    ]
}

fn model_named(name: &str) -> Option<ModelSpec> {
    all_models().into_iter().find(|model| model.name == name)
}

fn selected_models() -> Vec<ModelSpec> {
    let value = match env::var("EMBEDDING_BENCH_MODELS") {
        Ok(value) => value,
        Err(env::VarError::NotPresent) => return vec![model_named(DEFAULT_MODEL).unwrap()],
        Err(error) => panic!("EMBEDDING_BENCH_MODELS is not valid Unicode: {error}"),
    };

    if value.trim() == "all" {
        return all_models();
    }

    let mut selected = Vec::new();
    for name in value.split(',').map(str::trim) {
        let model = model_named(name).unwrap_or_else(|| {
            panic!(
                "unsupported model '{name}'; expected bge-small-en-v1.5, bge-base-en-v1.5, bge-large-en-v1.5, bge-m3, or all"
            )
        });
        if !selected
            .iter()
            .any(|selected: &ModelSpec| selected.name == model.name)
        {
            selected.push(model);
        }
    }
    assert!(!selected.is_empty(), "EMBEDDING_BENCH_MODELS is empty");
    selected
}

fn cache_dir() -> PathBuf {
    env::var_os("EMBEDDING_BENCH_CACHE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("memlite-mcp")
        })
}

fn long_context_memory(detail: &str) -> String {
    let mut text = "Routine archival context with no incident specific details. ".repeat(120);
    text.push_str(detail);
    text
}

fn corpus() -> Vec<CorpusItem> {
    vec![
        CorpusItem {
            name: "audit-events",
            text: "Decision: account changes are written as immutable events. Current state is rebuilt from the event stream so every prior value remains inspectable.".into(),
        },
        CorpusItem {
            name: "refresh-lock",
            text: "Fix: token renewal now uses one process wide mutex. Concurrent requests wait for the first refresh instead of invalidating each other's credentials.".into(),
        },
        CorpusItem {
            name: "retry-cap",
            text: "Worker retries use exponential backoff with jitter. The delay stops growing at thirty seconds before the next delivery attempt.".into(),
        },
        CorpusItem {
            name: "wal-backup",
            text: "Run a WAL checkpoint before copying the SQLite database. Copying only the main file while writes remain in the log can produce an incomplete backup.".into(),
        },
        CorpusItem {
            name: "secrets-es",
            text: "Seguridad: los secretos de la API se rotan cada noventa días. La credencial anterior sigue válida durante una hora para permitir el despliegue gradual.".into(),
        },
        CorpusItem {
            name: "offline-fr",
            text: "Synchronisation mobile: en cas de modification hors ligne concurrente, le serveur conserve la version ayant l'horodatage le plus récent et archive l'autre.".into(),
        },
        CorpusItem {
            name: "thumbnails-ja",
            text: "画像アップロード時に三つのサムネイルを生成する。元画像は非同期処理の完了後も監査用に保存する。".into(),
        },
        CorpusItem {
            name: "rollback-en",
            text: "Deployment rollback is automatic when the checkout error rate exceeds five percent for three consecutive minutes.".into(),
        },
        CorpusItem {
            name: "billing-tail",
            text: long_context_memory("Late incident detail: invoice reconciliation failed because the tax rounding mode differed between the ledger and payment processor."),
        },
        CorpusItem {
            name: "scheduler-tail",
            text: long_context_memory("Late incident detail: the overnight scheduler skipped jobs after a daylight saving clock change created a nonexistent local time."),
        },
        CorpusItem {
            name: "encryption-tail",
            text: long_context_memory("Late policy detail: archived customer exports must use envelope encryption with a separate data key for every tenant."),
        },
        CorpusItem {
            name: "theme",
            text: "The web client stores light and dark theme preference in local storage and follows the operating system when no preference exists.".into(),
        },
        CorpusItem {
            name: "connection-pool",
            text: "The PostgreSQL pool permits twenty active connections and rejects acquisition after a two second timeout.".into(),
        },
        CorpusItem {
            name: "email-digest",
            text: "Weekly email digests are assembled every Monday and omit projects that had no activity during the previous seven days.".into(),
        },
        CorpusItem {
            name: "metrics-retention",
            text: "High resolution service metrics remain available for fourteen days before aggregation into hourly samples.".into(),
        },
        CorpusItem {
            name: "crash-symbols",
            text: "Mobile release builds upload debug symbols so crash reports can be symbolicated without shipping symbols in the application bundle.".into(),
        },
    ]
}

fn queries() -> Vec<LabeledQuery> {
    vec![
        LabeledQuery {
            suite: SUITES[0],
            text: "How can we inspect every older value after an account record changes?",
            relevant: "audit-events",
        },
        LabeledQuery {
            suite: SUITES[0],
            text: "What prevents simultaneous credential renewals from interfering with one another?",
            relevant: "refresh-lock",
        },
        LabeledQuery {
            suite: SUITES[0],
            text: "What is the longest pause between failed job delivery attempts?",
            relevant: "retry-cap",
        },
        LabeledQuery {
            suite: SUITES[0],
            text: "Which preparation makes a live SQLite copy complete?",
            relevant: "wal-backup",
        },
        LabeledQuery {
            suite: SUITES[1],
            text: "How often are API credentials replaced, and how long does the old one overlap?",
            relevant: "secrets-es",
        },
        LabeledQuery {
            suite: SUITES[1],
            text: "How does mobile sync resolve two offline edits to the same record?",
            relevant: "offline-fr",
        },
        LabeledQuery {
            suite: SUITES[1],
            text: "What derivative images are produced when a user uploads a picture?",
            relevant: "thumbnails-ja",
        },
        LabeledQuery {
            suite: SUITES[1],
            text: "¿Cuándo revierte automáticamente una publicación defectuosa?",
            relevant: "rollback-en",
        },
        LabeledQuery {
            suite: SUITES[2],
            text: "Why did ledger invoices disagree with processor totals?",
            relevant: "billing-tail",
        },
        LabeledQuery {
            suite: SUITES[2],
            text: "Why were overnight jobs missed when the clocks changed?",
            relevant: "scheduler-tail",
        },
        LabeledQuery {
            suite: SUITES[2],
            text: "How must old customer export files be encrypted per tenant?",
            relevant: "encryption-tail",
        },
    ]
}

fn cosine(left: &[f32], right: &[f32]) -> f32 {
    assert_eq!(left.len(), right.len(), "cosine vector dimensions differ");
    assert!(!left.is_empty(), "cosine vectors are empty");

    let (dot, left_norm, right_norm) = left.iter().zip(right).fold(
        (0.0_f32, 0.0_f32, 0.0_f32),
        |(dot, left_norm, right_norm), (&left, &right)| {
            (
                dot + left * right,
                left_norm + left * left,
                right_norm + right * right,
            )
        },
    );
    let denominator = left_norm.sqrt() * right_norm.sqrt();
    assert!(denominator > 0.0, "cosine vector norm is zero");
    let score = dot / denominator;
    assert!(score.is_finite(), "cosine score is not finite");
    score
}

fn verify_embeddings(embeddings: &[Vec<f32>], expected: usize, dimensions: usize) {
    assert_eq!(embeddings.len(), expected, "embedding count differs");
    for embedding in embeddings {
        assert_eq!(embedding.len(), dimensions, "embedding dimensions differ");
    }
}

fn report_quality(
    model_name: &str,
    corpus: &[CorpusItem],
    corpus_embeddings: &[Vec<f32>],
    queries: &[LabeledQuery],
    query_embeddings: &[Vec<f32>],
) {
    // Ranking every corpus item with raw cosine scores
    for suite in SUITES {
        let mut count = 0_usize;
        let mut recall_at_1 = 0.0_f64;
        let mut recall_at_3 = 0.0_f64;
        let mut reciprocal_rank = 0.0_f64;
        let mut margin = 0.0_f64;

        for (query, query_embedding) in queries.iter().zip(query_embeddings) {
            if query.suite != suite {
                continue;
            }

            let relevant = corpus
                .iter()
                .position(|item| item.name == query.relevant)
                .expect("query relevant memory is missing");
            let scores: Vec<f32> = corpus_embeddings
                .iter()
                .map(|memory_embedding| cosine(query_embedding, memory_embedding))
                .collect();
            let mut ranking: Vec<usize> = (0..scores.len()).collect();
            ranking.sort_unstable_by(|&left, &right| scores[right].total_cmp(&scores[left]));
            let rank = ranking
                .iter()
                .position(|&index| index == relevant)
                .expect("relevant memory is missing from ranking")
                + 1;
            let best_negative = scores
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != relevant)
                .map(|(_, &score)| score)
                .max_by(f32::total_cmp)
                .expect("quality corpus needs a negative memory");

            count += 1;
            recall_at_1 += f64::from(rank <= 1);
            recall_at_3 += f64::from(rank <= 3);
            reciprocal_rank += 1.0 / rank as f64;
            margin += f64::from(scores[relevant] - best_negative);
        }

        assert!(count > 0, "quality suite has no queries");
        let count = count as f64;
        println!(
            "embedding_quality model={model_name} suite={suite} recall_at_1={:.3} recall_at_3={:.3} mrr={:.3} mean_relevant_best_negative_cosine_margin={:.4}",
            recall_at_1 / count,
            recall_at_3 / count,
            reciprocal_rank / count,
            margin / count,
        );
    }
}

fn benchmark_steady_state(
    criterion: &mut Criterion,
    model_name: &str,
    model: &mut TextEmbedding,
    single_query: &[&str],
    batch_documents: &[&str],
) {
    {
        let mut group = criterion.benchmark_group(format!("embedding_single_query/{model_name}"));
        group.sample_size(10);
        group.sampling_mode(SamplingMode::Flat);
        group.warm_up_time(Duration::from_secs(2));
        group.measurement_time(Duration::from_secs(10));
        group.throughput(Throughput::Elements(1));
        group.bench_function("raw_query", |bencher| {
            bencher.iter(|| {
                black_box(
                    model
                        .embed(black_box(single_query), None)
                        .expect("single query embedding failed"),
                )
            })
        });
        group.finish();
    }

    {
        let mut group =
            criterion.benchmark_group(format!("embedding_batch_32_documents/{model_name}"));
        group.sample_size(10);
        group.sampling_mode(SamplingMode::Flat);
        group.warm_up_time(Duration::from_secs(2));
        group.measurement_time(Duration::from_secs(10));
        group.throughput(Throughput::Elements(32));
        group.bench_function("raw_memories", |bencher| {
            bencher.iter(|| {
                black_box(
                    model
                        .embed(black_box(batch_documents), None)
                        .expect("batch document embedding failed"),
                )
            })
        });
        group.finish();
    }
}

fn milliseconds(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn bench_embedding_models(criterion: &mut Criterion) {
    let cache_dir = cache_dir();
    let corpus = corpus();
    let queries = queries();
    let corpus_texts: Vec<&str> = corpus.iter().map(|item| item.text.as_str()).collect();
    let query_texts: Vec<&str> = queries.iter().map(|query| query.text).collect();
    let batch_documents: Vec<&str> = (0..32)
        .map(|index| corpus[index % 8].text.as_str())
        .collect();
    let single_query = [queries[0].text];

    // Measuring models sequentially to avoid retaining every ONNX session at once
    for spec in selected_models() {
        let info = TextEmbedding::get_model_info(&spec.model).expect("model metadata missing");
        let dimensions = info.dim;
        let bytes_per_vector = dimensions * size_of::<f32>();
        let mib_per_100k_vectors = bytes_per_vector as f64 * 100_000.0 / (1024.0 * 1024.0);

        let init_started = Instant::now();
        let options = TextInitOptions::new(spec.model)
            .with_cache_dir(cache_dir.clone())
            .with_show_download_progress(false);
        let mut model = TextEmbedding::try_new(options).expect("embedding model init failed");
        let cached_init_duration = init_started.elapsed();

        // Embedding quality inputs first so Criterion measures a warmed runtime
        let corpus_started = Instant::now();
        let corpus_embeddings = model
            .embed(corpus_texts.as_slice(), None)
            .expect("initial corpus embedding failed");
        let corpus_duration = corpus_started.elapsed();

        let query_started = Instant::now();
        let query_embeddings = model
            .embed(query_texts.as_slice(), None)
            .expect("initial query embedding failed");
        let query_duration = query_started.elapsed();

        verify_embeddings(&corpus_embeddings, corpus.len(), dimensions);
        verify_embeddings(&query_embeddings, queries.len(), dimensions);

        println!(
            "embedding_model model={} dimensions={} raw_bytes_per_vector={} raw_mib_per_100k_vectors={:.2}",
            spec.name, dimensions, bytes_per_vector, mib_per_100k_vectors,
        );
        println!(
            "embedding_timing model={} cached_init_duration_ms={:.3} initial_corpus_embedding_ms={:.3} initial_query_embedding_ms={:.3} note=first_uncached_run_may_include_download cache_dir={}",
            spec.name,
            milliseconds(cached_init_duration),
            milliseconds(corpus_duration),
            milliseconds(query_duration),
            cache_dir.display(),
        );
        report_quality(
            spec.name,
            &corpus,
            &corpus_embeddings,
            &queries,
            &query_embeddings,
        );
        benchmark_steady_state(
            criterion,
            spec.name,
            &mut model,
            &single_query,
            &batch_documents,
        );
    }
}

criterion_group!(benches, bench_embedding_models);
criterion_main!(benches);
