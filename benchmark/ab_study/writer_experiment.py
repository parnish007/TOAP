#!/usr/bin/env python3
"""
n>1 writer experiment with a deterministic, bias-free accuracy scorer (resolves reviewer #1, #2, #4).

Controls: ONE fixed canonical upstream (DOC, FACTS, ANALYSIS that contains all 4 rubric themes), so the
only variables are {model, arm, sample}. Three arms:
  naive   = doc + facts + analysis            (transcript accumulation -- the weak baseline)
  summary = a short summary of the analysis   (a competent summarizing orchestrator -- strong baseline)
  toap    = analysis only                     (TOAP reference-minimization: the distilled slice)

Accuracy = deterministic theme coverage (keyword presence), NOT a model judge -> no same-family bias.
This script (a) builds the exact prompts, (b) scores collected outputs, (c) aggregates with ranges.
Outputs are collected from real subagents and stored in runs/writer_runs.json.

Run (after outputs collected): python benchmark/ab_study/writer_experiment.py
"""
import json, os, statistics, tiktoken
enc = tiktoken.get_encoding("cl100k_base")
def tok(s): return len(enc.encode(s))
HERE = os.path.dirname(os.path.abspath(__file__))
RUNS = os.path.join(HERE, "runs", "writer_runs.json")

DOC = ("Incident INC-4821. On March 3 at 14:02 UTC the payments API latency rose from 120ms to 9.4s "
       "and the error rate hit 38% for 47 minutes. The trigger was a marketing campaign that drove "
       "roughly 5x normal traffic to the checkout endpoint. The payments service runs 4 fixed pods "
       "with no autoscaling. The database connection pool (max 20) saturated; the queue had no "
       "backpressure, so memory grew and 2 pods OOM-restarted. A manual scale-up to 12 pods at 14:49 "
       "restored service. Revenue impact ~42,000 dollars. Load-tested only at 1x. No alert for "
       "connection-pool saturation; first signal was customer complaints. No deployment occurred.")
FACTS = ("5x campaign traffic; 4 fixed pods, no autoscaling; DB connection pool (max 20) saturated; "
         "no backpressure; 2 pods OOM; manual scale to 12 pods; ~$42,000 loss; load-tested only 1x; "
         "no saturation alert; first signal customer complaints.")
# Canonical analysis: explicitly contains all four rubric themes.
ANALYSIS = ("Root cause: capacity/architecture failure, not code. A 5x spike hit a statically-"
            "provisioned service (4 fixed pods, no autoscaling) whose DB connection pool (max 20) "
            "saturated; lack of backpressure let queues grow, causing memory growth and OOM restarts. "
            "Detection failed: no saturation alert. Severity: High (47 min, 38% errors, ~$42,000). "
            "Actions: add horizontal autoscaling and right-size the DB pool; add backpressure/load-"
            "shedding and request timeouts; alert on pool saturation, latency, error rate; load-test "
            "at realistic peak (5x+) before campaigns.")
# SUMMARY is generated once by a subagent (lossy, short) and pasted here for the summary arm.
# SUMMARY: generated once by a Sonnet orchestrator subagent (agent 5e... ), fixed for all runs.
SUMMARY = ("Payments outage (INC-4821): A 5x traffic spike overwhelmed a non-autoscaled service with a "
           "saturated DB connection pool, causing a 47-minute, ~$42K outage. Recommended fixes: add "
           "autoscaling, right-size the pool with backpressure, implement saturation alerting, and "
           "load-test at realistic peak traffic before campaigns.")

I_WRITE = ("You are a WRITER agent in a multi-agent pipeline. Do not use any tools. Do not read files "
           "or search. Respond directly.\n\nProduce the top prioritized action items as a short "
           "recommendation. At most 120 words. Output just the recommendation.")

def prompt(arm):
    if arm == "naive":
        return f'{I_WRITE}\n\nDocument:\n"{DOC}"\n\nExtracted facts:\n"{FACTS}"\n\nAnalysis:\n"{ANALYSIS}"'
    if arm == "summary":
        return f'{I_WRITE}\n\nSummary of analysis:\n"{SUMMARY}"'
    if arm == "toap":
        return f'{I_WRITE}\n\nAnalysis:\n"{ANALYSIS}"'
    raise ValueError(arm)

# --- Deterministic, bias-free theme scorer (the 4 rubric items) ---------------------------------
def covers(text):
    t = text.lower()
    autoscale = any(k in t for k in ["autoscal", "hpa", "horizontal pod"])
    pool = any(k in t for k in ["connection pool", "db pool", "pool size", "pool", "connections"])
    backpressure = any(k in t for k in ["backpressure", "back pressure", "load-shed", "load shed",
                                        "shedding", "circuit breaker", "queue limit", "bounded queue",
                                        "timeout", "rate limit", "throttl"])
    pool_bp = pool and backpressure
    alert = any(k in t for k in ["alert", "monitor", "observability", "paged", "saturation alert",
                                 "detection"])
    loadtest = any(k in t for k in ["load test", "load-test", "load testing", "stress test"])
    themes = {"autoscale": autoscale, "pool+backpressure": pool_bp, "alert": alert, "loadtest": loadtest}
    return themes, sum(themes.values())

def prompt_tokens():
    return {a: tok(prompt(a)) for a in ("naive", "summary", "toap")}

def analyze():
    if not os.path.exists(RUNS):
        print("no runs yet. prompt token sizes:", prompt_tokens()); return
    with open(RUNS, encoding="utf-8") as f:
        runs = json.load(f)["samples"]  # list of {model, arm, output, runtime_tokens}
    pt = prompt_tokens()
    cells = {}
    for r in runs:
        key = (r["model"], r["arm"])
        themes, score = covers(r["output"])
        total_tok = pt[r["arm"]] + tok(r["output"])
        cells.setdefault(key, {"scores": [], "toks": []})
        cells[key]["scores"].append(score)
        cells[key]["toks"].append(total_tok)
    print(f"{'model':8} {'arm':8} {'n':>2} {'acc mean/4':>10} {'acc range':>10} {'tok mean':>9} {'tok range':>13}")
    print("-" * 70)
    for (model, arm) in sorted(cells):
        c = cells[(model, arm)]
        n = len(c["scores"]); sc = c["scores"]; tks = c["toks"]
        rng = f"{min(sc)}-{max(sc)}"
        trng = f"{min(tks)}-{max(tks)}"
        print(f"{model:8} {arm:8} {n:>2} {statistics.mean(sc):>10.2f} {rng:>10} "
              f"{statistics.mean(tks):>9.0f} {trng:>13}")
    # token reduction vs naive, per model (mean)
    print("\nToken reduction vs naive (downstream writer prompt+output), per model:")
    for model in sorted({m for (m, _a) in cells}):
        base = statistics.mean(cells[(model, "naive")]["toks"]) if (model, "naive") in cells else None
        for arm in ("summary", "toap"):
            if (model, arm) in cells and base:
                red = base / statistics.mean(cells[(model, arm)]["toks"])
                print(f"  {model:8} {arm:8} {red:.2f}x")

if __name__ == "__main__":
    analyze()
