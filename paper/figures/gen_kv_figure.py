#!/usr/bin/env python3
"""Generate the REAL KV-bridge results figure from a committed kv_bench_all.json.
Overwrites fig_kv_bridge.png. Run AFTER the Colab run:
    python paper/figures/gen_kv_figure.py benchmark/kv_bridge/kv_bench_all.json

Three panels: (a) resident speedup (co-located compute win), (b) cross-node speedup (with KV
transfer cost), (c) KV/text size ratio. GQA models are drawn solid, MHA dashed, so the
architecture split is visible at a glance.
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

# GQA models have far fewer kv_heads; detect from the data so the split is data-driven, not hard-coded.
def is_gqa(run):
    rows = run.get("rows") or []
    if not rows:
        return False
    return rows[0]["kv_shape"].get("kv_heads", 99) <= 4

fig, axes = plt.subplots(1, 3, figsize=(13.5, 3.9))
ax1, ax2, ax3 = axes
for run in data["runs"]:
    rows = sorted([r for r in run["rows"]], key=lambda r: r["prefix_tokens"])
    if not rows:
        continue
    xs = [r["prefix_tokens"] for r in rows]
    label = run["model"].split("/")[-1]
    style = "-" if is_gqa(run) else "--"
    lw = 2.0 if is_gqa(run) else 1.3
    ax1.plot(xs, [r["speedup_resident"] for r in rows], style, marker="o", lw=lw, label=label)
    ax2.plot(xs, [r["speedup_crossnode"] for r in rows], style, marker="^", lw=lw, label=label)
    ax3.plot(xs, [r["kv_vs_text_ratio"] for r in rows], style, marker="s", lw=lw, label=label)

for ax in (ax1, ax2):
    ax.axhline(1.0, ls=":", color="black", lw=1.0)
    ax.set_xscale("log", base=2); ax.set_yscale("log")
    ax.set_xlabel("shared prefix length (tokens)")

ax1.set_ylabel("prefill speedup (x)")
ax1.set_title("(a) Resident: co-located compute win", fontsize=10)
ax1.legend(fontsize=6.5, loc="upper left")
ax2.set_ylabel("prefill speedup (x)")
ax2.set_title("(b) Cross-node: incl. KV transfer cost", fontsize=10)
ax3.set_xscale("log", base=2); ax3.set_yscale("log")
ax3.set_xlabel("shared prefix length (tokens)"); ax3.set_ylabel("KV size / text size (x)")
ax3.set_title("(c) Honest cost: KV cache vs text", fontsize=10)
ax3.legend(fontsize=6.5, loc="center right")

fig.text(0.5, -0.02, "Solid = grouped-query attention (GQA);  dashed = multi-head attention (MHA).  "
         "Dotted line = break-even (1x).", ha="center", fontsize=8.5)
fig.tight_layout()
fig.savefig(os.path.join(HERE, "fig_kv_bridge.png"), dpi=150, bbox_inches="tight")
print("wrote fig_kv_bridge.png (real data, 3 panels)")
