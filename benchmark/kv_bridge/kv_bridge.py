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

We measure two costs separately and honestly:
  * COMPUTE win   = prefill latency of (recompute prefix+query)  vs (prefill ONLY the query).
  * TRANSFER cost = serialize + move + deserialize the KV tensors, and their byte size vs the text.
A real deployment co-locates producer and consumer (shared GPU / NVLink / RDMA), so the headline win is
the compute side; the transfer cost is reported so the trade-off is never hidden.

Robust across transformers versions: tolerates DynamicCache, legacy tuples, and layers whose key/value
are temporarily ``None``; GQA-aware (counts real KV-head tensors).
"""
from __future__ import annotations
import io
import time
from dataclasses import dataclass, field

import torch
from transformers import AutoModelForCausalLM, AutoTokenizer

try:
    from transformers.cache_utils import DynamicCache
except Exception:  # very old transformers
    DynamicCache = None


def load(model_name: str, dtype=None, device=None, load_in_4bit: bool = False):
    """Load tokenizer + model. Uses the modern ``dtype=`` arg; optional 4-bit via bitsandbytes."""
    device = device or ("cuda" if torch.cuda.is_available() else "cpu")
    if dtype is None:
        dtype = torch.float16 if device == "cuda" else torch.float32
    tok = AutoTokenizer.from_pretrained(model_name)
    kwargs = {}
    if load_in_4bit:
        from transformers import BitsAndBytesConfig
        kwargs["quantization_config"] = BitsAndBytesConfig(
            load_in_4bit=True, bnb_4bit_compute_dtype=dtype, bnb_4bit_quant_type="nf4")
        kwargs["device_map"] = "auto"
    else:
        kwargs["dtype"] = dtype          # modern transformers (replaces deprecated torch_dtype)
    try:
        model = AutoModelForCausalLM.from_pretrained(model_name, **kwargs)
    except TypeError:
        # older transformers that still want torch_dtype
        kwargs.pop("dtype", None)
        model = AutoModelForCausalLM.from_pretrained(model_name, torch_dtype=dtype, **kwargs)
    if not load_in_4bit:
        model = model.to(device)
    model.eval()
    return tok, model, device


def model_context_limit(tok, model, fallback: int = 4096) -> int:
    """Best-effort maximum sequence length the model+tokenizer support."""
    for attr in ("n_positions", "max_position_embeddings"):
        v = getattr(model.config, attr, None)
        if isinstance(v, int) and 0 < v < 1_000_000:
            return v
    v = getattr(tok, "model_max_length", None)
    if isinstance(v, int) and 0 < v < 1_000_000:
        return v
    return fallback


# ---------------------------------------------------------------------------
# KV extraction / transfer  (version-robust)
# ---------------------------------------------------------------------------

def _iter_layers(past):
    """Yield (key, value) per layer across transformers versions. Skips None placeholders.

    Handles: legacy tuple ((k,v),...); transformers 5.x DynamicCache (``.layers[i].keys/.values``);
    transformers 4.x DynamicCache (``.to_legacy_cache()`` or ``.key_cache/.value_cache``)."""
    if past is None:
        return
    # legacy tuple/list form: ((k, v), (k, v), ...)
    if isinstance(past, (tuple, list)):
        for layer in past:
            if layer is None:
                continue
            yield layer[0], layer[1]
        return
    # transformers 5.x: DynamicCache.layers -> list of DynamicLayer(.keys, .values)
    layers = getattr(past, "layers", None)
    if layers is not None:
        for layer in layers:
            k = getattr(layer, "keys", None)
            v = getattr(layer, "values", None)
            if k is not None and v is not None:
                yield k, v
        return
    # transformers 4.x: to_legacy_cache()
    if hasattr(past, "to_legacy_cache"):
        try:
            for layer in past.to_legacy_cache():
                if layer is not None:
                    yield layer[0], layer[1]
            return
        except Exception:
            pass
    # transformers 4.x: .key_cache / .value_cache lists
    kc = getattr(past, "key_cache", None)
    vc = getattr(past, "value_cache", None)
    if kc is not None and vc is not None:
        for k, v in zip(kc, vc):
            yield k, v


@torch.no_grad()
def extract_kv(model, prefix_ids):
    """Prefill the shared context once and return its KV cache (as the model produced it)."""
    out = model(prefix_ids, use_cache=True)
    return out.past_key_values


def _make_cache_on_device(layers, device):
    """Build a forward-compatible cache from (k, v) tensors already on `device`."""
    if DynamicCache is not None:
        try:
            cache = DynamicCache()
            for i, (k, v) in enumerate(layers):
                cache.update(k, v, i)
            return cache
        except Exception:
            pass
        if hasattr(DynamicCache, "from_legacy_cache"):
            try:
                return DynamicCache.from_legacy_cache(tuple(layers))
            except Exception:
                pass
    return tuple(layers)


def resident_cache_factory(past, device):
    """Return a callable producing a fresh GPU-resident cache cloned from `past` each call.

    KV tensors stay on-device (no host round-trip), so timing a query prefill against this isolates the
    pure COMPUTE win of skipping the prefix prefill (the co-located / shared-broker case)."""
    base = [(k.detach().contiguous(), v.detach().contiguous()) for (k, v) in _iter_layers(past)
            if k is not None and v is not None]

    def make():
        return _make_cache_on_device([(k.clone(), v.clone()) for (k, v) in base], device)
    return make


def kv_byte_size(past) -> int:
    """Real size of the KV cache in bytes (sum of all key/value tensors; GQA-aware, None-safe)."""
    total = 0
    for k, v in _iter_layers(past):
        for t in (k, v):
            if t is not None:
                total += t.numel() * t.element_size()
    return total


def kv_shape_info(past):
    """Return (n_layers, kv_heads, head_dim, seq_len) from the first real layer, for reporting."""
    for k, v in _iter_layers(past):
        if k is not None and k.dim() == 4:
            # [batch, kv_heads, seq, head_dim]
            return {"kv_heads": k.shape[1], "head_dim": k.shape[3], "seq_len": k.shape[2]}
    return {}


def serialize_kv(past) -> bytes:
    """Serialize KV to bytes — the thing a real bridge would put on the wire / shm."""
    layers = [[k.contiguous().cpu(), v.contiguous().cpu()] for (k, v) in _iter_layers(past)
              if k is not None and v is not None]
    buf = io.BytesIO()
    torch.save(layers, buf)
    return buf.getvalue()


def _make_cache_from_layers(layers, device):
    """Build a cache object the model's forward pass accepts, across transformers versions.

    layers: list of [key, value] tensors (one per decoder layer), already on CPU.
    Strategy that works on 4.x AND 5.x: create an empty DynamicCache and populate it with
    ``cache.update(k, v, layer_idx)`` — the public API used during generation. Falls back to
    ``from_legacy_cache`` (4.x) and finally to a bare tuple (very old)."""
    layers = [(k.to(device), v.to(device)) for (k, v) in layers]
    if DynamicCache is not None:
        try:
            cache = DynamicCache()
            for i, (k, v) in enumerate(layers):
                cache.update(k, v, i)            # public update API (4.x and 5.x)
            return cache
        except Exception:
            pass
        if hasattr(DynamicCache, "from_legacy_cache"):
            try:
                return DynamicCache.from_legacy_cache(tuple(layers))
            except Exception:
                pass
    return tuple(layers)


def deserialize_kv(blob: bytes, device):
    """Rehydrate KV from bytes onto the target device as a forward-pass-compatible cache."""
    buf = io.BytesIO(blob)
    # weights_only=True: the blob is only tensors; never unpickle arbitrary objects from the wire.
    layers = torch.load(buf, map_location="cpu", weights_only=True)
    return _make_cache_from_layers(layers, device)


# ---------------------------------------------------------------------------
# Generation: baseline (recompute) vs KV-bridge (reuse prefix)
# ---------------------------------------------------------------------------

@dataclass
class GenResult:
    tokens: list = field(default_factory=list)
    prefill_ms: float = 0.0
    transfer_ms: float = 0.0
    total_ms: float = 0.0


def _cache_len(cache) -> int:
    """Current sequence length held in a cache, across versions (for absolute positioning)."""
    if cache is None:
        return 0
    for attr in ("get_seq_length",):
        fn = getattr(cache, attr, None)
        if callable(fn):
            try:
                return int(fn())
            except Exception:
                pass
    # fall back: read the seq dim of the first key tensor
    for k, _v in _iter_layers(cache):
        if k is not None and k.dim() == 4:
            return int(k.shape[2])
    return 0


@torch.no_grad()
def _forward_at(model, input_ids, cache, device):
    """Forward `input_ids` on top of `cache`, placing them at the correct ABSOLUTE positions.

    Passing explicit cache_position + a full attention_mask keeps RoPE/abs-positions valid when the
    query is appended after a transferred prefix — this is what makes KV-bridge token-lossless. Falls
    back to a plain call on versions that infer position automatically."""
    past_len = _cache_len(cache)
    seq = input_ids.shape[1]
    try:
        cache_position = torch.arange(past_len, past_len + seq, device=device)
        attn = torch.ones((input_ids.shape[0], past_len + seq), dtype=torch.long, device=device)
        return model(input_ids, past_key_values=cache, use_cache=True,
                     cache_position=cache_position, attention_mask=attn)
    except TypeError:
        return model(input_ids, past_key_values=cache, use_cache=True)


@torch.no_grad()
def _greedy(model, input_ids, past, new_tokens, device):
    """Greedy decode `new_tokens` steps starting from `input_ids` with optional `past`."""
    generated, cur, cache = [], input_ids, past
    for _ in range(new_tokens):
        out = _forward_at(model, cur, cache, device)
        cache = out.past_key_values
        nxt = out.logits[:, -1, :].argmax(dim=-1, keepdim=True)
        generated.append(int(nxt))
        cur = nxt
    return generated


def _sync(device):
    if device == "cuda":
        torch.cuda.synchronize()


@torch.no_grad()
def _time_call(fn, device, warmup=3, iters=15):
    """Time a no-arg callable with GPU warmup + CUDA events (accurate) or perf_counter (CPU).

    Returns (median_ms, mean_ms, std_ms, samples). Warmup absorbs first-call kernel autotuning — the
    artifact that made naive single-shot timing report a fake <1x 'slowdown' at short prefixes."""
    for _ in range(warmup):
        fn()
    _sync(device)
    samples = []
    use_events = (device == "cuda")
    for _ in range(iters):
        if use_events:
            e0, e1 = torch.cuda.Event(enable_timing=True), torch.cuda.Event(enable_timing=True)
            e0.record(); fn(); e1.record(); torch.cuda.synchronize()
            samples.append(e0.elapsed_time(e1))
        else:
            t0 = time.perf_counter(); fn(); samples.append((time.perf_counter() - t0) * 1000)
    s = sorted(samples)
    n = len(s)
    median = s[n // 2] if n % 2 else (s[n // 2 - 1] + s[n // 2]) / 2
    mean = sum(s) / n
    std = (sum((x - mean) ** 2 for x in s) / n) ** 0.5
    return median, mean, std, samples


@torch.no_grad()
def measure_prefill_recompute(model, prefix_ids, query_ids, device, warmup=3, iters=15):
    """Time ONLY the prefill of (prefix+query) from scratch — the work KV-bridge avoids."""
    full = torch.cat([prefix_ids, query_ids], dim=1)
    return _time_call(lambda: model(full, use_cache=True), device, warmup, iters)


@torch.no_grad()
def measure_prefill_bridge_resident(model, factory, query_ids, device, warmup=3, iters=15):
    """Time ONLY the query prefill on a GPU-resident cache (co-located case): the pure compute win.
    `factory()` returns a fresh resident cache each call so reuse side-effects don't accumulate."""
    def step():
        _forward_at(model, query_ids, factory(), device)
    return _time_call(step, device, warmup, iters)


