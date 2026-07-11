use super::*;
use tempfile::tempdir;

fn chunk(content: &str) -> Chunk {
    Chunk {
        content: content.to_string(),
        file_path: "f.ts".to_string(),
        start_line: 1,
        end_line: 1,
        language: None,
    }
}

// --- stub parity (golden vectors captured from the TS implementation) ---

#[test]
fn fnv1a_matches_ts() {
    assert_eq!(fnv1a("hello"), 1_335_831_723);
}

#[test]
fn stub_embed_matches_ts_golden() {
    // Golden values captured from the TS `stubEmbed` (Float32Array entries
    // widened to f64); `as f32` reproduces the exact stored f32.
    let expected_hello: [f64; 8] = [
        0.085_591_696_202_754_97,
        -0.438_301_533_460_617_07,
        -0.693_752_408_027_648_9,
        0.431_218_117_475_509_64,
        -0.016_508_268_192_410_47,
        -0.213_292_211_294_174_2,
        0.267_603_516_578_674_3,
        0.126_279_816_031_456,
    ];
    let hello = stub_embed("hello", 8);
    for (got, want) in hello.iter().zip(&expected_hello) {
        assert_eq!(*got, *want as f32);
    }

    let expected_foo: [f64; 4] = [
        0.054_837_439_209_222_794,
        -0.873_466_372_489_929_2,
        -0.401_930_719_614_028_93,
        -0.269_260_287_284_851_1,
    ];
    let foo = stub_embed("foo", 4);
    for (got, want) in foo.iter().zip(&expected_foo) {
        assert_eq!(*got, *want as f32);
    }
}

#[test]
fn stub_embed_is_unit_length() {
    let v = stub_embed("anything", 256);
    let norm: f64 = v
        .iter()
        .map(|&x| (x as f64) * (x as f64))
        .sum::<f64>()
        .sqrt();
    assert!((norm - 1.0).abs() < 1e-5);
}

// --- load_model / embed_chunks ---

#[test]
fn load_model_defaults_path_via_seam() {
    // Offline: inject a loader so no network/model download happens.
    let (model, path) = load_model_with(None, |_| Ok(make_stub_model(7)));
    assert_eq!(path, DEFAULT_MODEL_NAME);
    assert!(model.dim() > 0);
}

#[test]
fn load_model_resolves_distinct_paths_and_caches() {
    // Distinct paths each load once; a repeat path is served from cache.
    let (_, a) = load_model_with(Some("seam/path-X"), |_| Ok(make_stub_model(4)));
    let (_, b) = load_model_with(Some("seam/path-Y"), |_| Ok(make_stub_model(4)));
    // The loader must NOT fire for an already-cached path — panic proves it.
    let (_, a2) = load_model_with(Some("seam/path-X"), |_| {
        panic!("cached path must not reload")
    });
    assert_eq!(a, "seam/path-X");
    assert_eq!(b, "seam/path-Y");
    assert_eq!(a2, "seam/path-X");
}

#[test]
fn load_model_falls_back_to_stub_on_error() {
    let (model, path) = load_model_with(Some("seam/will-fail"), |_| Err("boom".to_string()));
    assert_eq!(path, "seam/will-fail");
    assert_eq!(model.dim(), DEFAULT_STUB_DIM); // stub fallback
}

/// Real Model2Vec load — downloads `minishlab/potion-code-16M-v2` from HF on
/// first run, so it's network-gated and not part of the default suite.
/// Run with: `cargo test -p csp -- --ignored real_model2vec`.
#[test]
#[ignore = "network: downloads potion-code-16M-v2 from Hugging Face"]
fn real_model2vec_loads_and_embeds() {
    let model = load_static(DEFAULT_MODEL_NAME).expect("load real model");
    assert!(model.dim() > 0);
    let vecs = model.encode(&["fn main() {}".to_string(), "def main(): pass".to_string()]);
    assert_eq!(vecs.len(), 2);
    assert_eq!(vecs[0].len(), model.dim());
    assert_ne!(vecs[0], vecs[1]);
}

#[test]
fn embed_empty_is_empty() {
    let model = make_stub_model(8);
    assert!(embed_chunks(&model, &[]).is_empty());
}

#[test]
fn embed_one_per_chunk() {
    let model = make_stub_model(8);
    let vectors = embed_chunks(&model, &[chunk("a"), chunk("b")]);
    assert_eq!(vectors.len(), 2);
    for v in &vectors {
        assert_eq!(v.len(), 8);
    }
}

#[test]
fn embed_is_deterministic() {
    let model = make_stub_model(16);
    let v1 = embed_chunks(&model, &[chunk("same")]);
    let v2 = embed_chunks(&model, &[chunk("same")]);
    assert_eq!(v1, v2);
}

#[test]
fn embed_differs_by_content() {
    let model = make_stub_model(16);
    let v1 = embed_chunks(&model, &[chunk("alpha")]);
    let v2 = embed_chunks(&model, &[chunk("beta")]);
    assert_ne!(v1, v2);
}

// --- SelectableBasicBackend::query ---

fn backend(n: usize, dim: usize) -> SelectableBasicBackend {
    let model = make_stub_model(dim);
    let vectors: Vec<Vec<f32>> = (0..n)
        .map(|i| stub_embed(&format!("doc{i}"), dim))
        .collect();
    let _ = model;
    SelectableBasicBackend::from_vectors(vectors).unwrap()
}

