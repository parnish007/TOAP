# Multi-model results — scenario 1 (incident), Haiku/Sonnet/Opus

> Real subagents per model. Token reduction = tiktoken(cl100k) on harness-canonical downstream prompts+outputs (reproducible). Accuracy = independent opus judge, blind, /4.

| Model | downstream tok (baseline) | downstream tok (TOAP) | reduction | accuracy baseline | accuracy TOAP |
|---|---|---|---|---|---|
| haiku | 1031 | 529 | 1.95x | 4/4 | 3/4 |
| sonnet | 1057 | 557 | 1.90x | 4/4 | 4/4 |
| opus | 1055 | 538 | 1.96x | 4/4 | 4/4 |

- **Token reduction (downstream):** mean 1.94x, range [1.90x, 1.96x] across the three model scales.
- **Accuracy — the key finding:** Sonnet and Opus hold parity (4/4 -> 4/4), but **Haiku drops 4/4 -> 3/4 under TOAP**: with only the distilled analysis, the smallest model omitted the backpressure/load-shedding action that it kept when given the full transcript. Context minimization is not free on small models.
- Runtime `subagent_tokens` (transparency only; overhead-polluted, NOT cross-model comparable): Haiku ~19.6k/call, Sonnet/Opus ~12.2k/call. See per-call ids in this file.

## Honest limitations
- One Claude *family* / one tokenizer (Haiku/Sonnet/Opus differ by scale, not vendor). Cross-vendor (GPT/Gemini/Llama) untested.
- n=1 scenario at multi-model; single sample per call.
- Judge is opus (same family), fresh + blind.
- Token reduction uses canonical full-document baseline; actual sent prompts during the run were sometimes condensed, which would only *understate* the reduction.