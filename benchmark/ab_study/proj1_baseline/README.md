# proj1 — BASELINE arm (no TOAP)

This arm models the common "stuff the whole transcript" pattern used by many agent frameworks: each
downstream agent's prompt contains the **full document plus every prior agent's verbatim output**.

There is no code to run here — the arm is defined by how `../harness.py` assembles prompts:

- extractor prompt = instruction + document
- analyst prompt  = instruction + document + extracted facts
- writer prompt   = instruction + document + extracted facts + analysis

The same model (Claude subagents), the same role instructions, the same documents, and the same
judge are used as in proj2. **Only the context assembly differs** — that is the experimental
treatment. (Giving both arms identical prompts would measure nothing.)
