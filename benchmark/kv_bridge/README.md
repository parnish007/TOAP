# KV-Bridge — real implementation + benchmark

This is the implemented version of TOAP's `KV_BRIDGE` materialization (Rust side: the `KvTransport`
trait in `toap-context`). Instead of re-sending a shared context as text and making the next agent
re-prefill it, the bridge transfers the already-computed **key/value cache** so the receiver prefills
only its short query.

## What it actually does (and its one honest constraint)

It implements the **correct** case: **same model, same tokenizer, prefix reuse**. The shared
context's KV is computed at absolute positions `0..n`; the query is appended at `n..`, so RoPE /
absolute positions stay valid. We deliberately do **not** splice a cache baked at different positions
— that is the failure mode the literature warns about (RoPE offset, cross-context attention loss), and
pretending it works would be dishonest.

## Files

- `kv_bridge.py` — extract / serialize / transfer / reuse KV; recompute vs bridge generation.
- `kv_bench.py` — measures the three things that decide if KV-bridge is worth it:
  1. **prefill latency** saved (the upside),
  2. **KV byte size vs text size** (the honest cost — a KV cache is far bigger than the text),
  3. **output correctness** (greedy tokens must match the recompute baseline — losslessness).

## Run it

### On your RTX 3050 (local, Windows/Linux)

```bash
cd benchmark/kv_bridge
pip install transformers
pip install torch --index-url https://download.pytorch.org/whl/cu121   # CUDA 12.x for the 3050
python kv_bench.py --model gpt2 --prefix-tokens 128 256 512 1024 --new-tokens 32 --repeats 5
# larger / more realistic:
python kv_bench.py --model EleutherAI/pythia-410m --prefix-tokens 256 512 1024 2048 --repeats 5
```

The 3050 has 4 GB VRAM, so stick to small models (`gpt2`, `distilgpt2`, `pythia-160m/410m`). Use fp16
on CUDA (the script does this automatically).

### On Google Colab (free T4, more VRAM)

Open `kv_bridge_colab.ipynb` (in this folder) and Run All, or paste:

```python
!pip -q install transformers
!git clone https://github.com/parnish007/TOAP.git
%cd TOAP/benchmark/kv_bridge
!python kv_bench.py --model gpt2-large --prefix-tokens 256 512 1024 2048 --new-tokens 32 --repeats 5
```

A T4 (16 GB) comfortably runs `gpt2-large` / `pythia-1.4b`, where the prefill-skip is more visible.

## Output

Writes `kv_bench_results.json`. **Send that file back** and the real numbers get folded into the
paper (`paper/main.tex`) and `docs/claims.md`. Nothing is hand-written — if KV-bridge does not beat
recompute on your hardware, the table will say so.

## How this connects to the Rust protocol

In TOAP proper, `toap-context::materialize_with_fallback` returns `Materialization::KvBridge { ctx,
model }` only when sender and receiver share a model and a transport handle exists; otherwise it
degrades to `CtxRef`. A production `KvTransport` implementation is exactly this Python sidecar (the
model runtime lives in Python/CUDA, not in Rust): the broker hands over a context id + model tag, and
the sidecar produces/consumes the KV blob measured here. The Rust side owns the **policy and
fallback**; this sidecar owns the **tensor transport**. That split is intentional and documented in
the paper's limitations.

## Expected shape of the result (hypothesis, to be confirmed by your run)

- **Latency:** bridge prefill should beat recompute and the gap should *widen* with prefix length
  (recompute re-does O(prefix) attention; bridge does O(query)).
- **Cost:** `kv_vs_text_ratio` will be large (often 100x–1000x) — a KV cache is far bigger than the
  text. This is the reason TOAP gates KV-bridge behind same-model + long-shared-context and otherwise
  prefers `CtxRef`. The benchmark exists to find where (if ever) the latency win justifies the size.
