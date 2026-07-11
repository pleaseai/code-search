//! In-memory cosine vector backend with selector filtering and persistence.

use std::path::Path;

use serde::{Deserialize, Serialize};

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
        if !vectors.is_empty() && dim == 0 {
            return Err(
                "Vector dimension must be greater than 0 for a non-empty index".to_string(),
            );
        }
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
        if meta.rows > 0 && meta.dim == 0 {
            return Err(
                "Invalid vector dimension: dim must be greater than 0 for a non-empty index"
                    .to_string(),
            );
        }

        let bytes = std::fs::read(dir.join("vectors.bin")).map_err(|e| e.to_string())?;
        let expected = meta
            .rows
            .checked_mul(meta.dim)
            .and_then(|elements| elements.checked_mul(std::mem::size_of::<f32>()))
            .ok_or_else(|| "Vector dimensions are too large (overflow)".to_string())?;
        if bytes.len() != expected {
            return Err(format!(
                "Vector file size mismatch: expected {expected} bytes, got {}",
                bytes.len()
            ));
        }

        let mut byte_chunks = bytes.chunks_exact(4);
        let mut vectors = Vec::with_capacity(meta.rows);
        for _ in 0..meta.rows {
            let mut row = Vec::with_capacity(meta.dim);
            for _ in 0..meta.dim {
                let arr: [u8; 4] = byte_chunks
                    .next()
                    .expect("validated vector byte count")
                    .try_into()
                    .expect("4-byte chunk");
                row.push(f32::from_le_bytes(arr));
            }
            vectors.push(row);
        }
        let mut backend = Self::new(vectors, meta.arguments)?;
        if meta.rows == 0 {
            backend.dim = meta.dim;
        }
        Ok(backend)
    }
}

#[derive(Serialize, Deserialize)]
struct BackendMeta {
    rows: usize,
    dim: usize,
    arguments: BasicArgs,
}
