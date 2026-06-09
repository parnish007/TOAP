#!/usr/bin/env python3
"""
Cross-check for the real subagent run. The runtime `subagent_tokens` are NOT re-derivable
(nondeterministic, opaque total). But the prompts and outputs are stored verbatim in
runs/run_2026-05-30_subagent_pipeline.json, so ANYONE can re-tokenize them with tiktoken and verify
the prompt/output sizes that drive the result. This script does exactly that and prints the
runtime numbers next to the reproducible tiktoken numbers, so you can judge for yourself.

Run: python benchmark/crosscheck.py
"""
import json
import os
import tiktoken

HERE = os.path.dirname(os.path.abspath(__file__))
REC = os.path.join(HERE, "runs", "run_2026-05-30_subagent_pipeline.json")
enc = tiktoken.get_encoding("cl100k_base")
def tk(s): return len(enc.encode(s))

with open(REC, encoding="utf-8") as f:
    rec = json.load(f)

overhead = rec["overhead_calibration_subagent_tokens"]
calls = {c["id"]: c for c in rec["calls"]}

print("Model-independent cross-check (tiktoken cl100k) vs runtime-reported usage")
print(f"(calibration overhead = {overhead} subagent_tokens)\n")
hdr = f"{'call':18} {'runtime_tok':>11} {'net(-ovh)':>10} {'tk(prompt)':>10} {'tk(out)':>8} {'tk(p+o)':>8}"
print(hdr)
print("-" * len(hdr))
for c in rec["calls"]:
    rt = c["subagent_tokens"]
    net = rt - overhead
    pt, ot = tk(c["prompt"]), tk(c["output"])
    print(f"{c['id']:18} {rt:>11} {net:>10} {pt:>10} {ot:>8} {pt+ot:>8}")

def pipe(ids):
    return sum(tk(calls[i]["prompt"]) + tk(calls[i]["output"]) for i in ids)

def pipe_net(ids):
    return sum(calls[i]["subagent_tokens"] - overhead for i in ids)

base_full = ["extractor", "analyst_baseline", "writer_baseline"]
toap_full = ["extractor", "analyst_toap", "writer_toap"]
base_down = ["analyst_baseline", "writer_baseline"]
toap_down = ["analyst_toap", "writer_toap"]

print("\n=== REPRODUCIBLE (tiktoken prompt+output) — verify this yourself ===")
b, t = pipe(base_full), pipe(toap_full)
print(f"pipeline  baseline={b}  toap={t}  reduction={b/t:.2f}x")
b, t = pipe(base_down), pipe(toap_down)
print(f"downstream baseline={b}  toap={t}  reduction={b/t:.2f}x")

print("\n=== RUNTIME-reported (net of overhead) — real but single-sample, not re-derivable ===")
b, t = pipe_net(base_full), pipe_net(toap_full)
print(f"pipeline  baseline={b}  toap={t}  reduction={b/t:.2f}x")
b, t = pipe_net(base_down), pipe_net(toap_down)
print(f"downstream baseline={b}  toap={t}  reduction={b/t:.2f}x")

print("\nNote: tiktoken sees only prompt+output text (reproducible). Runtime 'net' also includes the")
print("model's hidden reasoning tokens, so absolute numbers differ; what matters is that both agree")
print("on DIRECTION and rough magnitude. Writer-baseline used condensed (not verbatim) upstream text,")
print("so the true full-transcript baseline would be larger -> TOAP advantage is understated here.")
