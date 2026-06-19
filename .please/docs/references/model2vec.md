# Reference Analysis — Model2Vec (`model2vec` / `model2vec-rs`)

> Analysis of [MinishLab/model2vec](https://github.com/MinishLab/model2vec) (Python) and its Rust
> inference crate [MinishLab/model2vec-rs](https://github.com/MinishLab/model2vec-rs). **This is a
> direct dependency, not a port source** — Model2Vec *is* csp's dense-retrieval leg. semble uses the
> `minishlab/potion-code-16M` static model; the csp Rust port (`crates/csp/src/indexing/dense.rs`)
> wires the official `model2vec-rs` `StaticModel`, with a deterministic stub fallback until
> integration lands (see `dense-embedding-is-a-stub`). This doc captures how the embeddings are
> produced, which model csp uses, the published benchmarks, and the Rust crate's API/limits.
>
> **Analyzed at**: GitHub READMEs + HF model cards, 2026-06-19. `model2vec-rs` `0.2.1` (May 2026).
> Both projects MIT. Sources in §6.

---

## 1. What Model2Vec is

A method (and toolkit) that **distills a sentence-transformer into a static embedding model**:
each vocabulary token gets one precomputed vector; encoding a string is a **vocab→vector lookup +
mean pooling**, *not* a transformer forward pass. Result: ~50× smaller, up to ~500× faster on CPU,
with a modest quality drop. No GPU, no API key, deterministic. This is exactly why semble/csp can
do "single-digit-millisecond" search on a laptop — the dense signal is a matrix gather, not
inference.

**Distillation pipeline** (`distill()`, ~30s on CPU, no training data needed):
1. **Vocabulary forward pass** — run the vocab through the teacher (e.g. `BAAI/bge-base-en-v1.5`)
   to get one embedding per token.
2. **Token→vector table + mean pooling** — store the table; inference pools token vectors.
3. **Post-processing** — **PCA** (dim reduction), **Zipf weighting** (down-weight frequent tokens),
   and **tokenlearn / POTION** (the training trick that lifted the `potion-*` generation above
   plain distillation).

---

## 2. The "potion" pre-trained models

| Model | Params | Dim | Task | Notes |
|---|---|---|---|---|
| `potion-base-2M` | 1.8M | — | general | smallest |
| `potion-base-4M` | 3.7M | — | general | |
| `potion-base-8M` | 7.5M | 256 | general | crate's example default |
| `potion-base-32M` | 32.3M | 512 | general | "most performant static model" |
| `potion-retrieval-32M` | 32.3M | 512 | retrieval | retrieval-tuned `potion-base-32M` |
| `potion-multilingual-128M` | 128M | — | 101 langs | multilingual |
| **`potion-code-16M`** | **16M** | **256** | **code** | **← csp/semble's model** (vocab ≈ 62.5k) |

**`potion-code-16M`** (the one that matters for csp) is distilled from **`CodeRankEmbed`** (137M
teacher), then **tokenlearn on CornStack pairs**, **contrastive fine-tune (MultipleNegativesRankingLoss)**,
and **post-SIF re-regularization**. 256-dim output — matching what `dense.rs` expects.

---

## 3. Benchmarks (and what they mean for csp)

**General-text MTEB** (from the model2vec results README):

| Model | MTEB avg | Retrieval (NDCG@10) |
|---|---|---|
| `potion-base-32M` | 52.13 | 32.67 |
| `potion-base-8M` | 51.08 | — |
| `potion-retrieval-32M` | — | 35.06 |

**Code retrieval — CoIR** (from the `potion-code-16M` HF card; this is csp's relevant number):

| Model | Params | CoIR avg | CosQA | CodeFeedback-ST | CodeFeedback-MT |
|---|---|---|---|---|---|
| `CodeRankEmbed` (teacher) | 137M | 59.14 | 35.92 | 78.11 | 42.61 |
| **`potion-code-16M`** | 16M | **37.05** | 21.37 | 50.27 | 36.26 |
| **`potion-code-16M` + BM25 hybrid** | 16M | **40.41** | 21.63 | 64.26 | 51.23 |

> **The load-bearing line for csp**: the model's own card reports **dense-only 37.05 → +BM25 hybrid
> 40.41 (+3.36)**. The Model2Vec authors themselves measure that pairing the static dense model with
> sparse BM25 beats dense alone — which is precisely csp/semble's hybrid + adaptive-alpha design, and
> the direct counter-argument to a dense-only engine like [cocoindex-code](cocoindex.md). Static
> embeddings trade ~22 NDCG points vs. the 137M teacher for orders-of-magnitude speed; the BM25 leg
> is how csp claws some of that back.

---

## 4. `model2vec-rs` — the Rust inference crate (what csp wires)

- **Crate**: `model2vec-rs` `0.2.1` (crates.io), 100% Rust, MIT. ~1.7× the Python throughput
  (≈8000 vs 4650 samples/s). **Inference-only**.
- **API** — `StaticModel` struct:
  - `from_pretrained(id_or_path, token, normalize, subfolder)` — load from HF Hub or local path.
  - `from_bytes(...)` — in-memory load (WASM / embedded).
  - `encode(&[String])` — default params; `encode_with_args(.., max_length, batch_size)` — custom.
- **Formats**: `safetensors` with **f32 / f16 / i8** weights. Tokenization via `onig` or
  `fancy-regex`. Feature flags `local-only`, `wasm`. Ships a CLI for single/batch encode.
- **Does NOT do**: distillation, training, fine-tuning, dynamic embeddings — those stay in the
  Python `model2vec`. The Rust crate is purely the lookup+pool inference path.

```rust
let model = StaticModel::from_pretrained("minishlab/potion-code-16M", None, None, None)?;
let embeddings = model.encode(&["where do we embed chunks".to_string()]);
```

### How csp uses it

`crates/csp/src/indexing/dense.rs` exposes a `Model` enum wrapping `model2vec-rs` `StaticModel`
(real path) **and** a deterministic stub (`TODO(integration)`), so the Rust port reproduces the TS
stub bit-for-bit for fixture-level parity (see [semble.md §4.6](semble.md), `dense-embedding-is-a-stub`).
`SelectableBasicBackend` does cosine kNN over the resulting matrix. When the stub is swapped for real
`potion-code-16M` weights, the dense leg becomes the table above; the BM25 leg and ranking are
unchanged.

---

## 5. Relevance map (Model2Vec → csp)

| Model2Vec concept | csp counterpart |
|---|---|
| `StaticModel` (Python) / `model2vec-rs::StaticModel` (Rust) | `dense.rs` `Model` enum (real + stub) |
| `potion-code-16M` (256-dim) | the embedding model semble/csp target |
| vocab→vector lookup + mean pooling | `embed_chunks` dense matrix build |
| cosine over embeddings | `SelectableBasicBackend` kNN |
| dense-only vs. +BM25 hybrid (40.41) | csp's RRF fusion of dense + BM25 (the whole point) |

---

## 6. Sources

- `MinishLab/model2vec` (method, distill/encode, potion models) — <https://github.com/MinishLab/model2vec>
- MTEB results table — <https://github.com/MinishLab/model2vec/blob/main/results/README.md#mteb-results>
- `MinishLab/model2vec-rs` (Rust inference crate) — <https://github.com/MinishLab/model2vec-rs>
- `minishlab/potion-code-16M` card (CoIR + hybrid numbers, training recipe) — <https://huggingface.co/minishlab/potion-code-16M>
