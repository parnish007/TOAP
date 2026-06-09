#!/usr/bin/env python3
"""
A/B benchmark harness. Encodes the experimental design IN CODE so prompts are generated, not
hand-typed. The ONLY difference between the two arms is context assembly:

  proj1_baseline : each downstream agent receives the full document + verbatim prior outputs.
  proj2_toap     : each downstream agent receives ONLY the distilled upstream output it needs
                   (analyst <- facts; writer <- analysis) — the reference-minimization TOAP enables.

Everything else is held constant: model (Claude subagents), role instructions, the scenario
document, the rubric, the judge. Prompts are produced by the functions below from a scenario file
plus the verbatim upstream outputs collected during the run.

Usage (prints a prompt to copy into a subagent spawn):
  python harness.py <scenario.json> extractor
  python harness.py <scenario.json> analyst  --facts facts.txt
  python harness.py <scenario.json> writer   --facts facts.txt --analysis analysis.txt --arm baseline
  python harness.py <scenario.json> judge    --rec-a a.txt --rec-b b.txt
"""
import argparse
import json
import sys

NO_TOOLS = ("Do not use any tools. Do not read files or search. Respond directly.")

I_EXTRACT = (f"You are an EXTRACTION agent in a multi-agent pipeline. {NO_TOOLS}\n\n"
             "From the document below, extract the key structured facts only. Be factual and "
             "concise - at most 120 words. Output just the facts, no preamble.")
I_ANALYZE = (f"You are an ANALYSIS agent in a multi-agent pipeline. {NO_TOOLS}\n\n"
             "State the root cause/diagnosis, the severity, and what should be done about it. "
             "At most 120 words. Output just the analysis, no preamble.")
I_WRITE = (f"You are a WRITER agent in a multi-agent pipeline. {NO_TOOLS}\n\n"
           "Produce the top prioritized action items as a short recommendation. At most 120 words. "
           "Output just the recommendation.")
I_JUDGE = (f"You are an EVALUATION agent. {NO_TOOLS} Respond objectively.")


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def extractor_prompt(s):
    return f'{I_EXTRACT}\n\nDocument:\n"{s["document"]}"'


def analyst_prompt(s, facts, arm):
    if arm == "baseline":  # full context: document + facts
        return f'{I_ANALYZE}\n\nDocument:\n"{s["document"]}"\n\nExtracted facts:\n"{facts}"'
    return f'{I_ANALYZE}\n\nExtracted facts:\n"{facts}"'  # toap: facts only


def writer_prompt(s, facts, analysis, arm):
    if arm == "baseline":  # full transcript: document + facts + analysis (verbatim)
        return (f'{I_WRITE}\n\nDocument:\n"{s["document"]}"\n\nExtracted facts:\n"{facts}"\n\n'
                f'Analysis:\n"{analysis}"')
    return f'{I_WRITE}\n\nAnalysis:\n"{analysis}"'  # toap: analysis only


def judge_prompt(s, rec_a, rec_b):
    reqs = "\n".join(f"({i+1}) {r}" for i, r in enumerate(s["rubric"]))
    return (f'{I_JUDGE}\n\nA correct recommendation MUST cover these {len(s["rubric"])} required '
            f'actions:\n{reqs}\n\nScore each recommendation by how many it covers (0-{len(s["rubric"])}), '
            f'strictly, listing any missing. Recommendations are presented blind.\n\n'
            f'--- Recommendation A ---\n"{rec_a}"\n\n--- Recommendation B ---\n"{rec_b}"\n\n'
            f'Output exactly:\nA: <score>/{len(s["rubric"])} ; missing: <list or none>\n'
            f'B: <score>/{len(s["rubric"])} ; missing: <list or none>')


def _read(path):
    with open(path, encoding="utf-8") as f:
        return f.read().strip()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("scenario")
    ap.add_argument("stage", choices=["extractor", "analyst", "writer", "judge"])
    ap.add_argument("--facts"); ap.add_argument("--analysis")
    ap.add_argument("--arm", choices=["baseline", "toap"], default="baseline")
    ap.add_argument("--rec-a"); ap.add_argument("--rec-b")
    a = ap.parse_args()
    s = load(a.scenario)
    if a.stage == "extractor":
        print(extractor_prompt(s))
    elif a.stage == "analyst":
        print(analyst_prompt(s, _read(a.facts), a.arm))
    elif a.stage == "writer":
        print(writer_prompt(s, _read(a.facts), _read(a.analysis), a.arm))
    elif a.stage == "judge":
        print(judge_prompt(s, _read(a.rec_a), _read(a.rec_b)))


if __name__ == "__main__":
    main()
