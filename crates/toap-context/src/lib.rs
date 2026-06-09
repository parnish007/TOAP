//! Shared context store — the engine behind TOAP's strongest pillar (context-by-ID dedup, ~6×).
//!
//! Content is stored once and referenced by numeric `CTX:N`; only the ID travels on the wire.
//! This crate also defines:
//!   * the **Reference Materialization Spectrum (N3)** — one logical reference, three ways to
//!     materialize it, chosen by a cost model (`choose_materialization`);
//!   * per-context **ACL** (read/write/delete), **taint** (provenance), **TTL** + GC, and a
//!     **delta log** with field-level patches (the `DLT` message type).
//! See `research.md` §8.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// ACL
// ---------------------------------------------------------------------------

/// Read/write/delete permission set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Perms {
    pub r: bool,
    pub w: bool,
    pub d: bool,
}

impl Perms {
    fn from_str(s: &str) -> Perms {
        let mut p = Perms::default();
        for c in s.chars() {
            match c {
                'r' => p.r = true,
                'w' => p.w = true,
                'd' => p.d = true,
                '*' => {
                    p.r = true;
                    p.w = true;
                    p.d = true;
                }
                _ => {}
            }
        }
        p
    }
    fn has(&self, want: char) -> bool {
        match want {
            'r' => self.r,
            'w' => self.w,
            'd' => self.d,
            _ => false,
        }
    }
}

/// Access control list. Format: `agentA:rw,agentB:r,*:r`. The owner always has full access.
#[derive(Debug, Clone, Default)]
pub struct Acl {
    rules: Vec<(String, Perms)>,
}

impl Acl {
    pub fn parse(s: &str) -> Acl {
        let mut rules = Vec::new();
        for rule in s.split(',') {
            let rule = rule.trim();
            if rule.is_empty() {
                continue;
            }
            if let Some((agent, perms)) = rule.split_once(':') {
                rules.push((agent.to_string(), Perms::from_str(perms)));
            }
        }
        Acl { rules }
    }

    /// Does `agent` have permission `want` ('r'/'w'/'d')? `owner` always passes.
    pub fn allows(&self, owner: &str, agent: &str, want: char) -> bool {
        if agent == owner {
            return true;
        }
        // agent-specific rules first, then wildcard.
        for (a, p) in &self.rules {
            if a == agent && p.has(want) {
                return true;
            }
        }
        for (a, p) in &self.rules {
            if a == "*" && p.has(want) {
                return true;
            }
        }
        false
    }
}

// ---------------------------------------------------------------------------
// Reference materialization (N3)
// ---------------------------------------------------------------------------

/// The N3 reference primitive: how a logical reference to shared content is materialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Materialization {
    /// Embed the bytes directly (costs tokens now; for tiny / single-use content).
    Inline(Vec<u8>),
    /// Send the ID; receiver fetches from the store (the dedup win).
    CtxRef(u32),
    /// Point at a reusable KV-cache (skip prefill) — only valid same-model/same-tokenizer.
    KvBridge { ctx: u32, model: String },
}

/// Inputs to the materialization cost model.
#[derive(Debug, Clone, Copy)]
pub struct RefDecision {
    /// Size of the content in bytes.
    pub content_len: usize,
    /// How many times this content will be referenced in the workload.
    pub ref_count: u32,
    /// Sender and receiver share the same model + tokenizer (enables KV_BRIDGE).
    pub same_model: bool,
    /// Receiver shares the broker's context store locally (CtxRef fetch is cheap).
    pub colocated_store: bool,
}

/// Choose how to materialize a reference. This is the orchestration-level cost model (N3): inline
/// tiny/one-shot content, reference reused content, and only bridge KV when the constraints hold.
///
/// Honest thresholds (tunable): inlining tiny content avoids a store round-trip; referencing pays
/// off as soon as content is reused or is large. KV_BRIDGE is gated behind same-model AND a long
/// shared context, because a KV cache is far larger than the text (see research.md §3).
pub fn choose_materialization(id: u32, content: &[u8], d: RefDecision) -> Materialization {
    const TINY: usize = 48; // bytes below which inlining beats a store round-trip
    const KV_MIN: usize = 8192; // only consider KV transfer for long shared contexts

    if d.same_model && d.content_len >= KV_MIN && d.ref_count >= 1 {
        return Materialization::KvBridge { ctx: id, model: "same".to_string() };
    }
    if d.content_len <= TINY && d.ref_count <= 1 {
        return Materialization::Inline(content.to_vec());
    }
    // Default: reference it. This is where TOAP's measured ~6× on repeated content comes from.
    Materialization::CtxRef(id)
}

