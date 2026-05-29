# TOAP V1 Text Protocol

This document is the phase 1 protocol contract for V1. It is intentionally conservative: the goal is deterministic parsing, broker-owned identity, and no benchmark claims before measurement.

## Design Summary

| Rule | Decision |
| --- | --- |
| Frame shape | `TYPE|MSG_ID|TARGET|PAYLOAD` |
| Source identity | Derived from broker session, not trusted from wire text. |
| Payload shape | Function-style: `OP(args)?key=value&key=value`. |
| Context key | Numeric `CTX:N` first; named aliases can come later. |
| Parser behavior | Strict for protocol syntax, permissive for ordinary context data. |

## Frame

```text
TYPE|MSG_ID|TARGET|PAYLOAD
```

| Field | Required | Format | Meaning |
| --- | --- | --- | --- |
| `TYPE` | Yes | Uppercase message type | `SYN`, `ACK`, `REQ`, `RES`, `ERR`, `EVT`, `DLT`, `BYE`, or `HBT`. |
| `MSG_ID` | Yes | Decimal `uint32` | Correlation ID. The broker echoes it in related responses. |
| `TARGET` | Yes | Agent ID, `broker`, or `*` | Destination. Broadcast is only valid for message types that allow it. |
| `PAYLOAD` | Yes | Function-style payload | Operation, arguments, and options. |

There is no trusted `SRC` field. The broker attaches source metadata internally after it reads from an authenticated session.

## Message Types

| Type | Direction | Purpose |
| --- | --- | --- |
| `SYN` | Agent to broker | Request a session and present requested capabilities. |
| `ACK` | Broker to agent | Accept a message or complete handshake. |
| `REQ` | Agent to agent through broker | Request an operation. |
| `RES` | Agent to requester through broker | Return operation result. |
| `ERR` | Any to requester through broker | Return a structured failure. |
| `EVT` | Broker or agent to subscribers | Notify about context or broker events. |
| `DLT` | Agent to broker or subscribers | Send a partial context update. |
| `BYE` | Agent to broker | Clean disconnect. |
| `HBT` | Agent to broker | Heartbeat. |

## Payload Grammar

```text
PAYLOAD = OP "(" ARGS? ")" ("?" OPTIONS)?
OP      = [A-Z][A-Z0-9_]{1,15}
ARGS    = ARG ("," ARG)*
ARG     = CTX_REF | TOKEN
CTX_REF = "CTX:" UINT
OPTIONS = OPTION ("&" OPTION)*
OPTION  = KEY "=" VALUE
```

Phase 2 implements this grammar in `protocol/parser.cpp` with tests in `protocol/tests/test_protocol.cpp`.

## Examples

```text
SYN|1|broker|HELLO()?agent_id=agentA&caps=SUM,GEN&version=1
ACK|1|agentA|SESSION()?session_id=sess_123&agent_id=agentA&trust=internal&version=1
REQ|100|agentB|SUM(CTX:42)?max_words=150&lang=en
RES|100|agentA|OK(CTX:87)
ERR|100|agentA|NOCTX(CTX:42)
DLT|101|agentB|PATCH(CTX:42)?field=status&value=approved
HBT|102|broker|PING()
BYE|103|broker|CLOSE()
```

## Why Function-Style Payloads

The earlier planning syntax used `SUM:CTX:42`. That is ambiguous because `CTX:42` already contains a colon. Function-style payloads keep positional arguments and options distinct:

```text
SUM(CTX:42)?max_words=150
```

This is easier to parse, easier to document, and easier to fuzz.

## Context References

Canonical context references are numeric:

```text
CTX:1
CTX:42
CTX:4294967295
```

Named aliases are deferred. If added later, they should map to numeric canonical IDs inside the broker or context store.

## Source Identity

The broker stores session metadata:

```text
session_id
agent_id
capabilities
protocol_version
trust_level
created_at
expires_at
```

When a message arrives, the broker uses the connection to identify the source session. If a payload contains `src=agentX`, that value is ordinary data and must not affect routing, ACL checks, logs, or rate limits.

## Validation Rules

The parser must reject:

- Wrong field count.
- Empty required fields.
- Unknown message type.
- Non-numeric `MSG_ID`.
- `MSG_ID` outside `uint32`.
- Invalid target name.
- Invalid payload grammar.
- Invalid context reference.
- Unescaped frame delimiter inside a field.

The parser must not reject ordinary content solely because it contains SQL, markdown, HTML, code, or prompt-like text. Content trust is handled through taint metadata and broker policy.

## Implemented Operations (v1.0)

Broker-handled operations (`TARGET = broker`):

| Op | Frame example | Effect |
| --- | --- | --- |
| `SET` | `REQ|n|broker|SET()?data=...&acl=*:r&ttl=3600&taint=true` | Store content; returns `OK(CTX:id)`. Owner = broker-derived session. |
| `GET` | `REQ|n|broker|GET(CTX:42)` | ACL-checked read; returns `OK(CTX:42)?data=...` or `ERR NOCTX/NOPERM`. |
| `DEL` | `REQ|n|broker|DEL(CTX:42)` | ACL-checked delete (restricted op; blocked on tainted context). |
| `SUB` | `REQ|n|broker|SUB(CTX:42)` | Subscribe to change events for a context. |
| `PATCH` | `DLT|n|broker|PATCH(CTX:42)?field=status&value=approved` | Field-level delta; bumps version; fans out `EVT` to subscribers. |

Events to subscribers:

```text
EVT|0|<subscriber>|CHANGED(CTX:42)?field=status&value=approved&version=3
```

Agent-to-agent operations are routed by `TARGET = <agent_id>` (e.g. `SUM`, `CLS`, `ANS`). The broker
correlates the returning `RES`/`ERR` to the requester by `MSG_ID`, so a responder never learns (or
can spoof) the requester's identity.

### Security enforcement (broker-side)

- **ACL** per context: `agentA:rwd,agentB:r,*:r`. The owner always has full access. Default `*:r`.
- **Taint**: external/user content is tainted by default (`taint=true`). Restricted ops
  (`EXEC,EMAIL,SHELL,PAY,DEL`) are refused against tainted context (`ERR NOPERM reason=tainted_context`).
- **Rate limiting**: per-agent token bucket (`ERR RATE`).
- **Replay**: duplicate request `MSG_ID` within a session is rejected (`ERR REPLAY`). Full
  cross-reconnect replay protection (signed nonces) is future work.
- **TTL**: contexts may expire; expired reads return `NOCTX`; `gc()` evicts them.

## Parser Contract

The implementation exposes:

```text
parse_frame(raw) -> ToapMessage | ParseError
encode_frame(ToapMessage) -> string
parse_payload(raw_payload) -> ParsedPayload | ParseError
encode_payload(ParsedPayload) -> string
```

Round-trip tests must exist before broker integration begins.
