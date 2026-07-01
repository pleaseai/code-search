//! CodSpeed benchmarks for the `csp` hot paths.
//!
//! Covers the pure, allocation-heavy functions on the indexing and ranking
//! critical path: identifier tokenization (run over every indexed file) and the
//! ranking penalty / top-k reranking pipeline (run on every search query).
//!
//! Uses the `codspeed-criterion-compat` shim (aliased as `criterion`), so the
//! same file runs under vanilla Criterion locally and under CodSpeed's
//! instrumentation in CI.

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use csp::ranking::penalties::{file_path_penalty, rerank_top_k};
use csp::ranking::Scores;
use csp::tokens::{split_identifier, tokenize};
use csp::types::Chunk;

/// A representative slice of source-like text with mixed camelCase, snake_case,
/// PascalCase and plain identifiers — the kind of content that flows through
/// the indexing tokenizer.
const SAMPLE_SOURCE: &str = r#"
pub fn buildHandlerStack(config: &AppConfig, retryCount: u32) -> HandlerStack {
    let http_client = HttpClient::new(config.base_url, config.max_retries);
    let requestQueue = RequestQueue::with_capacity(DEFAULT_QUEUE_SIZE);
    for parseXMLResponse in getHTTPResponses(http_client, requestQueue) {
        let user_id = parseXMLResponse.userId;
        registerCallbackHandler(user_id, my_func, abc123Def);
    }
}
"#;

/// A batch of individual identifiers exercising each splitting branch.
const IDENTIFIERS: &[&str] = &[
    "HandlerStack",
    "getHTTPResponse",
    "XMLParser",
    "my_func",
    "simple",
    "abc123Def",
    "registerCallbackHandler",
    "DEFAULT_QUEUE_SIZE",
];

/// File paths spanning penalised (tests, examples, type stubs, barrels) and
/// ordinary code, matching the `file_path_penalty` branches.
const PATHS: &[&str] = &[
    "src/core/handler.ts",
    "src/core/handler.test.ts",
    "tests/test_indexing.py",
    "examples/demo.ts",
    "pkg/service_test.go",
    "src/types.d.ts",
    "src/__init__.py",
    "compat/legacy_shim.ts",
    "src/ranking/penalties.rs",
    "src/indexing/index.rs",
];

fn bench_tokenize(c: &mut Criterion) {
    c.bench_function("tokenize/source_block", |b| {
        b.iter(|| tokenize(black_box(SAMPLE_SOURCE)))
    });
}

fn bench_split_identifier(c: &mut Criterion) {
    c.bench_function("split_identifier/mixed_batch", |b| {
        b.iter(|| {
            for ident in black_box(IDENTIFIERS) {
                black_box(split_identifier(black_box(ident)));
            }
        })
    });
}

fn bench_file_path_penalty(c: &mut Criterion) {
    c.bench_function("file_path_penalty/mixed_paths", |b| {
        b.iter(|| {
            for path in black_box(PATHS) {
                black_box(file_path_penalty(black_box(path)));
            }
        })
    });
}

fn make_chunks(n: usize) -> Vec<Chunk> {
    (0..n)
        .map(|i| Chunk {
            content: format!("fn item_{i}() {{ /* body */ }}"),
            file_path: PATHS[i % PATHS.len()].to_string(),
            start_line: i as u32,
            end_line: (i + 5) as u32,
            language: None,
        })
        .collect()
}

fn make_scores(n: usize) -> Scores {
    // Deterministic pseudo-random-ish scores in (0, 1].
    (0..n)
        .map(|i| (i, ((i * 2654435761) % 997) as f64 / 997.0 + 0.001))
        .collect()
}

fn bench_rerank_top_k(c: &mut Criterion) {
    let chunks = make_chunks(500);
    let scores = make_scores(500);

    let mut group = c.benchmark_group("rerank_top_k");
    group.bench_function("500_candidates_top10_penalised", |b| {
        b.iter(|| {
            black_box(rerank_top_k(
                black_box(&scores),
                black_box(&chunks),
                10,
                true,
            ))
        })
    });
    group.bench_function("500_candidates_top10_no_penalty", |b| {
        b.iter(|| {
            black_box(rerank_top_k(
                black_box(&scores),
                black_box(&chunks),
                10,
                false,
            ))
        })
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_tokenize,
    bench_split_identifier,
    bench_file_path_penalty,
    bench_rerank_top_k,
);
criterion_main!(benches);
