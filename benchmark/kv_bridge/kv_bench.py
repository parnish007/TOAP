#!/usr/bin/env python3
"""
KV-bridge benchmark: recompute (re-prefill prefix+query) vs KV-bridge (reuse transferred prefix KV).

Measures the things that decide whether TOAP's KV_BRIDGE is ever worth it:
  1) PREFILL LATENCY  — reusing the prefix KV skips its prefill (the COMPUTE upside).
  2) TRANSFER COST    — serialize + deserialize the KV (reported separately, never hidden).
  3) KV BYTE SIZE     — cache size vs the text it replaces (the honest storage/bandwidth cost).
  4) CORRECTNESS      — do greedy tokens match the recompute baseline EXACTLY? (must be lossless).

It sweeps shared-context lengths to show the crossover, repeats each cell, and is robust:
  * builds the prefix to an EXACT token count (no over-tokenization warnings);
  * clamps every length to the model's context window (no index errors);
  * tolerates CUDA OOM (skips that cell, keeps going);
  * GQA-aware byte accounting.

Usage:
  python kv_bench.py --model gpt2                       # auto picks valid lengths < 1024
  python kv_bench.py --model EleutherAI/pythia-410m --prefix-tokens 256 512 1024 2048
  python kv_bench.py --model Qwen/Qwen2.5-0.5B-Instruct # GQA model
  python kv_bench.py --model mistralai/Mistral-7B-Instruct-v0.3 --load-in-4bit

Outputs kv_bench_results.json (and prints a table). Send that JSON back to fold into the paper.
"""
from __future__ import annotations
import argparse
import json
import os
import sys

import torch

import kv_bridge as kvb