@torch.no_grad()
def measure_transfer(prefix_kv_blob, device, warmup=2, iters=8):
    """Time deserialize+rehydrate of the KV blob onto the GPU (the cross-node transfer tax)."""
    return _time_call(lambda: deserialize_kv(prefix_kv_blob, device), device, warmup, iters)


@torch.no_grad()
def check_lossless(model, prefix_ids, query_ids, prefix_kv_blob, device, new_tokens=8):
    """Greedy-decode new_tokens via recompute vs bridge; return (match: bool, recompute_tokens)."""
    full = torch.cat([prefix_ids, query_ids], dim=1)
    out = model(full, use_cache=True)
    f = out.logits[:, -1, :].argmax(dim=-1, keepdim=True)
    rec = [int(f)] + (_greedy(model, f, out.past_key_values, new_tokens - 1, device) if new_tokens > 1 else [])
    cache = deserialize_kv(prefix_kv_blob, device)
    ob = _forward_at(model, query_ids, cache, device)
    fb = ob.logits[:, -1, :].argmax(dim=-1, keepdim=True)
    bri = [int(fb)] + (_greedy(model, fb, ob.past_key_values, new_tokens - 1, device) if new_tokens > 1 else [])
    return rec == bri, rec


@torch.no_grad()
def generate_recompute(model, prefix_ids, query_ids, new_tokens, device):
    """Baseline: the receiver gets prefix+query as text and prefills ALL of it from scratch."""
    full = torch.cat([prefix_ids, query_ids], dim=1)
    _sync(device); t0 = time.perf_counter()
    out = model(full, use_cache=True)                  # prefill over prefix+query (the wasted work)
    _sync(device); prefill_ms = (time.perf_counter() - t0) * 1000
    first = out.logits[:, -1, :].argmax(dim=-1, keepdim=True)
    rest = _greedy(model, first, out.past_key_values, new_tokens - 1, device) if new_tokens > 1 else []
    _sync(device); total_ms = (time.perf_counter() - t0) * 1000
    return GenResult([int(first)] + rest, prefill_ms, 0.0, total_ms)


