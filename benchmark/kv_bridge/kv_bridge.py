#!/usr/bin/env python3
"""
TOAP KV-bridge — minimal but REAL implementation.

The idea (TOAP's KV_BRIDGE materialization): instead of re-sending a shared context as text and making
the next agent re-prefill it, transfer the already-computed key/value cache for that context. The
receiver then only prefills its own (short) query suffix and decodes.

This module implements the genuinely-correct case: SAME MODEL, SAME TOKENIZER, PREFIX REUSE. The shared
context's KV is computed at absolute positions 0..n; the query is appended at positions n.., so RoPE /
absolute positions stay valid (this is the constraint the literature flags — we respect it instead of
pretending it away). Splicing a cache baked at different positions is explicitly NOT done here.

What it provides:
  - extract_kv(model, prefix_ids)            -> past_key_values for the shared context (the "produce")
  - serialize_kv / deserialize_kv            -> bytes over the wire (the "transfer"); measures real size
  - kv_byte_size(kv)                          -> honest cost accounting
  - generate_recompute(...)                  -> baseline: prefill prefix+query from scratch
  - generate_kv_bridge(...)                   -> reuse transferred prefix KV, prefill only the query

Run the benchmark with kv_bench.py. This file is import-only logic + a tiny self-check.
"""
from __future__ import annotations
import io
import time
from dataclasses import dataclass

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer
from transformers.cache_utils import DynamicCache


def load(model_name: str, dtype=None, device=None):
    device = device or ("cuda" if torch.cuda.is_available() else "cpu")
    if dtype is None:
        dtype = torch.float16 if device == "cuda" else torch.float32
    tok = AutoTokenizer.from_pretrained(model_name)
    model = AutoModelForCausalLM.from_pretrained(model_name, torch_dtype=dtype).to(device).eval()
    return tok, model, device


# ---------------------------------------------------------------------------
# KV extraction / transfer
# ---------------------------------------------------------------------------

def _to_legacy(past):
    """Normalize a DynamicCache or legacy tuple to the legacy tuple-of-(k,v) form."""
    if hasattr(past, "to_legacy_cache"):
        return past.to_legacy_cache()
    return past


@torch.no_grad()
def extract_kv(model, prefix_ids):
    """Prefill the shared context once and return its KV cache (legacy tuple form)."""
    out = model(prefix_ids, use_cache=True)
    return _to_legacy(out.past_key_values)


def kv_byte_size(past_legacy) -> int:
    """Real size of the KV cache in bytes (sum of all key/value tensors)."""
    total = 0
    for layer in past_legacy:
        for t in layer:
            total += t.numel() * t.element_size()
    return total


def serialize_kv(past_legacy) -> bytes:
    """Serialize KV to bytes — the thing a real bridge would put on the wire / shm."""
    buf = io.BytesIO()
    torch.save([[k.contiguous().cpu(), v.contiguous().cpu()] for (k, v) in past_legacy], buf)
    return buf.getvalue()


def deserialize_kv(blob: bytes, device):
    """Rehydrate KV from bytes onto the target device and wrap as a DynamicCache."""
    buf = io.BytesIO(blob)
    # weights_only=True: the blob is only tensors; never unpickle arbitrary objects from the wire.
    legacy = torch.load(buf, map_location=device, weights_only=True)
    legacy = tuple((k.to(device), v.to(device)) for (k, v) in legacy)
    return DynamicCache.from_legacy_cache(legacy)


# ---------------------------------------------------------------------------
# Generation: baseline (recompute) vs KV-bridge (reuse prefix)
# ---------------------------------------------------------------------------

@dataclass
class GenResult:
    tokens: list[int]
    text: str
    prefill_ms: float
    total_ms: float


@torch.no_grad()
def _greedy(model, input_ids, past, new_tokens):
    """Greedy decode `new_tokens` steps starting from `input_ids` with optional `past`. Returns ids."""
    generated = []
    cur = input_ids
    cache = past
    for _ in range(new_tokens):
        out = model(cur, past_key_values=cache, use_cache=True)
        cache = out.past_key_values
        nxt = out.logits[:, -1, :].argmax(dim=-1, keepdim=True)
        generated.append(int(nxt))
        cur = nxt
    return generated


@torch.no_grad()
def generate_recompute(model, prefix_ids, query_ids, new_tokens, device):
    """Baseline: the receiver gets prefix+query as text and prefills ALL of it from scratch."""
    full = torch.cat([prefix_ids, query_ids], dim=1)
    if device == "cuda":
        torch.cuda.synchronize()
    t0 = time.perf_counter()
    out = model(full, use_cache=True)          # <- prefill over prefix+query (the wasted work)
    if device == "cuda":
        torch.cuda.synchronize()
    prefill_ms = (time.perf_counter() - t0) * 1000
    cache = out.past_key_values
    first = out.logits[:, -1, :].argmax(dim=-1, keepdim=True)
    rest = _greedy(model, first, cache, new_tokens - 1)
    toks = [int(first)] + rest
    if device == "cuda":
        torch.cuda.synchronize()
    total_ms = (time.perf_counter() - t0) * 1000
    return GenResult(toks, "", prefill_ms, total_ms)


@torch.no_grad()
def generate_kv_bridge(model, prefix_kv_blob, query_ids, new_tokens, device):
    """KV-bridge: the receiver gets the prefix KV (already computed) + the query text.
    It prefills ONLY the query, reusing the transferred prefix cache."""
    if device == "cuda":
        torch.cuda.synchronize()
    t0 = time.perf_counter()
    cache = deserialize_kv(prefix_kv_blob, device)        # transfer + rehydrate
    out = model(query_ids, past_key_values=cache, use_cache=True)   # <- prefill ONLY the query
    if device == "cuda":
        torch.cuda.synchronize()
    prefill_ms = (time.perf_counter() - t0) * 1000
    cache = out.past_key_values
    first = out.logits[:, -1, :].argmax(dim=-1, keepdim=True)
    rest = _greedy(model, first, cache, new_tokens - 1)
    toks = [int(first)] + rest
    if device == "cuda":
        torch.cuda.synchronize()
    total_ms = (time.perf_counter() - t0) * 1000
    return GenResult(toks, "", prefill_ms, total_ms)


if __name__ == "__main__":
    # tiny self-check on CPU with the smallest model
    tok, model, device = load("sshleifer/tiny-gpt2", device="cpu")
    prefix = tok("The capital of France is Paris. " * 5, return_tensors="pt").input_ids.to(device)
    query = tok(" Question: what is the capital?", return_tensors="pt").input_ids.to(device)
    kv = extract_kv(model, prefix)
    blob = serialize_kv(kv)
    r = generate_recompute(model, prefix, query, 8, device)
    b = generate_kv_bridge(model, blob, query, 8, device)
    print("kv bytes:", kv_byte_size(kv), "| match:", r.tokens == b.tokens)
    print("recompute prefill ms:", round(r.prefill_ms, 2), "| bridge prefill ms:", round(b.prefill_ms, 2))
