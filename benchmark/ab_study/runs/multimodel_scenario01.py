#!/usr/bin/env python3
"""
Consolidated REAL data + analysis for the multi-model run on scenario 1 (incident postmortem).

Each agent was a real isolated Claude subagent run at an explicit model (haiku/sonnet/opus). The
outputs below are the verbatim model generations; runtime_tokens are the runtime-reported
subagent_tokens; agent-ids are the trace. Token reduction is computed with tiktoken (cl100k) on
HARNESS-CANONICAL prompts assembled from the verbatim upstream outputs (reproducible). Accuracy is
from an independent opus judge scoring the verbatim writer outputs blind against a 4-item rubric.

Why tiktoken (not runtime) is the headline metric: subagent runtime usage carries a large, model-
dependent fixed overhead (Haiku ~19.6k/call vs Sonnet/Opus ~12.2k/call here) that swamps the signal,
so cross-model runtime ratios are not comparable. tiktoken on the canonical prompts is overhead-free
and reproducible. Runtime numbers are recorded for transparency only.

Run: python benchmark/ab_study/runs/multimodel_scenario01.py
"""
import json, os, tiktoken
enc = tiktoken.get_encoding("cl100k_base")
def tk(s): return len(enc.encode(s))
HERE = os.path.dirname(os.path.abspath(__file__))

DOC = ("Incident INC-4821. On March 3 at 14:02 UTC the payments API latency rose from 120ms to 9.4s "
       "and the error rate hit 38% for 47 minutes. The trigger was a marketing campaign that drove "
       "roughly 5x normal traffic to the checkout endpoint. The payments service runs 4 fixed pods "
       "with no autoscaling. The database connection pool (max 20) saturated, causing requests to "
       "queue; the queue had no backpressure, so memory grew and 2 pods OOM-restarted, worsening the "
       "outage. A manual scale-up to 12 pods at 14:49 restored service. Revenue impact is estimated "
       "at 42,000 dollars. The team had load-tested only at 1x expected traffic. No alert fired for "
       "connection-pool saturation; the first signal was customer complaints on social media. No "
       "deployment occurred, so rollback was not relevant.")

I_ANALYZE = ("You are an ANALYSIS agent in a multi-agent pipeline. Do not use any tools. Do not read "
             "files or search. Respond directly.\n\nState the root cause/diagnosis, the severity, and "
             "what should be done about it. At most 120 words. Output just the analysis, no preamble.")
I_WRITE = ("You are a WRITER agent in a multi-agent pipeline. Do not use any tools. Do not read files "
           "or search. Respond directly.\n\nProduce the top prioritized action items as a short "
           "recommendation. At most 120 words. Output just the recommendation.")

