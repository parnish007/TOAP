# TOAP, Explained — Top to Bottom

A plain-language walkthrough of what TOAP is, how every layer works, and — just as
importantly — where it wins, where it loses, and why. This is the document to read if you want
to *understand* the system before reading the paper or the code.

> One-line summary: **TOAP is a state-management protocol for multi-agent LLM systems built on
> one idea — store content once, pass references everywhere — with a two-plane wire format, a
> reference-materialization spectrum, and capability-lattice security attached to every context.**

---

## 1. The Big Picture

Multi-agent pipelines have a quiet, expensive habit: they re-send the same context on every hop.

Without TOAP, the transcript grows at every step:

```
Agent 1   receives:  Document
Agent 2   receives:  Document + Agent1 output
Agent 3   receives:  Document + Agent1 output + Agent2 output
Agent 4   receives:  Document + Agent1 output + Agent2 output + Agent3 output
                     \_____________________  _____________________/
                                           \/
                          cost grows with depth (token explosion)
```

With TOAP, the content lives once in a shared store and only **IDs** travel:

```
Document  ──store──▶  CTX:1

Agent 1   ──▶  CTX:1
Agent 2   ──▶  CTX:1
Agent 3   ──▶  CTX:1
Agent 4   ──▶  CTX:1
```

That is the whole intuition. Everything else in TOAP exists to make that idea **safe**,
**lossless**, and **practical** across many heterogeneous agents.

### The honest framing (read this before the hype)

TOAP's own experiments show that a *competent summarizing orchestrator* beats reference-passing on
raw token count (~2.5× vs ~1.9× vs a naive baseline), at equal task accuracy. So TOAP is **not**
primarily a token-saving trick. Its real value is what a summary throws away:

- **Losslessness** — a reference can be re-expanded to the full original content on demand; a
  summary is lossy and a dropped detail is gone forever.
- **Security/provenance** — identity, taint, and capability constraints travel *with* the reference.
- **Systematization** — the behavior is uniform across agents/languages instead of being
  re-implemented as a prompt template in each app.

Think "operating-system-style messaging layer for agents," not "another prompt-compression hack."

---

## 2. The Layer Stack

TOAP is layered. A message travels down the stack on the way out and up the stack on the way in.

```
┌───────────────────────────────────────────────┐
│  Agents          LLMs / tools / rule-based      │   what does the work
├───────────────────────────────────────────────┤
│  TOAP Client     thin SDK                        │   hides the protocol
├───────────────────────────────────────────────┤
│  Protocol        V1 text  /  V2 binary codec     │   encode / validate
├───────────────────────────────────────────────┤
│  Security        identity · ACL · capability     │   allow / refuse
├───────────────────────────────────────────────┤
│  Broker / Router sessions · routing · fan-out    │   deliver / correlate
├───────────────────────────────────────────────┤
│  Context Store   CTX:N → content + metadata      │   store once
├───────────────────────────────────────────────┤
│  Transport       length-prefixed TCP             │   move bytes
└───────────────────────────────────────────────┘
```

Each layer below is explained in detail, with what it does, the relevant code, and its trade-offs.

---

## 3. Layer 1 — Agents

At the top are the agents. They can be LLMs, rule-based workers, tools, or MCP-connected agents.

```
┌──────────┐  ┌──────────┐  ┌──────────┐
│ Agent A  │  │ Agent B  │  │ Agent C  │
└──────────┘  └──────────┘  └──────────┘
```

**The key rule: agents never talk to each other directly.** Everything goes through the broker.
This is deliberate — it is what makes identity un-spoofable (Layer 4) and routing uniform (Layer 5).

In the repo, the demo agents are in `crates/toap-agents`:
- `agent_b` — a summarizer (`SUM`)
- `classifier` — a rule-based classifier (`CLS`)
- `orchestrator` — stores a document once and fans it out to both workers by reference

**Pros:** agents stay simple; they don't manage transport, retries, or identity.
**Cons:** the broker is a central dependency — if it's down, nobody talks (see Layer 5 cons).

---

## 4. Layer 2 — TOAP Client (the SDK)

Each agent uses a small async SDK so it never hand-writes wire bytes.

