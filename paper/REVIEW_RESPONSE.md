# Reviewer critique → fixes (tracked)

Critique received on the v1 paper. Each item, the fix, and status.

| # | Critique | Fix | Status |
|---|---|---|---|
| 1 | n=1 per cell; "mean 1.94x" misleading | **DONE.** Ran **n=33** controlled writer experiment; report ranges; reframed as preliminary pilot throughout. The n=1 Haiku regression **did not reproduce** and is **retracted** in the paper. | RESOLVED |
| 2 | Baseline sandbagged (naive transcript) | **DONE.** Added a **summarizing-baseline** arm. Honest result: **summary (~2.5x) beats TOAP (~1.9x)** on tokens at equal accuracy → paper now argues TOAP's value is losslessness+security+systematization, NOT token savings. | RESOLVED |
| 3 | Architecture reads as built but isn't | **DONE.** Callout box at top of paper + dedicated §"Implemented vs proposed"; V2/KV-bridge tagged "proposed" in the diagram and text. | RESOLVED |
| 4 | Judge same model family (bias) | **DONE.** Primary accuracy is now a **deterministic, model-independent rubric scorer**; model judge demoted to a discarded secondary check; bias caveat in abstract + limitations. | RESOLVED |
| 5 | Rust impl claims thin | **DONE.** §Implementation expanded with architectural decisions; clarified unit+integration incl. **real-TCP end-to-end (not mocked)**; security integration tests described. | RESOLVED |
| S1 | Abstract overloaded; finding buried | **DONE.** Abstract rewritten to lead with the honest finding (summary beats TOAP; parity; retraction). | RESOLVED |
| S2 | §5 too short vs §4 | **DONE.** Implementation expanded. | RESOLVED |
| S3 | "Why a protocol not a better orchestrator?" | **DONE.** Dedicated §"Why a Protocol, Not Just a Better Orchestrator?" (losslessness / security / systematization). | RESOLVED |
| F | Cleaner diagrams | **DONE.** Improved systems diagram + token-flow diagram + three-arm bar w/ min-max error bars + accuracy-parity bar. (Cache-reuse folded into token-flow.) | RESOLVED |

## Final numbers (n=33, all committed)
- Accuracy: **all 33 samples 4/4** (deterministic scorer); zero variance; n=1 regression retracted.
- Token reduction vs naive: **TOAP 1.88-1.93x**, **summary 2.39-2.62x** -- summary beats TOAP, both at 4/4.
- Bytes vs tokens: opcodes 50% bytes but 0-17% tokens.
- Paper: 9 pages, compiles clean, 5 figures (3 plots + 2 TikZ) + landscape table + results table.

## Experimental design for the n>1 redo (controls)
- **Fixed canonical upstream** (one DOC, one FACTS, one ANALYSIS containing all 4 rubric themes) so the
  only variables are {model, arm, sample}. Isolates the writer's behavior.
- **Arms:** naive (doc+facts+analysis) / summary (short summary of analysis) / TOAP (analysis only).
- **Models:** Haiku, Sonnet, Opus. **n=5** per cell.
- **Primary accuracy:** deterministic theme-coverage scorer (bias-free, reproducible). Each of the 4
  rubric themes detected by keyword sets. (Secondary: model judge, acknowledged biased.)
- **Key question (resolves #1):** does the single-sample Haiku 4/4→3/4 regression survive n=5, or was
  it noise / upstream propagation? Report whatever we find.

## Honest expected narrative
TOAP's token reduction ≈ what a competent summarizing orchestrator achieves; both beat naive
transcript-accumulation. TOAP's distinct value is (a) lossless references (summary is lossy → accuracy
risk), (b) security/taint/identity carried in-band, (c) systematic at the protocol layer. The pilot
quantifies the token/accuracy trade-off and whether small models lose accuracy under minimization.