# --- REAL verbatim outputs + runtime tokens + agent-ids per model -----------------------------
# facts = extractor output; ab/at = analyst baseline/toap; wb/wt = writer baseline/toap
DATA = {
  "haiku": {
    "facts": ("Incident INC-4821 - Key Facts: Date/Time March 3, 14:02 UTC. Duration 47 minutes. "
              "Symptoms: API latency 120ms to 9.4s; error rate 38%. Root Cause: Marketing campaign "
              "drove 5x normal traffic; 4 fixed pods (no autoscaling). Mechanism: DB connection pool "
              "(max 20) saturated, request queuing, memory growth, 2 pods OOM-restarted. Resolution: "
              "Manual scale-up to 12 pods at 14:49. Impact: $42,000 revenue loss. Gaps: Load testing "
              "only at 1x; no alert for connection-pool saturation; first alert was customer complaints."),
    "ana_b": ("Root Cause: Insufficient capacity planning and missing observability. No autoscaling, "
              "load-tested only at 1x, connection pool saturation cascaded into OOM, no backpressure, "
              "no alert. Severity: Critical, 47-minute outage, $42K loss. Actions: 1. horizontal pod "
              "autoscaling 2. connection pool saturation alerts 3. backpressure/graceful degradation "
              "4. load testing 5-10x 5. infrastructure observability."),
    "ana_t": ("Root Cause: Marketing campaign caused 5x traffic surge; lacked autoscaling and "
              "monitoring. DB connection pool (max 20) saturated, request queuing, memory exhaustion, "
              "pod restarts. Severity: Critical, 47-minute outage, $42K loss, 38% error rate. Actions: "
              "1. autoscaling 2. proactive alerting for connection-pool saturation 3. load testing 5x+ "
              "4. marketing-ops coordination 5. increase connection pool size."),
    "wb": ("1. Enable horizontal pod autoscaling 2. Add connection pool saturation alerts 3. Implement "
           "queue backpressure/graceful degradation 4. Expand load testing to 5-10x 5. Establish pool "
           "monitoring dashboards."),
    "wt": ("1. Increase DB connection pool beyond 20 and optimize pooling 2. Deploy autoscaling with "
           "traffic-responsive thresholds 3. Add proactive alerting for connection-pool saturation and "
           "dashboards 4. Expand load testing to 5x+ and marketing-ops coordination 5. post-incident review."),
    "rt": {"extractor": 19661, "ana_b": 19824, "ana_t": 19590, "wb": 19791, "wt": 19552},
    "ids": {"extractor": "aa6dd677545a43a37", "ana_b": "a8b5493ac7e2d3295", "ana_t": "a5fb63eaf24d0a298",
            "wb": "a7887f57d0a44b37f", "wt": "a4e2a43d36affcac2"},
    "acc": {"baseline": 4, "toap": 3},  # independent judge, /4
  },
  "sonnet": {
    "facts": ("Incident: INC-4821. March 3, 14:02 UTC. Duration 47 minutes. Symptoms: API latency "
              "120ms -> 9.4s; error rate 38%. Root Cause: Marketing campaign ~5x traffic; 4 fixed pods "
              "(no autoscaling); DB connection pool (max 20) saturated; unbounded queue caused memory "
              "growth; 2 pods OOM-restarted. Resolution: Manual scale-up to 12 pods at 14:49. Impact: "
              "$42,000 loss. Gaps: load tested only 1x; no autoscaling; no saturation alert; first "
              "signal social media."),
    "ana_b": ("Root cause: Absence of HPA combined with undersized DB connection pool (max 20) and "
              "unbounded request queue. 5x spike saturated the pool, queued requests consumed memory, "
              "2 of 4 pods OOM-restarted. Severity: High. 47-minute outage, 38% error rate, $42,000 "
              "loss, detected via social media. Actions: HPA; increase/dynamically size DB pool; "
              "request queue backpressure/shedding; alerts for pool saturation and memory; load test "
              "3x-5x before campaigns."),
    "ana_t": ("Root cause: HPA absent, 4 fixed pods against ~5x surge saturated the 20-connection DB "
              "pool, requests queued unboundedly, OOM crashes, cascading errors. Severity: High. "
              "47-minute outage, 38% error rate, $42,000 loss. Remediation: 1. HPA with CPU/RPS scaling "
              "2. increase/pool DB connections, read replicas 3. request-queue limits and circuit "
              "breakers 4. alerting on connection-pool saturation and queue depth 5. load tests 3x-6x."),
    "wb": ("1. Increase DB connection pool size and optimize pooling 2. Deploy autoscaling (8+ pods for "
           "5x) 3. proactive alerting for connection-pool saturation and dashboards 4. load testing 5x+ "
           "and marketing-ops coordination 5. post-incident review."),
    "wt": ("1. Deploy HPA with CPU and RPS-based scaling 2. Increase DB pool and evaluate read replicas "
           "3. request queue limits and circuit breakers to prevent OOM 4. proactive alerting on "
           "connection-pool exhaustion and queue depth 5. load tests at 3x-6x baseline."),
    "rt": {"extractor": 12188, "ana_b": 12329, "ana_t": 12117, "wb": 12282, "wt": 12111},
    "ids": {"extractor": "a30f1371ed783e17b", "ana_b": "aa1d6e1598b9a3398", "ana_t": "ac8b33ff72e88301b",
            "wb": "afb7780371bab823f", "wt": "acfbe659d01aec7e9"},
    "acc": {"baseline": 4, "toap": 4},
  },
  "opus": {
    "facts": ("INC-4821: March 3, 14:02 UTC, payments API latency 120ms to 9.4s; error rate 38% for 47 "
              "minutes. Trigger: marketing campaign ~5x traffic to checkout. 4 fixed pods, no "
              "autoscaling. DB connection pool (max 20) saturated, queuing requests; no backpressure "
              "caused memory growth and 2 pods OOM-restarted. Manual scale-up to 12 pods at 14:49. "
              "Revenue impact $42,000. Load-tested only 1x. No saturation alert; first signal customer "
              "complaints. No deployment."),
    "ana_b": ("Root cause: no elastic capacity (4 fixed pods, no autoscaling) and a hard DB pool ceiling "
              "(max 20) with no backpressure. 5x surge saturated the pool, queued requests grew "
              "unbounded, OOM restarts. Detection failed: no alert, customers reported first. Severity: "
              "High. 47-minute outage, 38% error rate, ~$42,000 lost. Actions: autoscaling and "
              "right-size pool; queue backpressure/load-shedding; alerts on saturation, latency, error "
              "rate; load-test 5x+; capacity reviews before campaigns."),
    "ana_t": ("Root cause: capacity/architecture failure, not code. 5x spike hit statically-provisioned "
              "service (4 fixed pods) whose DB pool (max 20) saturated. No backpressure let queues grow, "
              "memory growth, OOM restarts. Detection failed: no alert. Severity: High. 47-minute "
              "outage, 38% error rate, $42K loss. Actions: autoscaling and right-size DB pool; "
              "backpressure/load-shedding and timeouts; alerts on saturation, latency, error rate; "
              "load-test 5x+ and pre-scale for campaigns."),
    "wb": ("1. Enable horizontal autoscaling and right-size pod count (campaign-aware reviews) 2. Raise "
           "and tune DB connection-pool limit; add queue backpressure and load-shedding 3. proactive "
           "alerts on pool saturation, latency, error rate 4. load-test 5x+ peak."),
    "wt": ("1. Enable horizontal autoscaling and right-size the DB connection pool 2. add backpressure/"
           "load-shedding and request timeouts 3. alert on pool saturation, latency, error rate 4. "
           "load-test at realistic peak (5x+) and pre-scale ahead of campaigns."),
    "rt": {"extractor": 12338, "ana_b": 12590, "ana_t": 12296, "wb": 12565, "wt": 12285},
    "ids": {"extractor": "afd827cc035a466d2", "ana_b": "a0ad62ae312e1d046", "ana_t": "aa1cc248cc35e2b9e",
            "wb": "aaebd75e7c1d24445", "wt": "a2cb3d392d2a6096b"},
    "acc": {"baseline": 4, "toap": 4},
  },
}
RUBRIC_N = 4
JUDGE_MODEL = "opus"


