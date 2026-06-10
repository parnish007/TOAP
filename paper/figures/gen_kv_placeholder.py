#!/usr/bin/env python3
"""Generate a clean placeholder for the KV-bridge results figure, so the paper layout is final before
the real Colab data arrives. When kv_bench_all.json is available, run paper/figures/gen_kv_figure.py
(below) to overwrite fig_kv_bridge.png with the real plot. Run: python gen_kv_placeholder.py"""
import os
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

HERE = os.path.dirname(os.path.abspath(__file__))
fig, (ax1, ax2) = plt.subplots(1, 2, figsize=(9, 3.4))
for ax, title, ylab in [
    (ax1, "Prefill speedup vs prefix length", "speedup (resident, x)"),
    (ax2, "KV size / text size", "ratio (x)"),
]:
    ax.text(0.5, 0.5, "awaiting\nbenchmark run\n(T4 / Colab)", ha="center", va="center",
            fontsize=12, color="#888", transform=ax.transAxes,
            bbox=dict(boxstyle="round", fc="#f5f5f5", ec="#bbb"))
    ax.set_title(title, fontsize=10)
    ax.set_xlabel("shared prefix length (tokens)")
    ax.set_ylabel(ylab)
    ax.set_xticks([]); ax.set_yticks([])
fig.tight_layout()
fig.savefig(os.path.join(HERE, "fig_kv_bridge.png"), dpi=150)
print("wrote fig_kv_bridge.png (placeholder)")