#[test]
fn query_rejects_k_below_one() {
    let b = backend(3, 8);
    assert!(b.query(&[b.vectors[0].clone()], 0, None).is_err());
}

#[test]
fn new_rejects_non_empty_zero_dimension_vectors() {
    let err = SelectableBasicBackend::from_vectors(vec![vec![]]).unwrap_err();
    assert!(err.contains("dimension must be greater than 0"));
}

#[test]
fn new_rejects_inconsistent_dims() {
    let v0 = stub_embed("x", 8);
    let truncated = v0[..4].to_vec();
    let err = SelectableBasicBackend::from_vectors(vec![v0, truncated]).unwrap_err();
    assert!(err.contains("Inconsistent vector dimensions"));
}

#[test]
fn query_rejects_dim_mismatch() {
    let b = backend(3, 8);
    let bad = vec![0f32; 4];
    let err = b.query(&[bad], 1, None).unwrap_err();
    assert!(err.contains("Query vector dimension mismatch"));
}

#[test]
fn query_rejects_selector_out_of_bounds() {
    let b = backend(3, 8);
    let err = b.query(&[b.vectors[0].clone()], 1, Some(&[5])).unwrap_err();
    assert!(err.contains("Selector index out of bounds"));
}

#[test]
fn query_returns_sorted_topk_with_self_nearest() {
    let b = backend(3, 8);
    let results = b.query(&[b.vectors[0].clone()], 3, None).unwrap();
    assert_eq!(results.len(), 1);
    let hits = &results[0];
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].0, 0);
    assert!(hits[0].1.abs() < 1e-5);
    for i in 1..hits.len() {
        assert!(hits[i].1 >= hits[i - 1].1);
    }
}

#[test]
fn query_respects_selector_pool() {
    let b = backend(4, 8);
    let results = b.query(&[b.vectors[0].clone()], 2, Some(&[1, 2])).unwrap();
    let hits = &results[0];
    assert_eq!(hits.len(), 2);
    for (idx, _) in hits {
        assert!(*idx == 1 || *idx == 2);
    }
}

#[test]
fn query_handles_multiple_queries() {
    let b = backend(3, 8);
    let results = b
        .query(&[b.vectors[0].clone(), b.vectors[1].clone()], 1, None)
        .unwrap();
    assert_eq!(results.len(), 2);
    assert_eq!(results[0][0].0, 0);
    assert_eq!(results[1][0].0, 1);
}

#[test]
fn query_caps_k_at_num_vectors() {
    let b = backend(2, 8);
    let results = b.query(&[b.vectors[0].clone()], 5, None).unwrap();
    assert_eq!(results[0].len(), 2);
}

// --- save / load ---

#[test]
fn save_load_round_trips() {
    let original = backend(3, 8);
    let dir = tempdir().unwrap();
    original.save(dir.path()).unwrap();

    let loaded = SelectableBasicBackend::load(dir.path()).unwrap();
    assert_eq!(loaded.vectors.len(), original.vectors.len());
    assert_eq!(loaded.dim, original.dim);
    for (a, b) in loaded.vectors.iter().zip(&original.vectors) {
        assert_eq!(a, b);
    }

    let q = vec![original.vectors[0].clone()];
    let orig_hits: Vec<usize> = original.query(&q, 3, None).unwrap()[0]
        .iter()
        .map(|h| h.0)
        .collect();
    let loaded_hits: Vec<usize> = loaded.query(&q, 3, None).unwrap()[0]
        .iter()
        .map(|h| h.0)
        .collect();
    assert_eq!(orig_hits, loaded_hits);
}

#[test]
fn load_preserves_dimension_for_empty_index() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("args.json"),
        r#"{"rows":0,"dim":8,"arguments":{"metric":"cosine"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("vectors.bin"), []).unwrap();

    let loaded = SelectableBasicBackend::load(dir.path()).unwrap();
    assert!(loaded.vectors.is_empty());
    assert_eq!(loaded.dim, 8);
}

#[test]
fn load_rejects_zero_dimension_for_non_empty_index() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("args.json"),
        r#"{"rows":1,"dim":0,"arguments":{"metric":"cosine"}}"#,
    )
    .unwrap();
    std::fs::write(dir.path().join("vectors.bin"), []).unwrap();

    let err = SelectableBasicBackend::load(dir.path()).unwrap_err();
    assert!(err.contains("dim must be greater than 0"));
}

#[test]
fn load_rejects_overflowing_dimensions() {
    let dir = tempdir().unwrap();
    std::fs::write(
        dir.path().join("args.json"),
        format!(
            r#"{{"rows":{},"dim":2,"arguments":{{"metric":"cosine"}}}}"#,
            usize::MAX
        ),
    )
    .unwrap();
    std::fs::write(dir.path().join("vectors.bin"), []).unwrap();

    let err = SelectableBasicBackend::load(dir.path()).unwrap_err();
    assert!(err.contains("overflow"));
}

#[test]
fn load_rejects_truncated_vectors() {
    let original = backend(3, 8);
    let dir = tempdir().unwrap();
    original.save(dir.path()).unwrap();
    // Truncate vectors.bin to half its size.
    let path = dir.path().join("vectors.bin");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(SelectableBasicBackend::load(dir.path()).is_err());
}
