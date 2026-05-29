//! TOAP security primitives, enforced by the broker (never by self-claiming clients).
//!
//! - [`RateLimiter`]: per-agent token bucket (anti-DoS).
//! - [`ReplayGuard`]: rejects duplicate request IDs within a session (basic replay defense).
//! - [`TaintPolicy`]: refuses to use tainted (user-originated) context in restricted operations —
//!   structural containment of prompt-injection propagation (CaMeL-style information flow).
//!
//! Honest scope: this is *containment, not prevention*. Full replay protection across reconnects
//! needs signed per-frame nonces bound to the session (future work) — see research.md §6.

use std::collections::{HashMap, HashSet};
use std::time::Instant;

// ---------------------------------------------------------------------------
// Rate limiting (token bucket per agent)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy)]
pub struct RateConfig {
    pub capacity: f64,
    pub refill_per_sec: f64,
}

impl Default for RateConfig {
    fn default() -> Self {
        RateConfig { capacity: 500.0, refill_per_sec: 100.0 }
    }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

#[derive(Default)]
pub struct RateLimiter {
    cfg: RateConfig,
    buckets: HashMap<String, Bucket>,
}

impl RateLimiter {
    pub fn new(cfg: RateConfig) -> Self {
        RateLimiter { cfg, buckets: HashMap::new() }
    }

    /// Returns true if the agent is allowed to send one more message now.
    pub fn allow(&mut self, agent: &str) -> bool {
        self.allow_at(agent, Instant::now())
    }

    fn allow_at(&mut self, agent: &str, now: Instant) -> bool {
        let cfg = self.cfg;
        let b = self
            .buckets
            .entry(agent.to_string())
            .or_insert_with(|| Bucket { tokens: cfg.capacity, last: now });
        let elapsed = now.saturating_duration_since(b.last).as_secs_f64();
        b.tokens = (b.tokens + elapsed * cfg.refill_per_sec).min(cfg.capacity);
        b.last = now;
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Replay guard (per-session duplicate request-id rejection)
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct ReplayGuard {
    seen: HashMap<String, HashSet<u32>>,
}

impl ReplayGuard {
    pub fn new() -> Self {
        ReplayGuard { seen: HashMap::new() }
    }

    /// Record a request id for a session. Returns true if it is NEW (accept), false if a replay.
    pub fn accept(&mut self, session: &str, req_id: u32) -> bool {
        self.seen.entry(session.to_string()).or_default().insert(req_id)
    }

    /// Drop all state for a session (on disconnect).
    pub fn forget(&mut self, session: &str) {
        self.seen.remove(session);
    }
}

// ---------------------------------------------------------------------------
// Taint policy
// ---------------------------------------------------------------------------

/// Operations that must not run against tainted (user-originated) context without elevation.
pub struct TaintPolicy {
    restricted: HashSet<String>,
}

impl Default for TaintPolicy {
    fn default() -> Self {
        let restricted = ["EXEC", "EMAIL", "SHELL", "PAY", "DEL"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        TaintPolicy { restricted }
    }
}

impl TaintPolicy {
    pub fn new<I: IntoIterator<Item = String>>(ops: I) -> Self {
        TaintPolicy { restricted: ops.into_iter().collect() }
    }

    /// Is it allowed to run `op` against a context whose taint flag is `tainted`?
    /// Restricted ops on tainted content are denied unless `elevated`.
    pub fn allows(&self, op: &str, tainted: bool, elevated: bool) -> bool {
        if !tainted || elevated {
            return true;
        }
        !self.restricted.contains(op)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn rate_limiter_blocks_burst_then_refills() {
        let mut rl = RateLimiter::new(RateConfig { capacity: 3.0, refill_per_sec: 1.0 });
        let t0 = Instant::now();
        assert!(rl.allow_at("a", t0));
        assert!(rl.allow_at("a", t0));
        assert!(rl.allow_at("a", t0));
        assert!(!rl.allow_at("a", t0)); // bucket empty
        // after ~1.1s one token refills
        assert!(rl.allow_at("a", t0 + Duration::from_millis(1100)));
        // a different agent has its own bucket
        assert!(rl.allow_at("b", t0));
    }

    #[test]
    fn replay_guard_rejects_duplicates() {
        let mut g = ReplayGuard::new();
        assert!(g.accept("sess1", 100));
        assert!(g.accept("sess1", 101));
        assert!(!g.accept("sess1", 100)); // replay
        assert!(g.accept("sess2", 100)); // different session ok
    }

    #[test]
    fn taint_policy_contains_restricted_ops() {
        let p = TaintPolicy::default();
        assert!(p.allows("SUM", true, false)); // benign op on tainted data: ok
        assert!(!p.allows("EXEC", true, false)); // restricted op on tainted data: denied
        assert!(p.allows("EXEC", true, true)); // elevated: allowed
        assert!(p.allows("EXEC", false, false)); // untainted: allowed
    }
}
