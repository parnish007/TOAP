//! Shared context store — the engine behind TOAP's strongest pillar (context-by-ID dedup, ~6×).
//!
//! Content is stored once and referenced by numeric `CTX:N`; only the ID travels on the wire.
//! This crate also defines:
//!   * the **Reference Materialization Spectrum (N3)** — one logical reference, three ways to
//!     materialize it, chosen by a cost model (`choose_materialization`);
//!   * per-context **ACL** (read/write/delete), **taint** (provenance), **TTL** + GC, and a
//!     **delta log** with field-level patches (the `DLT` message type).
//! See `research.md` §8.

use std::collections::HashMap;
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
    pub tainted: bool,
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
    pub fn set(
        &mut self,
        owner: &str,
        data: Vec<u8>,
        tainted: bool,
        acl: Acl,
        ttl_secs: u32,
    ) -> u32 {
        let id = self.next;
        self.next += 1;
        let created = now_secs();
        let expires_at = if ttl_secs == 0 { 0 } else { created + ttl_secs as u64 };
        self.map.insert(
            id,
            ContextEntry {
                id,
                owner: owner.to_string(),
                data,
                tainted,
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
}
