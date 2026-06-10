# KV-Bridge: implementation, experiment plan, and the Rust↔Python boundary

This directory contains the **real** implementation and benchmark of TOAP's third reference
materialization, `KV_BRIDGE`. It turns the paper's weakest claim ("KV-bridge is interface-only") into a
measured result, and makes the architecture honest: KV-bridge is **part of the protocol**, not just a
benchmark.

## Why KV-bridge is architectural, not a side experiment

TOAP's reference primitive (N3) has three materializations: `INLINE` → `CTX_REF` → `KV_BRIDGE`. The
*policy* for choosing among them lives in the Rust core and is unit-tested:

- `crates/toap-context`: the `Materialization` enum, `choose_materialization`,
  `materialize_with_fallback`, and **`RegistryKvTransport`** — which enforces the **same-model
  constraint** and holds a `(ctx, model) → handle` registry, with `register` / `evict`.
- The broker consults `KvTransport::fetch(ctx, model)` to decide whether a bridge is *available*; if
  not (no runtime, model mismatch, evicted) it **gracefully degrades** to `CTX_REF` / `INLINE`.

What requires a GPU model runtime is only the **tensor engine** — producing, sizing, transferring, and
reusing the actual KV cache. That is this directory (Python / PyTorch / transformers). The split is
deliberate:

```
Rust core (policy)                         Python sidecar (tensor engine)
------------------                         ------------------------------
choose_materialization                     extract_kv          (produce)
RegistryKvTransport (same-model gating)    serialize_kv        (transfer)
materialize_with_fallback                  deserialize_kv      (rehydrate)
KvTransport::fetch -> handle?              generate_kv_bridge  (reuse: prefill only the query)
```

`RegistryKvTransport.register(ctx, model, handle)` is exactly the call the sidecar makes after
`extract_kv` succeeds. The benchmark measures whether that handle is worth using.

## The hypothesis under test (falsifiable, and kept honest)

KV reuse is a crowded field (KVComm, CacheGen, vLLM/SGLang prefix caching). We claim **no** novelty in
KV compression or transport. We test one specific question:

> For the **same model + tokenizer**, with the shared context as a **prefix** (absolute positions
> preserved — no cross-position splicing), does reusing the transferred prefix KV (a) **skip the
> prefix's prefill** and (b) stay **token-for-token lossless** vs recompute — and at what **KV byte
> cost** vs the text it replaces?

Quantities per (model, prefix-length) cell, all from **warmed-up CUDA-event timing of prefill only**
(decode excluded; warmup kills the first-call autotune artifact that fakes a <1× slowdown):

1. **`speedup_resident`** — `recompute_prefill / query-prefill-on-resident-KV`. The KV is already on the
   GPU (co-located / shared-broker / NVLink case) — this is the **pure compute win**, the headline.
2. **`speedup_crossnode`** — `recompute_prefill / (query-prefill + KV deserialize)`. Includes the
   **transfer tax** — the honest number when producer and consumer are on different machines.
3. **`kv_vs_text_ratio`** — KV bytes ÷ text bytes (KV ≫ text; larger for MHA than GQA).
4. **`outputs_match`** — greedy tokens must match recompute **exactly** (the losslessness claim).

Each timing reports median + std over N iterations. If the bridge does not beat recompute, or transfer
dominates, the JSON says so. The two speedups are the whole argument: KV-bridge wins big co-located,
but the transfer cost is why TOAP gates it behind same-model + co-location and otherwise falls back to
`CTX_REF`.

## Model matrix (Colab T4, 16 GB)

This is a **different axis** from the Claude Haiku/Sonnet/Opus study: that one measured context-
*reference* token savings; this one measures KV-*reuse* compute savings. We sweep open models spanning
attention architectures:

| Model | Arch | Why it's in the matrix |
|---|---|---|
| `gpt2`, `gpt2-large` | MHA, ctx 1024 | baseline multi-head attention |
| `EleutherAI/pythia-410m`, `pythia-1.4b` | NeoX + RoPE, ctx 2048 | rotary embeddings, longer context |
| `Qwen/Qwen2.5-0.5B/1.5B-Instruct` | **GQA**, long ctx | grouped-query attention → much smaller KV |
| `mistralai/Mistral-7B-Instruct-v0.3` (`--load-in-4bit`) | 7B GQA | does the win hold at 7B scale? |

GQA models matter: they shrink the KV cache dramatically, changing the byte-cost side of the trade-off
— exactly the cross-architecture result a reviewer wants.

## How to run

**Colab (recommended):** open `kv_bridge_colab.ipynb`, set runtime to **T4 GPU**, **Run all**. It runs
the matrix with per-model error isolation and writes `kv_bench_all.json`. Send that back.

**Local (RTX 3050, ~8 GB):** smaller models only; 7B needs `--load-in-4bit` and may still OOM.

```bash
pip install -r requirements.txt
python kv_bench.py --model gpt2                        # auto-picks valid lengths < ctx
python kv_bench.py --model EleutherAI/pythia-410m --prefix-tokens 256 512 1024 2048
python kv_bench.py --model Qwen/Qwen2.5-0.5B-Instruct  # GQA
```

## Robustness notes (why earlier Colab runs crashed, and the fixes)

- **`None` KV layers / version drift** — modern `transformers` cache objects differ and may carry
  `None` placeholders. `_iter_layers` normalizes DynamicCache / legacy tuples and skips `None`, so
  `kv_byte_size` / `serialize_kv` no longer crash.
- **`2817 > 1024` over-tokenization** — the prefix is built by **tiling token IDs** to an exact length
  and **clamped to the model context window**, so GPT-2 never sees a 2817-token string.
- **`torch_dtype` deprecation** — `load()` uses the modern `dtype=` arg (fallback for old versions).
- **OOM tolerance** — each cell is wrapped; CUDA OOM skips that cell and frees VRAM rather than
  aborting the sweep.
- **Transfer vs compute conflation** — deserialize time is now timed separately (`transfer_ms`), not
  folded into bridge prefill.

## Interpreting the result for the paper

Expected, honest shape (to be confirmed by your run):
- **Prefill speedup grows with prefix length** and model size (more prefill skipped).
- **KV/text ratio is large** (often 100–1000×), and **larger for MHA than GQA** — the reason the
  literature says "re-send text + prefix cache" often wins unless producer and consumer are co-located.
- **Correctness `match=True`** for prefix-reuse; any `False` is a red flag we must explain, not hide.

The paper will report this as: *KV-bridge delivers a real, token-lossless prefill saving that scales
with shared-prefix length, but at a KV-byte cost orders of magnitude above the text — so it is justified
only for co-located, same-model agents with long shared prefixes; otherwise the policy correctly
degrades to `CTX_REF`.* A defensible, non-overclaimed contribution.

## Output

`kv_bench_all.json` (Colab matrix) or `kv_bench_results.json` (single local run). Send it back; numbers
get folded into `paper/main.tex` and `docs/claims.md`. Nothing is hand-written — if KV-bridge loses on
your hardware, the table will say so.
