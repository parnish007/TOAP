#!/usr/bin/env python3
"""Figures for the n>1 controlled writer experiment. Run: python paper/figures/gen_writer_figs.py"""
import json, os
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

HERE = os.path.dirname(os.path.abspath(__file__))
RES = os.path.join(HERE, "..", "..", "benchmark", "ab_study", "writer_results.json")
d = json.load(open(RES))
cells = d["cells"]
models = ["haiku", "sonnet", "opus"]
arms = ["naive", "summary", "toap"]
colors = {"naive": "#b0413e", "summary": "#9aa0a6", "toap": "#2f6f4f"}
labels = {"naive": "Naive (full transcript)", "summary": "Summarizing baseline", "toap": "TOAP (reference)"}

# Figure A: total tokens per model x arm, with min-max error bars
x = range(len(models)); w = 0.26
fig, ax = plt.subplots(figsize=(7, 4.2))
for j, arm in enumerate(arms):
    means = [cells[f"{m}/{arm}"]["tot_tok_mean"] for m in models]
    lo = [cells[f"{m}/{arm}"]["tot_tok_mean"] - cells[f"{m}/{arm}"]["tot_tok_min"] for m in models]
    hi = [cells[f"{m}/{arm}"]["tot_tok_max"] - cells[f"{m}/{arm}"]["tot_tok_mean"] for m in models]
    pos = [i + (j - 1) * w for i in x]
    ax.bar(pos, means, w, yerr=[lo, hi], capsize=3, color=colors[arm], label=labels[arm])
ax.set_xticks(list(x)); ax.set_xticklabels([m.capitalize() for m in models])
ax.set_ylabel("Total tokens (prompt+output, tiktoken)")
ax.set_title("Writer-stage tokens by context strategy", fontsize=10)
ax.legend(fontsize=8)
fig.tight_layout(); fig.savefig(os.path.join(HERE, "fig_threearm.png"), dpi=150)
plt.close(fig)

# Figure B: accuracy parity (all cells 4/4) -- the retraction of the n=1 regression
fig, ax = plt.subplots(figsize=(7, 3.6))
for j, arm in enumerate(arms):
    accs = [cells[f"{m}/{arm}"]["acc_mean"] for m in models]
    pos = [i + (j - 1) * w for i in x]
    ax.bar(pos, accs, w, color=colors[arm], label=labels[arm])
ax.axhline(4, ls="--", color="gray", lw=0.8)
ax.set_xticks(list(x)); ax.set_xticklabels([m.capitalize() for m in models])
ax.set_ylabel("Rubric themes covered (/4)")
ax.set_ylim(0, 4.6)
ax.set_title("Accuracy parity: all 9 cells score 4/4", fontsize=10)
ax.legend(fontsize=8, loc="lower right")
fig.tight_layout(); fig.savefig(os.path.join(HERE, "fig_parity.png"), dpi=150)
plt.close(fig)

red = d.get("reduction_vs_naive", {})
print("wrote fig_threearm.png, fig_parity.png")
print("reductions vs naive:", json.dumps(red))