```rust
let client = Client::connect("127.0.0.1:7700", "agentA", "SUM,GEN").await?;
let cid     = client.set_context("the document text", /*tainted=*/true).await?;
let reply   = client.send_request("agentB", Payload::new("SUM").arg_ctx(cid)).await?;
let summary = client.get_context(reply.payload.first_ctx().unwrap()).await?;
client.subscribe(cid).await?;                  // get EVT notifications on changes
```

The client (`crates/toap-client`) handles:
- the SYN/ACK handshake that binds identity to the connection,
- request/response correlation by `msg_id` (so concurrent requests don't get crossed),
- a separate inbound channel for events (`recv_event`) vs. requests (`recv`).

```
Agent
  │  client.send_request(...)
  ▼
TOAP Client  ──── correlates reply by msg_id ────▶  back to the right await point
```

**Pros:** agent code is a handful of calls; protocol details are invisible.
**Cons:** today there is only a Rust SDK. Other languages must speak the wire format directly
(the format is simple text, so this is feasible, but there's no Python/JS client yet).

---

## 5. Layer 3 — Protocol Layer

This is where a message becomes bytes (and back). It does validation, serialization, parsing,
and encoding. TOAP has two wire versions.

### V1 — text

Human-readable, easy to debug. The frame is four pipe-separated fields:

```
TYPE | MSG_ID | TARGET | PAYLOAD
```

and the payload is function-style to avoid ambiguous colons:

```
REQ|100|agentB|SUM(CTX:42)?max_words=150&lang=en
RES|100|agentA|OK(CTX:87)
DLT|101|broker|PATCH(CTX:42)?field=status&value=approved
```

Why function-style? Because an earlier `SUM:CTX:42` shape is ambiguous — `CTX:42` already contains
a colon. `OP(args)?key=value` makes positional args and options unambiguous. The parser
(`crates/toap-core`) is **strict about protocol grammar** but **never rejects ordinary content** for
"looking dangerous" — SQL, HTML, code, and even "ignore previous instructions" are valid *data*.

### V2 — binary control plane

A compact binary header for the control fields, with the semantic payload kept as text. It
round-trips the exact same message model (`crates/toap-core/src/v2.rs`):

```
byte 0      version (0x02)
byte 1      message type
byte 2..6   msg_id (u32, big-endian)
byte 6      flags (e.g. tainted)
byte 7      target length
byte 8..    target
next 2      payload length (u16)
next N      payload bytes  ← the semantic plane stays here
```

**Pros:** text V1 is trivial to inspect and log; binary V2 shrinks the control overhead and is the
foundation of the two-plane design (Section 9).
**Cons:** two formats to maintain; V2 negotiation/upgrade is not yet a full handshake feature.

---

## 6. Layer 4 — Security Filter

One of the most interesting layers. Before a message is routed, it passes a filter that asks four
questions. (Implementations: `crates/toap-security` + enforcement in `crates/toap-broker`.)

```
Agent message
     │
     ▼
┌───────────────── Security Filter ─────────────────┐
│  1. Identity      who is really sending?            │
│  2. ACL           may they read / write / delete?   │
│  3. Capability    may THIS content reach THIS op?   │
│  4. Rate / Replay too fast?  a duplicate?           │
└────────────────────────────────────────────────────┘
     │  allow                          │  refuse
     ▼                                  ▼
   Broker                          ERR (NOPERM / RATE / REPLAY)
```

### 4.1 Identity — broker-derived, never claimed

The wire frame has **no `SRC` field**. The broker binds one `agent_id` to the connection at SYN and
uses that for routing, ACL, logs, and rate limits. A responder replies to the original `msg_id`, so
it never even learns who asked. This structurally defeats source-spoofing (a known weakness of
protocols where agents self-assert identity).

### 4.2 ACL — per-context read/write/delete

Each context carries an access list, e.g.:

```
agentA:rwd , agentB:r , *:r
```

The owner always has full access; others get exactly what the list grants. An unauthorized read
returns `NOPERM`, not the data.

### 4.3 Capability lattice — the structural injection guard

This is the part that makes TOAP feel like an OS. Content has an **origin** and a set of
**capabilities** it is allowed to flow into:

```
Origin            Allowed capabilities
──────            ────────────────────────────────────────────
Internal          read, summarize, transform, classify, EXEC, EMAIL, PAY, DELETE   (trusted: all)
External          read, summarize, transform, classify                              (no side effects)
User              read, summarize, classify                                         (most restricted)
```

The broker maps each opcode to a capability (`Capability::for_op`) and refuses anything the
content's provenance forbids:

```
User-originated content
        │
        ▼  attempts EXEC / PAY / DELETE
   ┌────────────┐
   │  REFUSED   │   ERR NOPERM reason=capability_denied
   └────────────┘
```

So if a malicious document literally says "delete the database," that *content* is structurally
blocked from reaching a destructive operation — regardless of how cleverly it's phrased. This is
CaMeL-style information-flow control, enforced at the protocol layer, and the taint **travels with
the reference** across agent hops (containing prompt-injection propagation).

### 4.4 Rate limit & replay

- **Rate limit:** a per-agent token bucket; bursts beyond capacity get `ERR RATE`.
- **Replay:** duplicate `msg_id`s within a session are rejected with `ERR REPLAY`.

**Pros:** identity can't be spoofed; injection is contained *structurally* (not by fragile keyword
bans); DoS and naive replay are handled.
**Cons:** replay protection is **per-session only** — full cross-reconnect protection needs signed
per-frame nonces (not yet built). There's no message **signing** yet, so a man-in-the-middle on the
transport could tamper with payloads; production deployments need TLS/mTLS underneath. The capability
lattice is coarse (three origins) — finer per-tool grants are future work.

---

## 7. Layer 5 — Broker / Router

The broker is TOAP's brain. Every message flows through it.

```
                 ┌───────────┐
                 │  Broker   │
                 └─────┬─────┘
          ┌───────────┼───────────┐
          ▼           ▼           ▼
      ┌───────┐   ┌───────┐   ┌───────┐
      │ Agt A │   │ Agt B │   │ Agt C │
      └───────┘   └───────┘   └───────┘
```

Responsibilities:

- **Routing** — deliver `REQ` to the target agent (`SEND → Agent B`).
- **Correlation** — match a `RES`/`ERR` back to the requester by `msg_id` (concurrent-safe).
- **Sessions** — one session record per connection (id, agent, caps, version).
- **Fan-out** — a `PATCH` to a subscribed context emits an `EVT` to every subscriber.
- **Replay protection** — rejects duplicate request ids in a session.

```
PATCH(CTX:42) ──▶ Broker ──┬──▶ EVT(CTX:42) ──▶ subscriber 1
                           ├──▶ EVT(CTX:42) ──▶ subscriber 2
                           └──▶ EVT(CTX:42) ──▶ subscriber 3
```

**Pros:** uniform enforcement point for identity/ACL/routing; subscriptions make collaborative
state changes push-based instead of polling.
**Cons:** the broker is a **single point of failure and a chokepoint** — this is an explicit
threat-model assumption. High-availability (multiple brokers, shared store, failover) is not yet
implemented. All traffic serializing through one broker is also a potential throughput ceiling.

---

## 8. Layer 6 — Shared Context Store

The heart of TOAP. Content is stored once and addressed by a numeric ID.

```
┌──────────────────────────────────────────────┐
│  CTX:1  →  Document (50k tokens)               │
│  CTX:2  →  Analysis                            │
│  CTX:3  →  Summary                             │
└──────────────────────────────────────────────┘

instead of sending 50k tokens, you send:  CTX:1
```

Each entry (`crates/toap-context`) carries more than bytes:

- **content**
- **owner** (broker-derived)
- **ACL** (read/write/delete)
- **provenance** (origin + capability set — Section 4.3)
- **TTL** + garbage collection (expired contexts are evicted)
- **version** + an append-only **delta log** (field-level `PATCH` history)
- **subscriptions** (who gets `EVT` on change)

It also exposes the **materialization** primitive used in Section 9.

**Pros:** dedup across the whole agent mesh (the thing provider prompt-caching can't do across
agents/providers); metadata travels with content; deltas + versioning enable collaborative edits and
time-travel-style inspection.
**Cons:** the current backend is **in-memory and single-node** — it does not survive a broker
restart and does not scale horizontally. Durable/distributed backends (Redis, mmap, etc.) are
roadmap. A reference is only as available as the store; if the store loses an entry, holders of the
ID must fall back to inline (handled by the spectrum in Section 9).

---

## 9. The Clever Part — Two-Plane Design

Most protocols mix routing metadata and payload into one blob and optimize one number. TOAP notices
that the two have **different readers with opposite cost functions**:

```
┌──────────────────────────────┐
│  CONTROL PLANE  (binary)      │  read by the BROKER
│  id · ACL · session · nonce · │  → optimize for BYTES
│  taint                        │     (bandwidth, parse latency)
├──────────────────────────────┤
│  SEMANTIC PLANE  (text)       │  read by the LLM
│  task · instructions · the    │  → optimize for TOKENS
│  actual content               │     (model cost, only what it reads)
└──────────────────────────────┘
```

- The **broker** mostly reads the control plane and never tokenizes the payload.
- The **LLM** mostly reads the semantic plane and never sees the control header.

By encoding each plane for its single reader, neither is dominated. This is why "bytes vs tokens"
stops being a confusion: they're literally different planes.

**Pros:** clean separation of concerns; lets the control plane go binary/compact while the semantic
plane stays tokenizer-friendly and human-readable.
**Cons:** more moving parts than a single JSON blob; the benefit only fully materializes once V2
binary framing is the default transport (today V1 text is the common path).

---

## 10. The Most Research-Worthy Idea — Reference Materialization Spectrum

A reference is one *logical* thing with three *physical* materializations, chosen per call by a cost
model:

```
        logical reference  R(content)
        ┌───────────┬───────────────┬──────────────┐
        ▼           ▼               ▼
   ┌─────────┐ ┌──────────┐  ┌──────────────┐
   │ INLINE  │ │ CTX_REF  │  │  KV_BRIDGE   │
   │ embed   │ │ send ID, │  │ reuse KV     │
   │ the text│ │ fetch it │  │ cache直接     │
   └─────────┘ └──────────┘  └──────────────┘
   tokens now   dedup win     skip prefill (same-model only)
```

- **INLINE** — embed the content as text. Best for tiny or single-use content (no store round-trip).
- **CTX_REF** — send the ID; the receiver fetches from the store. The dedup win for reused content.
- **KV_BRIDGE** — point at a reusable KV cache so the receiver skips prefill entirely. Only valid
  when sender and receiver share the *same model and tokenizer*.

The cost model weighs content length, reference frequency, link bandwidth, GPU load, and the
same-model constraint. Crucially, it **degrades gracefully** when constraints fail:

```
KV_BRIDGE  ──(no model runtime / different model / RoPE offset)──▶  CTX_REF
CTX_REF    ──(store unavailable / content tiny & one-shot)──────▶  INLINE
```

**Pros:** one decision spans the *token plane* (resend vs ID) and the *tensor plane* (re-prefill vs
load KV) — nobody else treats those as the same choice; the fallback chain means a reference is
never a hard dependency.
**Cons:** **KV_BRIDGE's actual tensor transport is not implemented** — only the policy and the
`KvTransport` trait exist; with no model runtime it always falls back to CTX_REF (which is the tested
default). KV reuse is also genuinely hard (RoPE position offsets, cross-context attention loss,
judge-vs-executor perturbation), and a KV cache is far larger than the text, so it often loses to
"re-send text + provider prefix cache." TOAP treats it as an optional, constraint-guarded mode for
exactly these reasons.

---

## 11. How a Request Flows (end to end)

Task: *summarize a document, then act on the summary.*

```
Step 1   Agent stores the doc once
         STORE(doc) ─────────────▶ CTX:1

Step 2   Agent asks a worker to summarize, by reference
         REQ SUM(CTX:1)?max_words=150

Step 3   Broker checks identity · ACL · capability · rate · replay
         (doc is User-origin → SUM is allowed; EXEC would be refused)

Step 4   Target agent receives CTX:1, materializes it (INLINE/CTX_REF/KV)
         reads the content, summarizes

Step 5   Worker stores its output
         OK(CTX:2)

Step 6   The next agent receives CTX:2 — NOT the whole transcript
         REQ ACT(CTX:2)
```

The point of Step 6: downstream agents get exactly the distilled context they need by reference,
instead of an ever-growing transcript.

---

## 12. Architecture in One Diagram

```
              ┌─────────────────────────────┐
              │           Agents            │   LLMs · tools · rule-based
              └──────────────┬──────────────┘
                             │  (never talk directly)
              ┌──────────────▼──────────────┐
              │         TOAP Client         │   async SDK
              └──────────────┬──────────────┘
                             │
              ┌──────────────▼──────────────┐
              │       Protocol Layer        │   V1 text · V2 binary
              └──────────────┬──────────────┘
                             │
              ┌──────────────▼──────────────┐
              │   Security: ACL · capability │   identity · taint · rate · replay
              └──────────────┬──────────────┘
                             │
              ┌──────────────▼──────────────┐
              │       Broker / Router       │   sessions · routing · fan-out
              └───────┬──────────────┬───────┘
                      │              │
            ┌─────────▼───┐   ┌──────▼──────┐
            │ Context     │   │   Agents    │
            │ Store       │   │ (targets)   │
            │ CTX:N→data  │   └─────────────┘
            └─────────────┘
                      │
              ┌───────▼──────────────────────┐
              │   Transport: TCP (→ WS/gRPC)  │
              └───────────────────────────────┘
```

---

## 13. Honest Pros and Cons — the Whole System

### Where TOAP genuinely wins

- **Lossless references** beat lossy summaries when a later step needs a detail an upfront summary
  would have dropped.
- **Security travels with data** — broker-derived identity + capability lattice contain spoofing and
  prompt-injection propagation structurally, which a prompt template cannot.
- **Cross-agent / cross-provider dedup** — provider prompt caching only helps a stable prefix within
  one provider/session; TOAP dedups across the whole mesh.
- **Systematized multi-agent state** — versioning, deltas, subscriptions, TTL, ACL in one place.

### Where TOAP does not win (and the paper says so)

- **Raw tokens:** a competent summarizing orchestrator is *more* token-efficient (~2.5× vs ~1.9×).
  If you only care about token count and accept lossiness, summarize.
- **Symbolic opcodes:** terse syntax saves ~50% of *bytes* but only 0–17% of *tokens* (BPE taxes
  punctuation). The win is content referencing, not opcode terseness.
- **Single agent reading one doc once:** referencing saves nothing — the agent still reads the doc.

### Current implementation limitations (roadmap)

| Limitation | Status |
|---|---|
| Context store is in-memory, single-node | durable/distributed backend is roadmap |
| Broker is a single point of failure | HA/failover not implemented |
| KV-bridge tensor transport | only the policy/trait exists; needs a model runtime |
| Message signing / cross-reconnect replay nonces | not yet (per-session replay only) |
| Cross-vendor token claims | only the Claude family/one tokenizer measured so far |
| Client SDKs | Rust only today |

### Evidence honesty

The benchmark is a real but **single-vendor, small-n pilot** (Claude Haiku/Sonnet/Opus, n=33,
deterministic bias-free scoring). It is enough to establish direction and to *disprove* the naive
"references save the most tokens" claim; it is **not** enough for a general efficiency claim. See
`docs/claims.md` and the paper's limitations section.

---

## 14. The One-Paragraph Takeaway

TOAP is best understood as an **operating-system-style messaging layer for LLM agents**, not another
agent framework and not a token-compression gimmick. Its three ideas that are actually worth
attention are the **two-plane architecture** (separate byte and token planes for separate readers),
the **reference materialization spectrum** (INLINE → CTX_REF → KV_BRIDGE with graceful fallback,
spanning the token and tensor planes), and **capability-lattice security attached to context**
(provenance and allowed-operations travel with every reference). "CTX references" themselves are old
(blackboard systems, the 1980s); the contribution is the combination, the honest measurement, and a
working, tested implementation.

---

*See also: [`paper/main.pdf`](../paper/main.pdf) (full method + results), [`docs/protocol_v1.md`](protocol_v1.md)
(exact wire contract), [`docs/security_model.md`](security_model.md), [`docs/decisions.md`](decisions.md),
and [`docs/claims.md`](claims.md) (the measured-claims ledger).*
