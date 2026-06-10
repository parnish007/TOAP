# TOAP Claims Ledger

This file is the source of truth for project claims. If a number is not listed under measured claims with reproduction metadata, it is not a project result.

## Current Status

v1.0 (Rust). Implemented and tested: protocol core (V1 text + V2 binary), context store
(ACL/capability-lattice/TTL/deltas/subscriptions), security (rate-limit/replay/capability), broker
(sessions/routing), client SDK, demo agents, MCP frontend, materialization policy + KV-bridge sidecar.
**28 automated Rust tests pass.** Three benchmark tiers exist (below): a synthetic wire-token harness,
a real multi-model token study (Claude Haiku/Sonnet/Opus), and a real multi-model KV-bridge compute
study (7 open models on a T4).

## Measured Claims

Source: `benchmark/runner.py`; tokenizer: tiktoken `cl100k_base` + `o200k_base`; baseline: A2A/
JSON-RPC-style messages that re-embed the document each turn; hardware: Windows 11, this dev host;
agents: **deterministic rule-based stand-ins (NOT LLMs)**; reproduce: `python benchmark/runner.py`.

| Scenario | Metric | Without TOAP | With TOAP | Reduction |
| --- | --- | --- | --- | --- |
| multi_turn_shared_doc (8 turns, 1 doc) | tokens (cl100k) | 2,807 | 843 | 3.33× |
| multi_turn_shared_doc | wire bytes | 12,588 | 3,137 | 4.01× |
| fanout_5_workers (co-located store) | tokens (cl100k) | 1,760 | 459 | 3.83× |
| fanout_5_workers (worst case: remote re-fetch) | tokens (cl100k) | 1,760 | 1,684 | 1.05× |
| opcode vs natural language (micro) | tokens (cl100k) | — | — | 0–17% only |

**Accuracy: identical (100% both)** — TOAP is content-lossless (same bytes delivered), so the
deterministic task gives the same answers. This proves *no accuracy loss from the transport*; it is
**not** evidence about real-LLM task accuracy (no LLM was run).

### Stated limitations (next to the claim, per policy)
- Synthetic workload, rule-based agents, one document, parameters chosen by us; baseline is a
  reasonable but self-authored JSON re-send.
- Savings are in **coordination/transport tokens**, NOT in the worker's own LLM *prompt* tokens
  (unchanged once a fetched doc enters a model prompt).
- Fan-out only wins when the store is co-located/reused; the remote-refetch row shows the weak case.
- Opcode terseness is a minor win (≤17%); the real win is context-by-ID dedup.

## Measured Claims — KV-bridge (compute plane)

Source: `benchmark/kv_bridge/` (`kv_bridge.py`, `kv_bench.py`, notebook); hardware: **NVIDIA T4**
(Google Colab); 7 open models × prefix lengths up to 8192; warmed-up CUDA-event timing of **prefill
only**; raw data: `benchmark/kv_bridge/kv_bench_all.json`; figure: `paper/figures/gen_kv_figure.py`.

| Quantity | Result |
| --- | --- |
| Resident prefill speedup (co-located, KV already on GPU) | up to **~181×** (Qwen2.5-0.5B @ 8192 tok); grows with prefix length + model size |
| Crossover (resident ≥ 1×) | ~256–1024 tokens depending on model; below it, no win |
| Cross-node speedup (includes KV serialize/transfer) | **>1× for GQA** (Qwen reaches 68×); **<1× for most MHA** at tested lengths |
| KV cache size vs text it replaces | **2,500–43,000×** larger; far larger for MHA than GQA |
| Losslessness (byte-identical greedy output) | **35/36 cells**; 1 fp16 one-token divergence (GPT-2 @512), exact in fp32 |

**What it proves:** same-model KV reuse is a real, near-lossless prefill speedup that scales with
context, but the cache is so much larger than the text that it only pays off co-located or under GQA —
grounding the `RegistryKvTransport` gating policy (same-model + co-location, else fall back to CTX_REF).

### Stated limitations (KV-bridge)
- Microbenchmark of one prefill step on a **single GPU (T4)**, vs **full recompute** — NOT vs provider
  prefix-caching (the real production alternative; we do not claim to beat it).
- "Transfer" = local serialize/deserialize, not a real RDMA/network hop.
- Same-model, same-tokenizer, **prefix reuse only** (no cross-position splicing).
- "Lossless" = token-identical in practice (fp16), not bit-exact in theory.

### Evidence tiers (read this before citing any number)

1. **Model-independent, reproducible (strongest):** TOAP wire frames vs JSON-RPC frames, counted with
   tiktoken. These are properties of the encodings; anyone reproduces them. (`benchmark/runner.py`.)
2. **Real-LLM measurement, single model (real but limited):** `benchmark/subagent_bench_results.md` —
   each agent is an isolated Claude subagent; token usage is **runtime-reported**, outputs are
   genuinely generated, accuracy scored by an **independent** judge subagent. Result on a real
   3-agent pipeline: **~1.40× fewer model tokens overall, ~1.65× on the downstream stages where TOAP
   applies, with 4/4 accuracy parity.** Limits: single model family, n=1, large constant subagent
   overhead calibrated once, judge same-family. This supersedes the earlier inflated "2.76×".
3. **Simulation, NOT measurement (do not cite as real):** `benchmark/real_llm_bench.py` — the agent
   outputs there are author-written string literals with no captured usage. Token counts are real but
   the "LLM inference" is not; keep it only as an illustrative harness.

For a publishable claim, tier 2 must be redone with ≥2 independent models, n≫1 with variance, and an
independent (different-family) judge — see `benchmark/subagent_bench_results.md` limitations.

## Design Targets

These are targets to test later:

| Target | What Must Be Measured |
| --- | --- |
| Reduce repeated wire bytes | Raw byte count for TOAP and baselines. |
| Reduce protocol/message tokens | Tokenizer count for coordination messages only. |
| Avoid repeated context transfer | Context store hit rate and transmitted context bytes. |
| Keep parsing deterministic | Parser unit tests and fuzz results. |
| Compare V1 and V2 overhead | Parse latency, encoded byte size, and CPU time. |
| Separate model cost from protocol cost | Model input tokens measured independently from wire tokens. |

## Non-Claims

TOAP does not currently claim:

- Measured 5x, 10x, or 20x savings (measured ~3–4× on the synthetic repeated-reference scenario).
- Reduced total LLM inference tokens for every workflow (saves transport, not inference).
- Improved answer quality, or any real-LLM accuracy result (no LLM has been run).
- Production-grade security (replay guard is per-session only; no message signing yet).
- Working KV-cache sharing (KV_BRIDGE is a typed stub, demoted per research.md §3).

## Required Metadata For Any Numeric Claim

Every numeric performance claim must include:

- Commit hash.
- Scenario name.
- Hardware and OS.
- Baseline implementation.
- Tokenizer and model details when token counts are involved.
- Raw benchmark output.
- Summary table.
- Reproduction command.

## Claim Review Checklist

- The result is generated by code in this repository.
- The baseline is documented.
- Wire bytes and model input tokens are not mixed.
- The sample size is stated.
- The limitation is stated next to the claim.
- The result can be reproduced from a clean checkout.
