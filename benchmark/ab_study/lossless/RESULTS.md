# Losslessness under pressure — results

**Question.** A summary is cheaper than a reference in tokens (see the writer experiment). But a
summary is *lossy*. Does that lossiness actually cost anything downstream, or is it harmless?

**Design (pre-registered in [`scenario.json`](scenario.json), fixed before any responder ran).**
A 130-word incident report carries 8 distinct facts. A summarizer subagent compressed it faithfully to
45 words — a length at which all 8 facts cannot survive. A downstream responder then answered 8 fixed
questions (one per fact) under two arms:

- **summary** — sees only the 45-word summary,
- **reference** — sees the full document (the lossless reference, materialized).

3 samples per arm, deterministic substring grading ([`grade.py`](grade.py)). Raw verbatim outputs in
[`runs.json`](runs.json); summarizer = Claude subagent `a40bc004`.

## Result (Claude family)

| | summary arm | reference arm |
|---|:---:|:---:|
| Mean score (3 runs) | **6 / 8** | **8 / 8** |
| Per-run | [6, 6, 6] | [8, 8, 8] |

The faithful summary dropped two facts — the **rollback time (09:51)** and **why staging passed (it
uses a different schema)**. The downstream responder then failed *exactly* those two questions in the
summary arm (0/3 each) and answered them in the reference arm (3/3 each). Every other question was 3/3
in both arms.

```
arm        n  mean/8   scores
summary    3   6.00    [6, 6, 6]
reference  3   8.00    [8, 8, 8]
Q2 rollback time   : summary 0/3  vs  reference 3/3   <- dropped by summary
Q6 staging reason  : summary 0/3  vs  reference 3/3   <- dropped by summary
```

## What this shows, and what it does not

- **Shows:** the mechanism is real. A faithful summary drops facts, and a task needing a dropped fact
  then fails silently; the lossless reference does not, because the information is still there.
- **Does not show:** *how often* this matters in practice. This is one scenario at one compression
  ratio; the 2-fact drop is a property of that ratio, not a universal constant. Generalizing the
  frequency needs several document types (contracts, specs, narrative) at larger n.

## Cross-vendor replication

`lossless_colab.ipynb` runs the identical pre-registered experiment on an open, non-Claude instruct
model (Qwen2.5-7B / Llama-3.1-8B / Mistral-7B) at n=10 per arm on a free Colab T4. This is the run that
removes the "single model family" caveat for the losslessness claim: if an open model also scores
higher with the reference than the summary, the effect is not Claude-specific. Output:
`lossless_crossvendor.json`.