// ---------------------------------------------------------------------------
// Capability lattice (N4) — replaces the earlier boolean taint with a provenance + allowed-capability
// model (CaMeL-style information flow). Origin forms a lattice Internal ⊒ External ⊒ User; each origin
// carries the set of capabilities that data may flow into. `permits` is the structural check the
// broker uses to refuse, e.g., routing user-originated content into an EXEC/EMAIL/PAY/DEL operation.
// ---------------------------------------------------------------------------

/// What an operation wants to do with content (mapped from the wire opcode).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Capability {
    Read,
    Summarize,
    Transform,
    Classify,
    Execute,
    Email,
    Pay,
    Delete,
}

impl Capability {
    /// Map a wire opcode to the capability it exercises (unknown ops are treated as Execute = strict).
    pub fn for_op(op: &str) -> Capability {
        match op {
            "GET" | "ANS" | "RET" | "LST" => Capability::Read,
            "SUM" => Capability::Summarize,
            "XFM" | "TRN" | "MRG" | "SET" | "PATCH" => Capability::Transform,
            "CLS" | "VLD" | "CMP" => Capability::Classify,
            "DEL" => Capability::Delete,
            "EMAIL" => Capability::Email,
            "PAY" => Capability::Pay,
            "EXEC" | "SHELL" => Capability::Execute,
            _ => Capability::Execute,
        }
    }
}

/// Where content came from (the lattice level).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Internal,
    External,
    User,
}

/// Provenance tag carried by a context: its origin plus the capabilities its data may flow into.
#[derive(Debug, Clone)]
pub struct Provenance {
    pub origin: Origin,
    pub allowed: HashSet<Capability>,
}

impl Default for Provenance {
    fn default() -> Self {
        Provenance::external()
    }
}

impl Provenance {
    fn set(caps: &[Capability]) -> HashSet<Capability> {
        caps.iter().copied().collect()
    }

    /// Trusted internal content: all capabilities permitted.
    pub fn internal() -> Self {
        Provenance {
            origin: Origin::Internal,
            allowed: Self::set(&[
                Capability::Read, Capability::Summarize, Capability::Transform, Capability::Classify,
                Capability::Execute, Capability::Email, Capability::Pay, Capability::Delete,
            ]),
        }
    }

    /// External (third-party) content: read/summarize/transform/classify only — no side effects.
    pub fn external() -> Self {
        Provenance {
            origin: Origin::External,
            allowed: Self::set(&[
                Capability::Read, Capability::Summarize, Capability::Transform, Capability::Classify,
            ]),
        }
    }

    /// User-originated content: the most restricted — read/summarize/classify, no transform or effects.
    pub fn user() -> Self {
        Provenance {
            origin: Origin::User,
            allowed: Self::set(&[Capability::Read, Capability::Summarize, Capability::Classify]),
        }
    }

    /// Does this provenance permit `cap`? (`elevated` is a broker-policy override.)
    pub fn permits(&self, cap: Capability, elevated: bool) -> bool {
        elevated || self.allowed.contains(&cap)
    }

    /// True if the content is not trusted-internal (the boolean-taint compatibility view).
    pub fn is_tainted(&self) -> bool {
        self.origin != Origin::Internal
    }
}

// ---------------------------------------------------------------------------
// In-band cache coordinate (N4) — lets a reference cooperate with provider prefix-caching by marking
// which references belong in the stable cacheable prefix vs the volatile suffix.
// ---------------------------------------------------------------------------

/// Where a referenced span sits in a provider's KV/prefix cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheCoord {
    pub provider: String,
    pub breakpoint: u32,
    pub position: u32,
    /// Stable content belongs in the cacheable prefix; volatile content in the suffix.
    pub stable: bool,
}

/// Order references so stable (cacheable-prefix) ones precede volatile ones, preserving relative
/// order within each group. Laying messages out this way lets provider prefix-caching and TOAP
/// dedup compound instead of the protocol invalidating the cache.
pub fn cache_layout(refs: &[(u32, bool)]) -> Vec<u32> {
    let mut stable: Vec<u32> = refs.iter().filter(|(_, s)| *s).map(|(id, _)| *id).collect();
    let volatile: Vec<u32> = refs.iter().filter(|(_, s)| !*s).map(|(id, _)| *id).collect();
    stable.extend(volatile);
    stable
}

// ---------------------------------------------------------------------------
// KV-bridge transport (N3, optional) — the decision/fallback policy is implemented and tested here;
// the actual tensor transfer requires a model runtime and is provided by an external `KvTransport`.
// With no runtime (NoopKvTransport) the policy gracefully degrades KV_BRIDGE -> CTX_REF -> INLINE.
// ---------------------------------------------------------------------------

