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
use std::sync::{Arc, LazyLock, Mutex};

use model2vec_rs::model::StaticModel;

use crate::types::Chunk;

/// Default Model2Vec model name (kept identical to semble for parity;
/// semble#219 bumped this to the `-v2` weights).
pub const DEFAULT_MODEL_NAME: &str = "minishlab/potion-code-16M-v2";

/// Stub embedding dimension (the real `potion-code-16M-v2` emits 256-dim vectors).
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

mod backend;

pub use backend::{BasicArgs, SelectableBasicBackend};

#[cfg(test)]
mod tests;
