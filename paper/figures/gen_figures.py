#!/usr/bin/env python3
"""Generate paper figures from the real multi-model results. Run: python paper/figures/gen_figures.py"""
import json, os
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt

HERE = os.path.dirname(os.path.abspath(__file__))
RES = os.path.join(HERE, "..", "..", "benchmark", "ab_study", "multimodel_results.json")
with open(RES, encoding="utf-8") as f:
    data = json.load(f)
rows = data["rows"]
models = [r["model"] for r in rows]
base = [r["down_base_tok"] for r in rows]
toap = [r["down_toap_tok"] for r in rows]
red = [r["reduction"] for r in rows]
acc_b = [r["acc_base"] for r in rows]
acc_t = [r["acc_toap"] for r in rows]

x = range(len(models))
w = 0.38

# Figure 1: downstream tokens baseline vs TOAP
fig, ax = plt.subplots(figsize=(6, 4))
ax.bar([i - w/2 for i in x], base, w, label="Baseline (full context)", color="#b0413e")
ax.bar([i + w/2 for i in x], toap, w, label="TOAP (reference-minimized)", color="#2f6f4f")
for i, r in enumerate(red):
    ax.text(i, max(base[i], toap[i]) + 15, f"{r:.2f}x", ha="center", fontsize=10, fontweight="bold")
ax.set_xticks(list(x)); ax.set_xticklabels([m.capitalize() for m in models])
ax.set_ylabel("Downstream LLM tokens (prompt+output, tiktoken)")
ax.set_title("Downstream token cost: Baseline vs TOAP (scenario 1)")
ax.legend(); ax.set_ylim(0, max(base) * 1.2)
fig.tight_layout(); fig.savefig(os.path.join(HERE, "fig_tokens.png"), dpi=150)
plt.close(fig)

# Figure 2: accuracy baseline vs TOAP per model
fig, ax = plt.subplots(figsize=(6, 4))
ax.bar([i - w/2 for i in x], acc_b, w, label="Baseline", color="#b0413e")
ax.bar([i + w/2 for i in x], acc_t, w, label="TOAP", color="#2f6f4f")
ax.axhline(4, ls="--", color="gray", lw=0.8)
for i in x:
    ax.text(i + w/2, acc_t[i] + 0.05, str(acc_t[i]), ha="center", fontsize=10)
    ax.text(i - w/2, acc_b[i] + 0.05, str(acc_b[i]), ha="center", fontsize=10)
ax.set_xticks(list(x)); ax.set_xticklabels([m.capitalize() for m in models])
ax.set_ylabel("Rubric items covered (independent judge, /4)")
ax.set_title("Accuracy: Baseline vs TOAP (Haiku drops under minimization)")
ax.set_ylim(0, 4.6); ax.legend(loc="lower right")
fig.tight_layout(); fig.savefig(os.path.join(HERE, "fig_accuracy.png"), dpi=150)
plt.close(fig)

print("wrote fig_tokens.png, fig_accuracy.png to", HERE)