/// Abstracts the (model-runtime-specific) ability to obtain a reusable KV handle for a context.
pub trait KvTransport {
    /// Return a handle id if a reusable KV cache for `ctx` under `model` is available, else `None`.
    fn fetch(&self, ctx: u32, model: &str) -> Option<String>;
}

/// No model runtime available: KV transfer always declines, forcing graceful fallback.
pub struct NoopKvTransport;
impl KvTransport for NoopKvTransport {
    fn fetch(&self, _ctx: u32, _model: &str) -> Option<String> {
        None
    }
}

/// Constraint-guarded materialization with graceful fallback. Tries KV_BRIDGE only when the cost
/// model and constraints allow AND the transport actually has a handle; otherwise falls back to the
/// text-plane choice (CTX_REF / INLINE). Always benchmark against text+prefix-cache, never re-prefill.
pub fn materialize_with_fallback(
    id: u32,
    content: &[u8],
    d: RefDecision,
    model: &str,
    kv: &dyn KvTransport,
) -> Materialization {
    const KV_MIN: usize = 8192;
    if d.same_model && d.content_len >= KV_MIN {
        if kv.fetch(id, model).is_some() {
            return Materialization::KvBridge { ctx: id, model: model.to_string() };
        }
        // transport declined (e.g. no runtime, RoPE-offset/tokenizer mismatch) -> fall back.
    }
    // Fall back to the text-plane choice. Disable the KV branch so a declined bridge does not
    // get re-selected by the chooser (graceful degradation KV_BRIDGE -> CTX_REF -> INLINE).
    let text_only = RefDecision { same_model: false, ..d };
    choose_materialization(id, content, text_only)
}

// ---------------------------------------------------------------------------
// Context entry + store
// ---------------------------------------------------------------------------

/// One field-level delta (the `DLT`/`PATCH` message).
#[derive(Debug, Clone)]
pub struct Delta {
    pub field: String,
    pub value: String,
    pub at: u64,
    pub by: String,
}

/// One stored context, with the metadata needed for ACL, taint, TTL, and deltas.
#[derive(Debug, Clone)]
pub struct ContextEntry {
    pub id: u32,
    /// Broker-derived owner (the authenticated agent that created it) — never client-claimed.
    pub owner: String,
    pub data: Vec<u8>,
    /// True for externally/user-originated content. Default-taint is the safe policy (CaMeL-style).
    /// Kept for compatibility; the richer view is `provenance` (capability lattice, N4).
    pub tainted: bool,
    /// Capability-lattice provenance (N4): origin + the capabilities this data may flow into.
    pub provenance: Provenance,
    pub acl: Acl,
    pub created_at: u64,
    /// Absolute expiry time (0 = never).
    pub expires_at: u64,
    /// Optimistic-lock / change counter, bumped by every delta.
    pub version: u32,
    /// Field-level state (updated by deltas).
    pub fields: HashMap<String, String>,
    /// Append-only change log (most recent last; capped).
    pub delta_log: Vec<Delta>,
}

impl ContextEntry {
    pub fn is_expired(&self, now: u64) -> bool {
        self.expires_at != 0 && now >= self.expires_at
    }
}

const MAX_DELTAS: usize = 100;

/// Result of an access-controlled operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Ok,
    NoCtx,
    NoPerm,
}

/// In-memory context store (Phase-1 backend). Single-node; Redis/mmap backends come later.
#[derive(Debug, Default)]
pub struct ContextStore {
    next: u32,
    map: HashMap<u32, ContextEntry>,
}

impl ContextStore {
    pub fn new() -> Self {
        ContextStore { next: 1, map: HashMap::new() }
    }

    /// Store new content, returning its assigned numeric context ID.
    /// `tainted` is the compatibility flag; provenance is derived (tainted -> External, else Internal).
    pub fn set(
        &mut self,
        owner: &str,
        data: Vec<u8>,
        tainted: bool,
        acl: Acl,
        ttl_secs: u32,
    ) -> u32 {
        let prov = if tainted { Provenance::external() } else { Provenance::internal() };
        self.set_with_provenance(owner, data, prov, acl, ttl_secs)
    }