@torch.no_grad()
def generate_kv_bridge(model, prefix_kv_blob, query_ids, new_tokens, device):
    """KV-bridge: the receiver gets the prefix KV (already computed) + the query text.
    It prefills ONLY the query, reusing the transferred prefix cache. Transfer (deserialize) time is
    measured SEPARATELY from prefill, so the compute win and the transfer cost are not conflated.
    The query is forwarded at ABSOLUTE positions prefix_len.. (via _forward_at) so RoPE/positions stay
    valid and outputs stay token-lossless vs recompute."""
    _sync(device); t_tr = time.perf_counter()
    cache = deserialize_kv(prefix_kv_blob, device)     # transfer + rehydrate
    _sync(device); transfer_ms = (time.perf_counter() - t_tr) * 1000

    t0 = time.perf_counter()
    out = _forward_at(model, query_ids, cache, device)   # prefill ONLY the query, at correct positions
    _sync(device); prefill_ms = (time.perf_counter() - t0) * 1000
    first = out.logits[:, -1, :].argmax(dim=-1, keepdim=True)
    rest = _greedy(model, first, out.past_key_values, new_tokens - 1, device) if new_tokens > 1 else []
    _sync(device); total_ms = (time.perf_counter() - t0) * 1000 + transfer_ms
    return GenResult([int(first)] + rest, prefill_ms, transfer_ms, total_ms)


if __name__ == "__main__":
    # tiny self-check on CPU with the smallest model
    tok, model, device = load("sshleifer/tiny-gpt2", device="cpu")
    prefix = tok("The capital of France is Paris. " * 5, return_tensors="pt").input_ids.to(device)
    query = tok(" Question: what is the capital?", return_tensors="pt").input_ids.to(device)
    kv = extract_kv(model, prefix)
    blob = serialize_kv(kv)
    r = generate_recompute(model, prefix, query, 8, device)
    b = generate_kv_bridge(model, blob, query, 8, device)
    print("kv bytes:", kv_byte_size(kv), "| shape:", kv_shape_info(kv), "| match:", r.tokens == b.tokens)
    print("recompute prefill ms:", round(r.prefill_ms, 2),
          "| bridge prefill ms:", round(b.prefill_ms, 2), "| transfer ms:", round(b.transfer_ms, 2))
