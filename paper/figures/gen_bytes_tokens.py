#!/usr/bin/env python3
"""Bytes-vs-tokens figure: shows byte savings overstate token savings for symbolic opcodes.
Real tiktoken (cl100k) measurement. Run: python paper/figures/gen_bytes_tokens.py"""
import os
import matplotlib
matplotlib.use("Agg")
import matplotlib.pyplot as plt
import tiktoken

HERE = os.path.dirname(os.path.abspath(__file__))
enc = tiktoken.get_encoding("cl100k_base")
def tok(s): return len(enc.encode(s))
def byt(s): return len(s.encode("utf-8"))

pairs = [
    ("SUM(CTX:42)?max_words=150", "Please summarize document 42 in at most 150 words."),
    ("SUM(CTX:42)", "Summarize document 42."),
    ("SUM(CTX:42)?max_words=150|TONE:formal|FMT:bullets",
     "Summarize document 42 in at most 150 words, in a formal tone, as bullet points."),
    ("CLS(CTX:55)?labels=spam,ham", "Classify document 55 as either spam or ham."),
]
labels = ["ex1", "ex2", "ex3", "ex4"]
byte_save = []
tok_save = []
for toap, nl in pairs:
    byte_save.append((1 - byt(toap) / byt(nl)) * 100)
    tok_save.append((1 - tok(toap) / tok(nl)) * 100)

x = range(len(pairs))
w = 0.38
fig, ax = plt.subplots(figsize=(6.5, 4))
ax.bar([i - w/2 for i in x], byte_save, w, label="Byte saving (misleading)", color="#9aa0a6")
ax.bar([i + w/2 for i in x], tok_save, w, label="Token saving (what actually matters)", color="#2f6f4f")
for i in x:
    ax.text(i - w/2, byte_save[i] + 1, f"{byte_save[i]:.0f}%", ha="center", fontsize=9)
    ax.text(i + w/2, tok_save[i] + 1, f"{tok_save[i]:.0f}%", ha="center", fontsize=9)
ax.set_xticks(list(x)); ax.set_xticklabels(labels)
ax.set_ylabel("Reduction vs natural language (%)")
ax.set_title("Symbolic opcodes: bytes overstate the saving; tokens are modest")
ax.axhline(0, color="black", lw=0.6)
ax.legend(); ax.set_ylim(min(0, min(tok_save) - 5), max(byte_save) + 12)
fig.tight_layout(); fig.savefig(os.path.join(HERE, "fig_bytes_tokens.png"), dpi=150)
print("wrote fig_bytes_tokens.png ; byte_save=%s tok_save=%s" %
      ([round(b) for b in byte_save], [round(t) for t in tok_save]))
