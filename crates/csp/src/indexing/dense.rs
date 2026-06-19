//! Dense embeddings + cosine vector backend. Port of `src/indexing/dense.ts`
//! (← semble `index/dense.py`).
//!
//! [`load_model`] loads a **real** Model2Vec model via `model2vec-rs` (the
//! official MinishLab Rust port) — `StaticModel::from_pretrained(id_or_path)` +
//! `encode` — matching semble's `StaticModel`. When the model can't be loaded
//! (offline, missing weights, bad path) it falls back to a deterministic stub
//! embedder so indexing still works; the stub reproduces the former TS stub
//! bit-for-bit (FNV-1a over UTF-16 units, mulberry32, Box-Muller, exact f64↔f32
//! narrowing) and is also what the offline unit tests use.
//!
//! `SelectableBasicBackend` is the in-memory cosine backend with optional
//! candidate-selector filtering and a csp-local on-disk format.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use model2vec_rs::model::StaticModel;
use serde::{Deserialize, Serialize};

use crate::types::Chunk;

/// Default Model2Vec model name (kept identical to semble for parity).
pub const DEFAULT_MODEL_NAME: &str = "minishlab/potion-code-16M";

/// Stub embedding dimension (the real `potion-code-16M` emits 256-dim vectors).
const DEFAULT_STUB_DIM: usize = 256;

/// Deterministic 32-bit FNV-1a over UTF-16 code units (matches JS `charCodeAt`).
fn fnv1a(s: &str) -> u32 {
    let mut h: u32 = 0x811C_9DC5;
    for unit in s.encode_utf16() {
        h ^= unit as u32;
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

/// Mulberry32 PRNG — deterministic, matching the JS implementation's u32 ops.
struct Mulberry32 {
    a: u32,
}

impl Mulberry32 {
    fn new(seed: u32) -> Self {
        Self { a: seed }
    }

    fn next_unit(&mut self) -> f64 {
        self.a = self.a.wrapping_add(0x6D2B_79F5);
        let mut t = self.a;
        t = (t ^ (t >> 15)).wrapping_mul(t | 1);
        // JS `t ^= t + Math.imul(...)`: the `+` is exact, then `^=` reduces mod
        // 2^32 — i.e. a wrapping add followed by xor.
        t ^= t.wrapping_add((t ^ (t >> 7)).wrapping_mul(t | 61));
        ((t ^ (t >> 14)) as f64) / 4_294_967_296.0
    }
}

/// Build a deterministic unit-length vector from a string. Reproduces the TS
/// `stub_embed` exactly, including its f64↔f32 narrowing: `g` is stored to f32,
/// but `norm` accumulates the pre-narrowing f64 `g`, and the final scale reads
/// the f32 value back, divides in f64, and re-narrows.
fn stub_embed(text: &str, dim: usize) -> Vec<f32> {
    let mut rng = Mulberry32::new(fnv1a(text));
    let mut v = vec![0f32; dim];
    let mut norm: f64 = 0.0;
    for slot in v.iter_mut() {
        let u1 = rng.next_unit().max(1e-12);
        let u2 = rng.next_unit();
        let g = (-2.0 * u1.ln()).sqrt() * (2.0 * std::f64::consts::PI * u2).cos();
        *slot = g as f32;
        norm += g * g;
    }
    norm = norm.sqrt();
    if norm == 0.0 || norm.is_nan() {
        norm = 1.0; // matches JS `Math.sqrt(norm) || 1` (0 and NaN → 1)
    }
    for slot in v.iter_mut() {
        *slot = ((*slot as f64) / norm) as f32;
    }
    v
}

/// A loaded embedding model: either a real Model2Vec model (`model2vec-rs`) or a
/// deterministic stub (tests / offline fallback). Both expose `.encode(texts)`
/// and `.dim()`.
#[derive(Clone)]
pub enum Model {
    /// Real Model2Vec. `Arc` keeps `Clone` cheap and the model `Send + Sync`.
    Static { inner: Arc<StaticModel>, dim: usize },
    /// Deterministic hash-seeded stub (reproduces the former TS stub bit-for-bit).
    Stub { dim: usize },
}

impl std::fmt::Debug for Model {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Model::Static { dim, .. } => f.debug_struct("Model::Static").field("dim", dim).finish(),
            Model::Stub { dim } => f.debug_struct("Model::Stub").field("dim", dim).finish(),
        }
    }
}

impl Model {
    /// Embed each text into a row vector (one row per input).
    pub fn encode(&self, texts: &[String]) -> Vec<Vec<f32>> {
        match self {
            Model::Static { inner, .. } => inner.encode(texts),
            Model::Stub { dim } => texts.iter().map(|t| stub_embed(t, *dim)).collect(),
        }
    }

