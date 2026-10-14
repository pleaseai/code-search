//! CodSpeed benchmarks for the `csp` hot paths.
//!
//! Covers the pure, allocation-heavy functions on the indexing and ranking
//! critical path: identifier tokenization (run over every indexed file) and the
//! ranking penalty / top-k reranking pipeline (run on every search query), plus
//! a cold index build comparing serial and parallel per-file work.
//!
//! Uses the `codspeed-criterion-compat` shim (aliased as `criterion`), so the
//! same file runs under vanilla Criterion locally and under CodSpeed's
//! instrumentation in CI.

use std::path::Path;

use criterion::{black_box, criterion_group, criterion_main, Criterion};

use csp::indexing::create::{create_index_from_path, CreateIndexOptions};
use csp::indexing::dense::make_stub_model;
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
    // Deterministic pseudo-random-ish scores in (0.001, ~1.0].
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

/// Write a generated ~300-file polyglot tree (TypeScript, Python, Rust, Go),
/// each file a handful of functions so chunking and tokenizing dominate.
fn write_index_tree(root: &Path) {
    for i in 0..300 {
        let (dir, ext) = [("web", "ts"), ("svc", "py"), ("core", "rs"), ("cmd", "go")][i % 4];
        let body: String = (0..12)
            .map(|f| match ext {
                "ts" => format!(
                    "export function handleRequest{i}_{f}(userId: string, retryCount: number) {{\n  const requestQueue = buildQueue(userId)\n  return requestQueue.map((item) => item.value + retryCount)\n}}\n\n"
                ),
                "py" => format!(
                    "def compute_value_{i}_{f}(user_id, retry_count):\n    request_queue = build_queue(user_id)\n    return [item.value + retry_count for item in request_queue]\n\n\n"
                ),
                "rs" => format!(
                    "pub fn parse_item_{i}_{f}(user_id: u32, retry_count: u32) -> Vec<u32> {{\n    let request_queue = build_queue(user_id);\n    request_queue.iter().map(|item| item + retry_count).collect()\n}}\n\n"
                ),
                _ => format!(
                    "func RenderView{i}_{f}(userID int, retryCount int) []int {{\n\tqueue := buildQueue(userID)\n\tfor i := range queue {{\n\t\tqueue[i] += retryCount\n\t}}\n\treturn queue\n}}\n\n"
                ),
            })
            .collect();
        let header = if ext == "go" { "package main\n\n" } else { "" };
        let path = root.join(dir).join(format!("mod_{i}.{ext}"));
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, format!("{header}{body}")).unwrap();
    }
}

fn bench_index_build(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    write_index_tree(dir.path());
    let model = make_stub_model(16);

    let mut group = c.benchmark_group("index_build");
    group.sample_size(10);
    // `0` is rayon's default (one worker per CPU), pinned rather than left to
    // `CSP_INDEX_WORKERS` so the comparison is stable.
    for (name, workers) in [("300_files_serial", 1), ("300_files_parallel", 0)] {
        let options = CreateIndexOptions {
            display_root: Some(dir.path().to_path_buf()),
            index_workers: Some(workers),
            ..CreateIndexOptions::new(&model)
        };
        group.bench_function(name, |b| {
            b.iter(|| black_box(create_index_from_path(dir.path(), &options, None).unwrap()))
        });
    }
    group.finish();
}

criterion_group!(
    benches,
    bench_tokenize,
    bench_split_identifier,
    bench_file_path_penalty,
    bench_rerank_top_k,
    bench_index_build,
);
criterion_main!(benches);