    /// Store new content with an explicit capability-lattice provenance (N4).
    pub fn set_with_provenance(
        &mut self,
        owner: &str,
        data: Vec<u8>,
        provenance: Provenance,
        acl: Acl,
        ttl_secs: u32,
    ) -> u32 {
        let id = self.next;
        self.next += 1;
        let created = now_secs();
        let expires_at = if ttl_secs == 0 { 0 } else { created + ttl_secs as u64 };
        let tainted = provenance.is_tainted();
        self.map.insert(
            id,
            ContextEntry {
                id,
                owner: owner.to_string(),
                data,
                tainted,
                provenance,
                acl,
                created_at: created,
                expires_at,
                version: 0,
                fields: HashMap::new(),
                delta_log: Vec::new(),
            },
        );
        id
    }

    /// Raw lookup (no ACL, no expiry check). Prefer `get_for`.
    pub fn get(&self, id: u32) -> Option<&ContextEntry> {
        self.map.get(&id)
    }

    /// ACL- and TTL-checked read.
    pub fn get_for(&self, id: u32, agent: &str) -> (Access, Option<&ContextEntry>) {
        match self.map.get(&id) {
            None => (Access::NoCtx, None),
            Some(e) if e.is_expired(now_secs()) => (Access::NoCtx, None),
            Some(e) => {
                if e.acl.allows(&e.owner, agent, 'r') {
                    (Access::Ok, Some(e))
                } else {
                    (Access::NoPerm, None)
                }
            }
        }
    }

    /// ACL-checked delete.
    pub fn delete_for(&mut self, id: u32, agent: &str) -> Access {
        match self.map.get(&id) {
            None => Access::NoCtx,
            Some(e) => {
                if e.acl.allows(&e.owner, agent, 'd') {
                    self.map.remove(&id);
                    Access::Ok
                } else {
                    Access::NoPerm
                }
            }
        }
    }

    /// ACL-checked field-level delta (the `DLT`/`PATCH` operation). Returns new version on success.
    pub fn patch_for(&mut self, id: u32, agent: &str, field: &str, value: &str) -> (Access, u32) {
        match self.map.get_mut(&id) {
            None => (Access::NoCtx, 0),
            Some(e) => {
                if !e.acl.allows(&e.owner, agent, 'w') {
                    return (Access::NoPerm, 0);
                }
                e.fields.insert(field.to_string(), value.to_string());
                e.delta_log.push(Delta {
                    field: field.to_string(),
                    value: value.to_string(),
                    at: now_secs(),
                    by: agent.to_string(),
                });
                if e.delta_log.len() > MAX_DELTAS {
                    let excess = e.delta_log.len() - MAX_DELTAS;
                    e.delta_log.drain(0..excess);
                }
                e.version += 1;
                (Access::Ok, e.version)
            }
        }
    }