    /// Embedding dimension.
    pub fn dim(&self) -> usize {
        match self {
            Model::Static { dim, .. } | Model::Stub { dim } => *dim,
        }
    }
}

/// Construct a stub model of the given dimension (tests / offline fallback).
pub fn make_stub_model(dim: usize) -> Model {
    Model::Stub { dim }
}

/// Load a real Model2Vec model from a HF repo id or local directory. Probes the
/// embedding dimension once via a single-token encode.
fn load_static(path: &str) -> Result<Model, String> {
    let inner = StaticModel::from_pretrained(path, None, None, None).map_err(|e| e.to_string())?;
    let dim = inner.encode_single("a").len();
    if dim == 0 {
        return Err(format!(
            "model '{path}' produced a zero-dimension embedding"
        ));
    }
    Ok(Model::Static {
        inner: Arc::new(inner),
        dim,
    })
}

static MODEL_CACHE: LazyLock<Mutex<HashMap<String, Model>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Load (and cache) a model by path, defaulting to [`DEFAULT_MODEL_NAME`].
/// Returns the model and the resolved path. Falls back to the deterministic stub
/// (with a warning) when the real model can't be loaded, so indexing degrades
/// gracefully offline.
pub fn load_model(model_path: Option<&str>) -> (Model, String) {
    load_model_with(model_path, load_static)
}

/// Cache + fallback orchestration with an injectable loader (the seam unit tests
/// use to stay offline).
fn load_model_with(
    model_path: Option<&str>,
    load: impl Fn(&str) -> Result<Model, String>,
) -> (Model, String) {
    let resolved = model_path.unwrap_or(DEFAULT_MODEL_NAME).to_string();
    let mut cache = MODEL_CACHE.lock().expect("model cache mutex");
    if let Some(model) = cache.get(&resolved) {
        return (model.clone(), resolved);
    }
    let model = load(&resolved).unwrap_or_else(|e| {
        eprintln!(
            "csp: could not load Model2Vec model '{resolved}': {e}. \
             Falling back to the deterministic stub embedder — set --model to a valid \
             Model2Vec id/path (and ensure network/HF cache) for real embeddings."
        );
        make_stub_model(DEFAULT_STUB_DIM)
    });
    cache.insert(resolved.clone(), model.clone());
    (model, resolved)
}

/// Embed chunks with the model — one row per chunk, `[]` for empty input.
pub fn embed_chunks(model: &Model, chunks: &[Chunk]) -> Vec<Vec<f32>> {
    if chunks.is_empty() {
        return Vec::new();
    }
    let texts: Vec<String> = chunks.iter().map(|c| c.content.clone()).collect();
    model.encode(&texts)
}

// ---------------------------------------------------------------------------
// SelectableBasicBackend
// ---------------------------------------------------------------------------

/// Backend arguments. For parity only cosine is supported.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BasicArgs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metric: Option<String>,
}

impl Default for BasicArgs {
    fn default() -> Self {
        Self {
            metric: Some("cosine".to_string()),
        }
    }
}

/// L2-normalise a vector in place (f64 accumulation, f32 storage — matching TS).
/// Zero vectors stay zero.
fn normalize_in_place(v: &mut [f32]) {
    let mut n: f64 = 0.0;
    for &x in v.iter() {
        n += (x as f64) * (x as f64);
    }
    n = n.sqrt();
    if n == 0.0 {
        return;
    }
    for x in v.iter_mut() {
        *x = ((*x as f64) / n) as f32;
    }
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    let mut s = 0.0;
    for i in 0..a.len() {
        s += (a[i] as f64) * (b[i] as f64);
    }
    s
}

/// In-memory cosine vector backend with optional candidate-selector filtering —
/// port of `SelectableBasicBackend(CosineBasicBackend)`.
#[derive(Debug)]
pub struct SelectableBasicBackend {
    /// Pre-normalised row vectors.
    pub vectors: Vec<Vec<f32>>,
    pub arguments: BasicArgs,
    pub dim: usize,
}

impl SelectableBasicBackend {
    /// Build from raw vectors (defensively copied and L2-normalised so cosine
    /// distance reduces to `1 - dot`). Errors on inconsistent dimensions.
    pub fn new(vectors: Vec<Vec<f32>>, arguments: BasicArgs) -> Result<Self, String> {
        let dim = vectors.first().map(Vec::len).unwrap_or(0);
        let mut normalized = Vec::with_capacity(vectors.len());
        for v in vectors {
            if v.len() != dim {
                return Err(format!(
                    "Inconsistent vector dimensions: expected {dim}, got {}",
                    v.len()
                ));
            }
            let mut copy = v;
            normalize_in_place(&mut copy);
            normalized.push(copy);
        }
        Ok(Self {
            vectors: normalized,
            arguments,
            dim,
        })
    }

