#!/usr/bin/env python3
"""Build the KV-bridge results figure from a committed kv_bench_all.json.
Usage: python gen_kv_figure.py [path/to/kv_bench_all.json]

Three panels share one model legend: (a) resident speedup (co-located compute win),
(b) cross-node speedup (KV transfer charged), (c) KV/text size ratio. GQA models are solid,
MHA dashed, so the architecture split reads at a glance.
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
runs = json.load(open(path, encoding="utf-8"))["runs"]

# A fixed colour per model keeps the three panels consistent.
palette = ["#1f77b4", "#ff7f0e", "#2ca02c", "#d62728", "#7e3ff2", "#8c564b", "#e377c2"]

def kv_heads(run):
    rows = run.get("rows") or []
    return rows[0]["kv_shape"].get("kv_heads", 99) if rows else 99

fig, (ax1, ax2, ax3) = plt.subplots(1, 3, figsize=(13.5, 4.1))
handles, labels = [], []
for i, run in enumerate(runs):
    rows = sorted(run["rows"], key=lambda r: r["prefix_tokens"])
    if not rows:
        continue
    xs = [r["prefix_tokens"] for r in rows]
    gqa = kv_heads(run) <= 4
    color = palette[i % len(palette)]
    style = dict(color=color, linestyle="-" if gqa else "--", lw=2.0 if gqa else 1.4, marker="o", ms=4)
    (h,) = ax1.plot(xs, [r["speedup_resident"] for r in rows], **style)
    ax2.plot(xs, [r["speedup_crossnode"] for r in rows], **style)
    ax3.plot(xs, [r["kv_vs_text_ratio"] for r in rows], **style)
    handles.append(h)
    labels.append(run["model"].split("/")[-1] + ("  (GQA)" if gqa else ""))

for ax in (ax1, ax2):
    ax.axhline(1.0, ls=":", color="black", lw=1.0)
    ax.set_xscale("log", base=2)
    ax.set_yscale("log")
    ax.set_xlabel("shared prefix length (tokens)")
    ax.set_ylabel("prefill speedup ($\\times$)")
ax1.set_title("(a) Resident: co-located compute win", fontsize=10)
ax2.set_title("(b) Cross-node: KV transfer charged", fontsize=10)
ax3.set_xscale("log", base=2)
ax3.set_yscale("log")
ax3.set_xlabel("shared prefix length (tokens)")
ax3.set_ylabel("KV size / text size ($\\times$)")
ax3.set_title("(c) Cost: KV cache vs the text", fontsize=10)

# Lay the panels out in the top ~78% and reserve the bottom strip for the shared legend, so the
# legend never collides with the panels or the caption line.
fig.tight_layout(rect=(0, 0.20, 1, 1))
fig.legend(handles, labels, loc="lower center", ncol=4, fontsize=8.5,
           frameon=True, bbox_to_anchor=(0.5, 0.045))
fig.text(0.5, 0.005, "Solid = grouped-query attention; dashed = multi-head attention. "
         "Dotted line = break-even (1$\\times$).", ha="center", fontsize=8)
fig.savefig(os.path.join(HERE, "fig_kv_bridge.png"), dpi=150)
print("wrote fig_kv_bridge.png")
