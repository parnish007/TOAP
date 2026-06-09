# proj2 — TOAP arm (reference minimization)

This arm models TOAP's contribution: outputs are stored once and passed **by reference**, so each
downstream agent's prompt contains **only the distilled upstream context it needs** — not the raw
document or the full transcript.

Defined by `../harness.py`:

- extractor prompt = instruction + document        (identical to baseline — the extractor must read the doc)
- analyst prompt  = instruction + extracted facts   (NO raw document)
- writer prompt   = instruction + analysis          (NO document, NO facts)

## Using the live TOAP/MCP path (optional, realistic)

In a deployed system the agent would not be handed the minimized context; it would hold a reference
(`CTX:n`) and fetch what it needs:

- agent-to-agent: the broker delivers `ANALYZE(CTX:2)`; the agent calls the store for CTX:2.
- MCP host: run `cargo run -p toap-mcp` and the agent calls `toap_get {id}` as an MCP tool.

We do NOT route the live MCP call through the measured subagents because (a) the fetched bytes would
re-enter the agent context anyway, so the measured quantity is the same, and (b) attaching a custom
MCP server to spawned subagents is not reliably supported. The harness therefore supplies the
minimized context directly — measuring the identical token quantity the reference path would produce.