    /// Evict expired contexts; returns the number removed.
    pub fn gc(&mut self) -> usize {
        let now = now_secs();
        let before = self.map.len();
        self.map.retain(|_, e| !e.is_expired(now));
        before - self.map.len()
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_get_roundtrip() {
        let mut s = ContextStore::new();
        let id = s.set("agentA", b"hello world".to_vec(), true, Acl::parse("*:r"), 0);
        assert_eq!(id, 1);
        let (acc, e) = s.get_for(id, "agentB");
        assert_eq!(acc, Access::Ok);
        assert_eq!(e.unwrap().data, b"hello world");
    }

    #[test]
    fn acl_denies_unauthorized_read() {
        let mut s = ContextStore::new();
        // Only agentA may read; no public access.
        let id = s.set("agentA", b"secret".to_vec(), false, Acl::parse("agentA:rwd"), 0);
        assert_eq!(s.get_for(id, "agentA").0, Access::Ok); // owner
        assert_eq!(s.get_for(id, "agentB").0, Access::NoPerm); // outsider denied
    }

    #[test]
    fn acl_wildcard_read_but_not_write() {
        let mut s = ContextStore::new();
        let id = s.set("agentA", b"doc".to_vec(), false, Acl::parse("*:r"), 0);
        assert_eq!(s.get_for(id, "agentZ").0, Access::Ok);
        assert_eq!(s.patch_for(id, "agentZ", "status", "done").0, Access::NoPerm);
        // owner can write
        assert_eq!(s.patch_for(id, "agentA", "status", "done").0, Access::Ok);
    }

    #[test]
    fn ttl_expiry() {
        let mut s = ContextStore::new();
        let id = s.set("a", b"x".to_vec(), false, Acl::parse("*:r"), 0);
        // Force expiry by rewriting expires_at into the past.
        s.map.get_mut(&id).unwrap().expires_at = 1;
        assert_eq!(s.get_for(id, "a").0, Access::NoCtx);
        assert_eq!(s.gc(), 1);
        assert!(s.is_empty());
    }

    #[test]
    fn deltas_accumulate_and_bump_version() {
        let mut s = ContextStore::new();
        let id = s.set("a", b"x".to_vec(), false, Acl::parse("a:rwd"), 0);
        assert_eq!(s.patch_for(id, "a", "status", "open").1, 1);
        assert_eq!(s.patch_for(id, "a", "status", "approved").1, 2);
        let e = s.get(id).unwrap();
        assert_eq!(e.fields.get("status").map(String::as_str), Some("approved"));
        assert_eq!(e.delta_log.len(), 2);
    }

    #[test]
    fn materialization_chooser() {
        // tiny + one-shot -> inline
        let m = choose_materialization(1, b"hi", RefDecision {
            content_len: 2, ref_count: 1, same_model: false, colocated_store: true,
        });
        assert!(matches!(m, Materialization::Inline(_)));
        // reused -> ctx ref
        let m = choose_materialization(1, b"some longer content here", RefDecision {
            content_len: 24, ref_count: 5, same_model: false, colocated_store: true,
        });
        assert_eq!(m, Materialization::CtxRef(1));
        // same-model + long -> KV bridge
        let big = vec![0u8; 9000];
        let m = choose_materialization(7, &big, RefDecision {
            content_len: big.len(), ref_count: 2, same_model: true, colocated_store: true,
        });
        assert!(matches!(m, Materialization::KvBridge { ctx: 7, .. }));
    }

    #[test]
    fn capability_lattice_contains_flow() {
        let user = Provenance::user();
        assert!(user.permits(Capability::Summarize, false)); // benign read/summarize ok
        assert!(!user.permits(Capability::Execute, false)); // user data cannot drive EXEC
        assert!(!user.permits(Capability::Pay, false));
        assert!(user.permits(Capability::Execute, true)); // elevated override
        assert!(Provenance::internal().permits(Capability::Pay, false)); // trusted: all caps
        assert!(Provenance::external().permits(Capability::Transform, false));
        assert!(!Provenance::external().permits(Capability::Delete, false));
        assert!(user.is_tainted() && !Provenance::internal().is_tainted());
    }

    #[test]
    fn opcode_capability_mapping() {
        assert_eq!(Capability::for_op("SUM"), Capability::Summarize);
        assert_eq!(Capability::for_op("EXEC"), Capability::Execute);
        assert_eq!(Capability::for_op("WAT"), Capability::Execute); // unknown -> strict
        assert_eq!(Capability::for_op("GET"), Capability::Read);
    }

    #[test]
    fn store_default_provenance() {
        let mut s = ContextStore::new();
        let tid = s.set("a", b"x".to_vec(), true, Acl::parse("*:r"), 0);
        let uid = s.set_with_provenance("a", b"y".to_vec(), Provenance::user(), Acl::parse("*:r"), 0);
        assert_eq!(s.get(tid).unwrap().provenance.origin, Origin::External);
        assert_eq!(s.get(uid).unwrap().provenance.origin, Origin::User);
        let clean = s.set("a", b"z".to_vec(), false, Acl::parse("*:r"), 0);
        assert_eq!(s.get(clean).unwrap().provenance.origin, Origin::Internal);
    }

    #[test]
    fn cache_layout_puts_stable_first() {
        // ids 1,3 stable; 2,4 volatile -> stable prefix then volatile suffix, order preserved
        let out = cache_layout(&[(1, true), (2, false), (3, true), (4, false)]);
        assert_eq!(out, vec![1, 3, 2, 4]);
    }

    #[test]
    fn kv_bridge_falls_back_without_runtime() {
        let big = vec![0u8; 9000];
        let d = RefDecision { content_len: big.len(), ref_count: 2, same_model: true, colocated_store: true };
        // No runtime: even a same-model long context degrades to CtxRef (graceful fallback).
        let m = materialize_with_fallback(7, &big, d, "llama3-8b", &NoopKvTransport);
        assert_eq!(m, Materialization::CtxRef(7));

        // A transport that has a handle returns KV_BRIDGE.
        struct Stub;
        impl KvTransport for Stub {
            fn fetch(&self, _c: u32, _m: &str) -> Option<String> { Some("h1".into()) }
        }
        let m2 = materialize_with_fallback(7, &big, d, "llama3-8b", &Stub);
        assert!(matches!(m2, Materialization::KvBridge { ctx: 7, .. }));

        // Cross-model (same_model=false) never bridges, even with a willing transport.
        let d2 = RefDecision { same_model: false, ..d };
        let m3 = materialize_with_fallback(7, &big, d2, "llama3-8b", &Stub);
        assert_eq!(m3, Materialization::CtxRef(7));
    }
}
