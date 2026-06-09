#!/usr/bin/env python3
"""
KV-bridge benchmark: recompute (re-prefill prefix+query) vs KV-bridge (reuse transferred prefix KV).

Measures the three things that decide whether TOAP's KV_BRIDGE is ever worth it:
  1) PREFILL LATENCY  — does reusing the prefix KV actually skip work? (the upside)
  2) KV BYTE SIZE      — how big is the cache vs the text it replaces? (the honest cost)
  3) CORRECTNESS       — do greedy tokens match the recompute baseline exactly? (must be lossless)

It sweeps several shared-context lengths so you can see the crossover, and repeats each cell to
report mean/min/max. Honest by construction: if KV-bridge does not beat recompute, the numbers say so.

Usage (RTX 3050 / Colab):
  pip install torch transformers
  python kv_bench.py --model gpt2 --prefix-tokens 128 256 512 1024 --new-tokens 32 --repeats 5
  python kv_bench.py --model EleutherAI/pythia-410m --prefix-tokens 256 512 1024 2048

Outputs kv_bench_results.json (and prints a table). Send that JSON back to fold into the paper.
"""
from __future__ import annotations
import argparse
import json
import os
import statistics
import sys

import torch

import kv_bridge as kvb


def build_prefix(tok, n_tokens, device):
    """A realistic-ish shared context padded/truncated to exactly n_tokens."""
    base = ("In a multi-agent system, the orchestrator stores a document once and shares it by "
            "reference. The payments service had no autoscaling and its connection pool saturated "
            "under a 5x traffic spike, causing an outage. ")
    ids = tok(base * 64, return_tensors="pt").input_ids[:, :n_tokens].to(device)
    return ids


def build_query(tok, device):
    return tok(" Summarize the incident in one sentence.", return_tensors="pt").input_ids.to(device)


def human(n):
    for unit in ["B", "KB", "MB", "GB"]:
        if abs(n) < 1024:
            return f"{n:.1f}{unit}"
        n /= 1024
    return f"{n:.1f}TB"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--model", default="gpt2")
    ap.add_argument("--prefix-tokens", type=int, nargs="+", default=[128, 256, 512, 1024])
    ap.add_argument("--new-tokens", type=int, default=32)
    ap.add_argument("--repeats", type=int, default=5)
    ap.add_argument("--out", default=os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                                  "kv_bench_results.json"))
    a = ap.parse_args()

    tok, model, device = kvb.load(a.model)
    gpu = torch.cuda.get_device_name(0) if device == "cuda" else "cpu"
    dtype = str(next(model.parameters()).dtype)
    print(f"model={a.model}  device={device} ({gpu})  dtype={dtype}  new_tokens={a.new_tokens}  "
          f"repeats={a.repeats}\n")

    query = build_query(tok, device)
    rows = []
    hdr = f"{'prefix_tok':>10} {'kv_size':>10} {'text_bytes':>10} {'recompute_ms':>13} {'bridge_ms':>11} {'speedup':>8} {'match':>6}"
    print(hdr); print("-" * len(hdr))

    for n in a.prefix_tokens:
        prefix = build_prefix(tok, n, device)
        actual_n = prefix.shape[1]
        text_bytes = len(tok.decode(prefix[0]).encode("utf-8"))

        kv = kvb.extract_kv(model, prefix)
        kv_size = kvb.kv_byte_size(kv)
        blob = kvb.serialize_kv(kv)

        rec_pref, br_pref, matches = [], [], []
        for _ in range(a.repeats):
            r = kvb.generate_recompute(model, prefix, query, a.new_tokens, device)
            b = kvb.generate_kv_bridge(model, blob, query, a.new_tokens, device)
            rec_pref.append(r.prefill_ms)
            br_pref.append(b.prefill_ms)
            matches.append(r.tokens == b.tokens)

        rec_m = statistics.mean(rec_pref)
        br_m = statistics.mean(br_pref)
        speedup = rec_m / br_m if br_m else float("nan")
        all_match = all(matches)
        print(f"{actual_n:>10} {human(kv_size):>10} {human(text_bytes):>10} "
              f"{rec_m:>13.2f} {br_m:>11.2f} {speedup:>7.2f}x {str(all_match):>6}")
        rows.append({
            "prefix_tokens": actual_n,
            "kv_bytes": kv_size,
            "kv_bytes_per_token": kv_size / actual_n,
            "text_bytes": text_bytes,
            "kv_vs_text_ratio": kv_size / text_bytes if text_bytes else None,
            "recompute_prefill_ms_mean": rec_m,
            "recompute_prefill_ms_min": min(rec_pref),
            "recompute_prefill_ms_max": max(rec_pref),
            "bridge_prefill_ms_mean": br_m,
            "bridge_prefill_ms_min": min(br_pref),
            "bridge_prefill_ms_max": max(br_pref),
            "prefill_speedup": speedup,
            "outputs_match": all_match,
        })

    result = {
        "model": a.model, "device": device, "gpu": gpu, "dtype": dtype,
        "new_tokens": a.new_tokens, "repeats": a.repeats,
        "transformers_note": "prefix-reuse only (positions preserved); no cross-position splicing",
        "rows": rows,
    }
    with open(a.out, "w", encoding="utf-8") as f:
        json.dump(result, f, indent=2)
    print(f"\n[written] {a.out}")

    if not all(r["outputs_match"] for r in rows):
        print("WARNING: some KV-bridge outputs did NOT match recompute — investigate before claiming "
              "losslessness.", file=sys.stderr)


if __name__ == "__main__":
    main()
