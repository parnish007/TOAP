#!/usr/bin/env python3
"""Generate the REAL KV-bridge results figure from a committed kv_bench_all.json.
Overwrites fig_kv_bridge.png. Run AFTER the Colab run:
    python paper/figures/gen_kv_figure.py benchmark/kv_bridge/kv_bench_all.json
"""
import json
import os
import sys
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

HERE = os.path.dirname(os.path.abspath(__file__))
path = sys.argv[1] if len(sys.argv) > 1 else os.path.join(
    HERE, "..", "..", "benchmark", "kv_bridge", "kv_bench_all.json")
data = json.load(open(path, encoding="utf-8"))

fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(10, 3.8))
for run in data["runs"]:
    rows = sorted([r for r in run["rows"]], key=lambda r: r["prefix_tokens"])
    if not rows:
        continue
    xs = [r["prefix_tokens"] for r in rows]
    label = run["model"].split("/")[-1]
    ax1.plot(xs, [r["speedup_resident"] for r in rows], marker="o", label=label)
    ax2.plot(xs, [r["kv_vs_text_ratio"] for r in rows], marker="s", label=label)

ax1.axhline(1.0, ls="--", color="gray", lw=0.8)
ax1.set_xlabel("shared prefix length (tokens)"); ax1.set_ylabel("prefill speedup (resident, x)")
ax1.set_title("KV-bridge compute win (>1 = faster than recompute)", fontsize=10)
ax1.set_xscale("log", base=2); ax1.legend(fontsize=7)
ax2.set_xlabel("shared prefix length (tokens)"); ax2.set_ylabel("KV size / text size (x)")
ax2.set_title("Honest cost: KV cache vs the text it replaces", fontsize=10)
ax2.set_xscale("log", base=2); ax2.set_yscale("log"); ax2.legend(fontsize=7)
fig.tight_layout()
fig.savefig(os.path.join(HERE, "fig_kv_bridge.png"), dpi=150)
print("wrote fig_kv_bridge.png (real data)")