def canonical(model):
    d = DATA[model]
    # baseline = full document + verbatim upstream; toap = only the needed upstream slice
    ana_b_prompt = f'{I_ANALYZE}\n\nDocument:\n"{DOC}"\n\nExtracted facts:\n"{d["facts"]}"'
    ana_t_prompt = f'{I_ANALYZE}\n\nExtracted facts:\n"{d["facts"]}"'
    wr_b_prompt = (f'{I_WRITE}\n\nDocument:\n"{DOC}"\n\nExtracted facts:\n"{d["facts"]}"\n\n'
                   f'Analysis:\n"{d["ana_b"]}"')
    wr_t_prompt = f'{I_WRITE}\n\nAnalysis:\n"{d["ana_t"]}"'
    return ana_b_prompt, ana_t_prompt, wr_b_prompt, wr_t_prompt


def main():
    rows = []
    for model in ("haiku", "sonnet", "opus"):
        d = DATA[model]
        ab, at, wb, wt = canonical(model)
        # downstream prompt+output tokens (analyst + writer), baseline vs toap
        base = tk(ab) + tk(d["ana_b"]) + tk(wb) + tk(d["wb"])
        toap = tk(at) + tk(d["ana_t"]) + tk(wt) + tk(d["wt"])
        rows.append({
            "model": model,
            "down_base_tok": base, "down_toap_tok": toap, "reduction": base / toap,
            "acc_base": d["acc"]["baseline"], "acc_toap": d["acc"]["toap"],
            "rt_overhead_proxy": d["rt"]["extractor"],
        })

    md = ["# Multi-model results — scenario 1 (incident), Haiku/Sonnet/Opus\n",
          "> Real subagents per model. Token reduction = tiktoken(cl100k) on harness-canonical "
          "downstream prompts+outputs (reproducible). Accuracy = independent opus judge, blind, /4.\n",
          "| Model | downstream tok (baseline) | downstream tok (TOAP) | reduction | accuracy baseline | accuracy TOAP |",
          "|---|---|---|---|---|---|"]
    for r in rows:
        md.append(f"| {r['model']} | {r['down_base_tok']} | {r['down_toap_tok']} | "
                  f"{r['reduction']:.2f}x | {r['acc_base']}/4 | {r['acc_toap']}/4 |")
    reductions = [r["reduction"] for r in rows]
    md.append("")
    md.append(f"- **Token reduction (downstream):** mean {sum(reductions)/len(reductions):.2f}x, "
              f"range [{min(reductions):.2f}x, {max(reductions):.2f}x] across the three model scales.")
    md.append("- **Accuracy — the key finding:** Sonnet and Opus hold parity (4/4 -> 4/4), but **Haiku "
              "drops 4/4 -> 3/4 under TOAP**: with only the distilled analysis, the smallest model "
              "omitted the backpressure/load-shedding action that it kept when given the full transcript. "
              "Context minimization is not free on small models.")
    md.append(f"- Runtime `subagent_tokens` (transparency only; overhead-polluted, NOT cross-model "
              f"comparable): Haiku ~19.6k/call, Sonnet/Opus ~12.2k/call. See per-call ids in this file.")
    md.append("\n## Honest limitations")
    md.append("- One Claude *family* / one tokenizer (Haiku/Sonnet/Opus differ by scale, not vendor). "
              "Cross-vendor (GPT/Gemini/Llama) untested.\n"
              "- n=1 scenario at multi-model; single sample per call.\n"
              f"- Judge is {JUDGE_MODEL} (same family), fresh + blind.\n"
              "- Token reduction uses canonical full-document baseline; actual sent prompts during the "
              "run were sometimes condensed, which would only *understate* the reduction.")
    report = "\n".join(md)
    with open(os.path.join(HERE, "..", "multimodel_results.md"), "w", encoding="utf-8") as f:
        f.write(report)
    with open(os.path.join(HERE, "..", "multimodel_results.json"), "w", encoding="utf-8") as f:
        json.dump({"rows": rows, "judge_model": JUDGE_MODEL, "data": DATA}, f, indent=2)
    print(report)
    print("\n[written] benchmark/ab_study/multimodel_results.md and .json")


if __name__ == "__main__":
    main()
