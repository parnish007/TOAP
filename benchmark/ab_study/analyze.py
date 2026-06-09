#!/usr/bin/env python3
"""
Aggregate the A/B study across scenarios. Reads run records (each = one scenario, both arms, real
subagent calls with verbatim prompts/outputs + runtime subagent_tokens), and reports:
  * a REPRODUCIBLE metric: tiktoken(prompt+output) per stage (anyone can recompute from the records);
  * the RUNTIME metric: subagent_tokens net of the per-record calibration overhead (real, n=1/sample);
  * accuracy parsed from the independent judge's output;
  * per-scenario reductions and the aggregate mean across scenarios.

Run: python benchmark/ab_study/analyze.py
"""
import json
import os
import re
import tiktoken

HERE = os.path.dirname(os.path.abspath(__file__))
enc = tiktoken.get_encoding("cl100k_base")
def tk(s): return len(enc.encode(s))

# Run records. scenario_01 was recorded before this folder existed; we read it in place.
RUN_FILES = [
    os.path.join(HERE, "..", "runs", "run_2026-05-30_subagent_pipeline.json"),  # scenario_01 incident
    os.path.join(HERE, "runs", "scenario_02_support.json"),
]


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def calls_by_id(rec):
    return {c["id"]: c for c in rec["calls"]}


def parse_judge(text):
    # expects lines like 'A: 4/4 ; missing: none'
    out = {}
    for label in ("A", "B"):
        m = re.search(rf"{label}:\s*(\d+)\s*/\s*(\d+)", text)
        if m:
            out[label] = (int(m.group(1)), int(m.group(2)))
    return out


def reductions(rec):
    c = calls_by_id(rec)
    ovh = rec["overhead_calibration_subagent_tokens"]

    def tkpo(cid):
        return tk(c[cid]["prompt"]) + tk(c[cid]["output"])

    def net(cid):
        return c[cid]["subagent_tokens"] - ovh

    full_b = ["extractor", "analyst_baseline", "writer_baseline"]
    full_t = ["extractor", "analyst_toap", "writer_toap"]
    down_b = ["analyst_baseline", "writer_baseline"]
    down_t = ["analyst_toap", "writer_toap"]

    tk_pipe = (sum(map(tkpo, full_b)), sum(map(tkpo, full_t)))
    tk_down = (sum(map(tkpo, down_b)), sum(map(tkpo, down_t)))
    rt_pipe = (sum(map(net, full_b)), sum(map(net, full_t)))
    rt_down = (sum(map(net, down_b)), sum(map(net, down_t)))

    acc = parse_judge(c["judge"]["output"]) if "judge" in c else {}
    return {"tk_pipe": tk_pipe, "tk_down": tk_down, "rt_pipe": rt_pipe, "rt_down": rt_down, "acc": acc}


def ratio(pair):
    b, t = pair
    return b / t if t else float("nan")


def main():
    print("A/B study — reproducible tiktoken metric (and runtime cross-reference)\n")
    print(f"{'scenario':24} {'tk pipe':>10} {'tk down':>10} {'rt pipe':>10} {'rt down':>10} {'acc B/A':>9}")
    print("-" * 78)
    agg = {"tk_pipe": [], "tk_down": [], "rt_pipe": [], "rt_down": []}
    for path in RUN_FILES:
        if not os.path.exists(path):
            print(f"(missing: {os.path.basename(path)})")
            continue
        rec = load(path)
        r = reductions(rec)
        for k in agg:
            agg[k].append(ratio(r[k]))
        a = r["acc"]
        acc_str = ""
        if "A" in a and "B" in a:
            acc_str = f"{a['B'][0]}/{a['B'][1]} vs {a['A'][0]}/{a['A'][1]}"
        sid = rec.get("scenario", os.path.basename(path))[:24]
        print(f"{sid:24} {ratio(r['tk_pipe']):>9.2f}x {ratio(r['tk_down']):>9.2f}x "
              f"{ratio(r['rt_pipe']):>9.2f}x {ratio(r['rt_down']):>9.2f}x {acc_str:>9}")

    n = len(agg["tk_pipe"])
    if n:
        def mean(xs): return sum(xs) / len(xs)
        def spread(xs): return (min(xs), max(xs))
        print("\n=== aggregate (n = %d scenarios) ===" % n)
        for k, label in [("tk_pipe", "tiktoken pipeline"), ("tk_down", "tiktoken downstream"),
                         ("rt_pipe", "runtime pipeline"), ("rt_down", "runtime downstream")]:
            lo, hi = spread(agg[k])
            print(f"  {label:22} mean {mean(agg[k]):.2f}x   range [{lo:.2f}x, {hi:.2f}x]")
        print("\nReproducible numbers = tiktoken (recompute from the committed run records).")
        print("Runtime numbers = real subagent usage, single-sample per call. n is still small;")
        print("more scenarios tighten the estimate. Accuracy: independent judge, themes covered.")


if __name__ == "__main__":
    main()
