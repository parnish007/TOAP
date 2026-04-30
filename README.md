# TOAP

TOAP means Token-Optimized Agent Protocol. It is planned as a communication layer for multi-agent LLM systems where agents pass compact, structured messages and shared context references instead of repeatedly sending verbose natural language or JSON payloads.

This repository is currently in phase 1: project foundation and specification cleanup. The implementation is not complete yet.

## Scope

TOAP is for agent-to-agent coordination. It is designed to reduce repeated message overhead by combining:

- Compact operation messages, such as `SUM(CTX:42)?max_words=150`.
- A shared context store, where large content is stored once and referenced by ID.
- Broker-routed sessions, where source identity is derived from the authenticated connection.
- ACL and taint rules enforced by the broker.
- Reproducible benchmarks that separate wire bytes, protocol tokens, model input tokens, and latency.

TOAP is not intended to replace MCP, A2A, LangChain, AutoGen, CrewAI, or an LLM runtime. It can sit under or beside those systems as a lower-level payload and routing protocol.

## What This Project Is For

The project targets systems where several agents collaborate on the same documents, tasks, or intermediate results. In those systems, message cost often grows because the same context is copied between agents again and again.

TOAP's intended role is to:

- Store shared data once.
- Route compact requests between agents.
- Let agents fetch only the context they are allowed to read.
- Send deltas instead of full state when possible.
- Make protocol behavior deterministic enough for C++, Python, and LLM-backed agents.
- Avoid treating peer-agent messages as trusted.

## What TOAP Does Not Claim Yet

TOAP can reduce wire bytes and protocol/message tokens when agents send references instead of repeated content.

TOAP does not automatically reduce total LLM inference tokens when an agent still has to fetch a context and place it into a model prompt. Those savings require additional strategies such as summarization, retrieval, caching, or a future KV cache bridge.

Until benchmarks exist, all numeric savings are targets or hypotheses, not measured project results. Measured claims will live in `docs/claims.md`.

## Planned End Result

The target end product is:

- A C++ protocol library for V1 text encoding and V2 binary encoding.
- A TCP broker that manages sessions, identity, routing, ACLs, taint, and rate limits.
- A shared context store with TTL, metadata, deltas, and events.
- A Python SDK for writing agents without manually formatting wire messages.
- Example agents for summarization, classification, orchestration, and delta subscriptions.
- A benchmark suite with JSON, natural-language, and A2A-style JSON-RPC baselines.
- Documentation that clearly separates implemented behavior, measured results, and roadmap research.

## Current Phase 1 Deliverables

Phase 1 creates the repository foundation:

- `.gitignore`
- `CMakeLists.txt`
- `requirements-dev.txt`
- `TEST_PLAN.md`
- `docs/protocol_v1.md`
- `docs/security_model.md`
- `docs/claims.md`
- `docs/decisions.md`

The corrected V1 design is documented before code is written so the implementation does not inherit the known mistakes from `blueprint.md`.

## Corrected V1 Protocol Direction

The V1 frame is:

```text
TYPE|MSG_ID|TARGET|PAYLOAD
```

The broker derives source identity from the authenticated connection/session. Agents do not provide a trusted `SRC` field.

Payloads use function-style syntax to avoid ambiguity:

```text
SUM(CTX:42)?max_words=150&lang=en
CMP(CTX:11,CTX:22)
SET(CTX:99)?data=hello_world
```

## Build Status

There is no compiled implementation yet. The root CMake project is present so later phases can add C++ targets without restructuring the repository.

Configure check:

```bash
cmake -S . -B build
```

Python development dependencies will be used once the SDK and tests exist:

```bash
python -m pip install -r requirements-dev.txt
```

## Roadmap

The full staged plan is in `phases.md`.

Build phases:

1. Repository foundation and spec cleanup.
2. Protocol parser and encoder.
3. Broker skeleton and connection-bound sessions.
4. Routing and request/response flow.
5. Shared context store.
6. ACL, taint, and security policy.
7. Python SDK.
8. Delta updates, events, and subscriptions.
9. V2 binary encoding and negotiation.
10. Benchmarks, docs polish, and release candidate.

Testing phases:

1. Protocol unit tests.
2. Broker and session integration tests.
3. Context store, ACL, and taint tests.
4. SDK and end-to-end workflow tests.
5. Benchmarks, fuzzing, security, and release gates.