    /// Convenience constructor with default (cosine) arguments.
    pub fn from_vectors(vectors: Vec<Vec<f32>>) -> Result<Self, String> {
        Self::new(vectors, BasicArgs::default())
    }

    /// Batched k-NN query. Returns, per query, `[(chunk_index, cosine_distance)]`
    /// sorted by ascending distance. `selector` constrains results to a pool.
    pub fn query(
        &self,
        query_vectors: &[Vec<f32>],
        k: usize,
        selector: Option<&[u32]>,
    ) -> Result<Vec<Vec<(usize, f64)>>, String> {
        if k < 1 {
            return Err(format!("k should be >= 1, is now {k}"));
        }

        let num_vectors = self.vectors.len();
        let mut effective_k = k.min(num_vectors);
        if let Some(sel) = selector {
            for &idx in sel {
                if idx as usize >= num_vectors {
                    return Err(format!(
                        "Selector index out of bounds: {idx} (total vectors: {num_vectors})"
                    ));
                }
            }
            effective_k = effective_k.min(sel.len());
        }

        let mut out: Vec<Vec<(usize, f64)>> = Vec::with_capacity(query_vectors.len());
        if effective_k == 0 {
            out.resize(query_vectors.len(), Vec::new());
            return Ok(out);
        }

        for raw in query_vectors {
            if raw.len() != self.dim {
                return Err(format!(
                    "Query vector dimension mismatch: expected {}, got {}",
                    self.dim,
                    raw.len()
                ));
            }
            let mut q = raw.clone();
            normalize_in_place(&mut q);

            let pool_size = selector.map(<[u32]>::len).unwrap_or(num_vectors);
            // (pool_idx, distance) pairs, stably sorted by ascending distance.
            let mut pairs: Vec<(usize, f64)> = (0..pool_size)
                .map(|i| {
                    let vec_idx = selector.map_or(i, |s| s[i] as usize);
                    (i, 1.0 - dot(&q, &self.vectors[vec_idx]))
                })
                .collect();
            // total_cmp is NaN-safe (a stray NaN distance can't panic the sort).
            pairs.sort_by(|a, b| a.1.total_cmp(&b.1));
            pairs.truncate(effective_k);

            let mapped: Vec<(usize, f64)> = pairs
                .into_iter()
                .map(|(pool_idx, dist)| (selector.map_or(pool_idx, |s| s[pool_idx] as usize), dist))
                .collect();
            out.push(mapped);
        }

        Ok(out)
    }

    /// Persist vectors + args to `<dir>/vectors.bin` (flat little-endian f32) and
    /// `<dir>/args.json`.
    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let mut bytes = Vec::with_capacity(self.vectors.len() * self.dim * 4);
        for row in &self.vectors {
            for &x in row {
                bytes.extend_from_slice(&x.to_le_bytes());
            }
        }
        std::fs::write(dir.join("vectors.bin"), &bytes)?;

        let meta = BackendMeta {
            rows: self.vectors.len(),
            dim: self.dim,
            arguments: self.arguments.clone(),
        };
        let json = serde_json::to_string(&meta)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(dir.join("args.json"), json)
    }

    /// Inverse of [`save`](Self::save).
    pub fn load(dir: &Path) -> Result<Self, String> {
        let meta_raw = std::fs::read_to_string(dir.join("args.json")).map_err(|e| e.to_string())?;
        let meta: BackendMeta = serde_json::from_str(&meta_raw).map_err(|e| e.to_string())?;

        let bytes = std::fs::read(dir.join("vectors.bin")).map_err(|e| e.to_string())?;
        let expected = meta.rows * meta.dim * 4;
        if bytes.len() != expected {
            return Err(format!(
                "Vector file size mismatch: expected {expected} bytes, got {}",
                bytes.len()
            ));
        }

        let mut vectors = Vec::with_capacity(meta.rows);
        for r in 0..meta.rows {
            let mut row = Vec::with_capacity(meta.dim);
            for c in 0..meta.dim {
                let off = (r * meta.dim + c) * 4;
                let arr: [u8; 4] = bytes[off..off + 4].try_into().expect("4-byte chunk");
                row.push(f32::from_le_bytes(arr));
            }
            vectors.push(row);
        }
        Self::new(vectors, meta.arguments)
    }
}

#[derive(Serialize, Deserialize)]
struct BackendMeta {
    rows: usize,
    dim: usize,
    arguments: BasicArgs,
}

#[cfg(test)]
mod tests {
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

    /// Real Model2Vec load — downloads `minishlab/potion-code-16M` from HF on
    /// first run, so it's network-gated and not part of the default suite.
    /// Run with: `cargo test -p csp -- --ignored real_model2vec`.
    #[test]
    #[ignore = "network: downloads potion-code-16M from Hugging Face"]
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
}
