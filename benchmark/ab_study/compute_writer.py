import json, os, statistics, tiktoken
import writer_experiment as w
enc = tiktoken.get_encoding("cl100k_base")
def tk(s): return len(enc.encode(s))

pt = {a: tk(w.prompt(a)) for a in ("naive", "summary", "toap")}
runs = json.load(open(os.path.join(os.path.dirname(__file__), "runs", "writer_runs.json")))["samples"]

cells = {}
for r in runs:
    k = (r["model"], r["arm"])
    themes, score = w.covers(r["output"])
    out_tok = tk(r["output"])
    cells.setdefault(k, {"scores": [], "out_tok": [], "tot_tok": []})
    cells[k]["scores"].append(score)
    cells[k]["out_tok"].append(out_tok)
    cells[k]["tot_tok"].append(pt[r["arm"]] + out_tok)

out = {"prompt_tokens_per_arm": pt, "cells": {}}
for (m, a), c in cells.items():
    out["cells"][f"{m}/{a}"] = {
        "n": len(c["scores"]),
        "acc_mean": round(statistics.mean(c["scores"]), 2),
        "acc_min": min(c["scores"]), "acc_max": max(c["scores"]),
        "out_tok_mean": round(statistics.mean(c["out_tok"]), 1),
        "tot_tok_mean": round(statistics.mean(c["tot_tok"]), 1),
        "tot_tok_min": min(c["tot_tok"]), "tot_tok_max": max(c["tot_tok"]),
    }
# reductions vs naive per model (total prompt+output)
out["reduction_vs_naive"] = {}
for m in sorted({mm for (mm, _a) in cells}):
    base = statistics.mean(cells[(m, "naive")]["tot_tok"])
    out["reduction_vs_naive"][m] = {
        "summary": round(base / statistics.mean(cells[(m, "summary")]["tot_tok"]), 3),
        "toap": round(base / statistics.mean(cells[(m, "toap")]["tot_tok"]), 3),
    }
json.dump(out, open(os.path.join(os.path.dirname(__file__), "writer_results.json"), "w"), indent=2)
print("OK")