def build_prefix(tok, n_tokens, device):
    """Build a prefix of EXACTLY n_tokens by tokenizing a base passage once and tiling token IDs.
    Tiling at the token level avoids tokenizing a giant string (which triggers >max-length warnings)."""
    base = ("In a multi-agent system, the orchestrator stores a document once and shares it by "
            "reference. The payments service had no autoscaling and its connection pool saturated "
            "under a 5x traffic spike, causing an outage. ")
    unit = tok(base, return_tensors="pt").input_ids[0]          # 1-D token ids
    reps = (n_tokens // unit.numel()) + 1
    ids = unit.repeat(reps)[:n_tokens].unsqueeze(0).to(device)  # exactly n_tokens
    return ids


def build_query(tok, device):
    q = tok(" Summarize the incident in one sentence.", return_tensors="pt").input_ids.to(device)
    return q


def human(n):
    n = float(n)
    for unit in ["B", "KB", "MB", "GB"]:
        if abs(n) < 1024:
            return f"{n:.1f}{unit}"
        n /= 1024
    return f"{n:.1f}TB"


def default_lengths(limit, query_len, new_tokens):
    """Powers-of-two prefix lengths that fit the model context (prefix+query+new < limit)."""
    budget = limit - query_len - new_tokens - 4
    candidates = [128, 256, 512, 1024, 2048, 4096, 8192]
    valid = [c for c in candidates if c <= budget]
    return valid or [max(16, budget)]


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="gpt2")
    ap.add_argument("--prefix-tokens", type=int, nargs="+", default=None,
                    help="explicit prefix lengths; default = auto powers-of-two within context")
    ap.add_argument("--new-tokens", type=int, default=32)
    ap.add_argument("--repeats", type=int, default=5)
    ap.add_argument("--load-in-4bit", action="store_true")
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                  "kv_bench_results.json"))
    a = ap.parse_args()

    tok, model, device = kvb.load(a.model, load_in_4bit=a.load_in_4bit)
    gpu = torch.cuda.get_device_name(0) if device == "cuda" else "cpu"
    dtype = str(next(model.parameters()).dtype)
    limit = kvb.model_context_limit(tok, model)
    query = build_query(tok, device)
    query_len = query.shape[1]

    lengths = a.prefix_tokens or default_lengths(limit, query_len, a.new_tokens)
    # clamp: every length must leave room for query + new tokens within the context window
    max_prefix = limit - query_len - a.new_tokens - 4
    lengths = sorted({min(n, max_prefix) for n in lengths if n > 0 and min(n, max_prefix) > 0})

    print(f"model={a.model}  device={device} ({gpu})  dtype={dtype}  ctx_limit={limit}  "
          f"new_tokens={a.new_tokens}  repeats={a.repeats}  4bit={a.load_in_4bit}")
    if not lengths:
        print("No valid prefix lengths fit this model's context window.", file=sys.stderr)
        sys.exit(2)
    print(f"prefix lengths: {lengths}\n")

    rows = []
    hdr = (f"{'prefix':>7} {'kv_size':>9} {'kv/text':>8} {'recomp_ms':>10} {'resident_ms':>11} "
           f"{'xfer_ms':>8} {'sp_resident':>11} {'sp_xnode':>9} {'match':>6}")
    print(hdr); print("-" * len(hdr))

    for n in lengths:
        try:
            prefix = kvb.build_prefix(tok, n, device) if hasattr(kvb, "build_prefix") \
                else build_prefix(tok, n, device)
            actual_n = prefix.shape[1]
            text_bytes = len(tok.decode(prefix[0]).encode("utf-8"))

            kv = kvb.extract_kv(model, prefix)
            kv_size = kvb.kv_byte_size(kv)
            shape = kvb.kv_shape_info(kv)
            blob = kvb.serialize_kv(kv)
            factory = kvb.resident_cache_factory(kv, device)

            # Three independent, warmed-up, CUDA-event-timed measurements:
            rec_med, rec_mean, rec_std, _ = kvb.measure_prefill_recompute(
                model, prefix, query, device, iters=a.repeats)
            res_med, res_mean, res_std, _ = kvb.measure_prefill_bridge_resident(
                model, factory, query, device, iters=a.repeats)
            xf_med, xf_mean, xf_std, _ = kvb.measure_transfer(blob, device)
            match, _toks = kvb.check_lossless(model, prefix, query, blob, device, new_tokens=a.new_tokens)

            sp_resident = rec_med / res_med if res_med else float("nan")     # pure compute win (co-located)
            sp_xnode = rec_med / (res_med + xf_med) if (res_med + xf_med) else float("nan")  # with transfer tax
            ratio = kv_size / text_bytes if text_bytes else None
            print(f"{actual_n:>7} {human(kv_size):>9} {(ratio or 0):>7.0f}x {rec_med:>10.2f} "
                  f"{res_med:>11.2f} {xf_med:>8.2f} {sp_resident:>10.2f}x {sp_xnode:>8.2f}x "
                  f"{str(match):>6}")
            rows.append({
                "prefix_tokens": actual_n, "kv_bytes": kv_size,
                "kv_bytes_per_token": kv_size / actual_n, "kv_shape": shape,
                "text_bytes": text_bytes, "kv_vs_text_ratio": ratio,
                "recompute_prefill_ms_median": rec_med, "recompute_prefill_ms_std": rec_std,
                "bridge_resident_prefill_ms_median": res_med, "bridge_resident_prefill_ms_std": res_std,
                "transfer_ms_median": xf_med, "transfer_ms_std": xf_std,
                "speedup_resident": sp_resident,      # co-located: pure compute win, no transfer
                "speedup_crossnode": sp_xnode,        # includes serialize/deserialize transfer tax
                "outputs_match": match,
            })
            del kv, blob, factory
            if device == "cuda":
                torch.cuda.empty_cache()
        except torch.cuda.OutOfMemoryError:
            print(f"{n:>7}  (skipped: CUDA OOM)")
            if device == "cuda":
                torch.cuda.empty_cache()
        except Exception as e:                       # keep the sweep going; record the failure
            print(f"{n:>7}  (skipped: {type(e).__name__}: {e})")
            if device == "cuda":
                torch.cuda.empty_cache()

    result = {
        "model": a.model, "device": device, "gpu": gpu, "dtype": dtype,
        "context_limit": limit, "new_tokens": a.new_tokens, "iters_per_cell": a.repeats,
        "load_in_4bit": a.load_in_4bit,
        "note": ("Warmed-up CUDA-event timing of PREFILL ONLY (decode excluded). speedup_resident = "
                 "recompute / query-prefill-on-resident-KV (co-located, pure compute win). "
                 "speedup_crossnode = recompute / (query-prefill + KV deserialize) (with transfer tax). "
                 "prefix-reuse only, absolute positions preserved. outputs_match = greedy losslessness."),
        "rows": rows,
    }
    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2)
    print(f"\n[written] {a.out}")

    if not rows:
        print("ERROR: produced ZERO rows — every cell was skipped (likely a transformers/cache API "
              "mismatch). The result JSON is empty; do not use it.", file=sys.stderr)
        sys.exit(3)
    if not all(r["outputs_match"] for r in rows):
        print("WARNING: some KV-bridge outputs did NOT match recompute — investigate before claiming "
              "losslessness.", file=sys.stderr)


if __name__ == "__main__":
    main()
