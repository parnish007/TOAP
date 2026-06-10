#!/usr/bin/env python3
"""Deterministically grade the losslessness-under-pressure experiment.

A question is correct if any of its pre-registered answer_substrings appears in the response and the
response is not "NOT IN CONTEXT". For Q2 (two facts: name + time) both substrings must appear.
"""
import json
import os
import statistics

HERE = os.path.dirname(os.path.abspath(__file__))
scn = json.load(open(os.path.join(HERE, "scenario.json")))
runs = json.load(open(os.path.join(HERE, "runs.json")))
questions = scn["questions"]


def score_answer(ans, q):
    a = ans.lower()
    if "not in context" in a:
        return 0
    subs = [s.lower() for s in q["answer_substrings"]]
    # Q2 needs both the name and the time; others need any one listed substring.
    if q["fact"].startswith("Priya"):
        return 1 if (("priya" in a) and ("09:51" in a)) else 0
    return 1 if any(s in a for s in subs) else 0


by_arm = {"summary": [], "toap": []}
per_q = {"summary": [0] * len(questions), "toap": [0] * len(questions)}
for s in runs["samples"]:
    total = 0
    for i, q in enumerate(questions):
        c = score_answer(s["answers"][i], q)
        total += c
        per_q[s["arm"]][i] += c
    by_arm[s["arm"]].append(total)

print(f"{'arm':8} {'n':>2} {'mean/8':>7} {'scores':>12}")
print("-" * 34)
for arm in ("summary", "toap"):
    sc = by_arm[arm]
    print(f"{arm:8} {len(sc):>2} {statistics.mean(sc):>7.2f} {str(sc):>12}")

print("\nPer-question correct (out of 3 samples):  [summary / toap]")
n = len(runs["samples"]) // 2
for i, q in enumerate(questions):
    flag = "  <-- summary drops it" if per_q["summary"][i] < per_q["toap"][i] else ""
    print(f"  Q{i+1} ({q['fact']:32}): {per_q['summary'][i]}/{n}  /  {per_q['toap'][i]}/{n}{flag}")

out = {
    "summary_mean": statistics.mean(by_arm["summary"]),
    "toap_mean": statistics.mean(by_arm["toap"]),
    "summary_scores": by_arm["summary"],
    "toap_scores": by_arm["toap"],
    "n_questions": len(questions),
    "per_question_summary": per_q["summary"],
    "per_question_toap": per_q["toap"],
}
json.dump(out, open(os.path.join(HERE, "results.json"), "w"), indent=2)
print("\n[written] results.json")
