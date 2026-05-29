//! TOAP broker.
//!
//! Responsibilities (v1.0):
//! - Accept TCP connections framed with a 4-byte length prefix.
//! - SYN/ACK bind a session to the connection. **Source identity is broker-derived from the
//!   authenticated connection — never read from a wire `SRC` field.**
//! - Own the shared context store (SET/GET/DEL) with **per-context ACLs** and **TTL**.
//! - Field-level **deltas** (`DLT`/`PATCH`) and **subscriptions** (`SUB` -> `EVT` fan-out).
//! - Enforce **rate limiting**, **replay rejection** (duplicate request ids per session), and a
//!   **taint policy** (restricted ops cannot run against user-originated/tainted context).
//! - Route `REQ` to a target agent and correlate the returning `RES`/`ERR` by `msg_id`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};

use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, Mutex as TokioMutex};

use toap_context::{Access, Acl, ContextStore};
use toap_core::{encode_frame, parse_frame, Arg, Message, MessageType, Payload};
use toap_security::{RateConfig, RateLimiter, ReplayGuard, TaintPolicy};
use toap_wire::{read_frame, write_frame};

type Tx = mpsc::UnboundedSender<String>;

/// Shared broker state.
pub struct State {
    registry: TokioMutex<HashMap<String, Tx>>,
    store: TokioMutex<ContextStore>,
    /// msg_id -> requester agent_id (response correlation).
    pending: StdMutex<HashMap<u32, String>>,
    /// ctx_id -> subscriber agent_ids.
    subs: TokioMutex<HashMap<u32, Vec<String>>>,
    rate: TokioMutex<RateLimiter>,
    replay: TokioMutex<ReplayGuard>,
    taint: TaintPolicy,
    session_counter: AtomicU64,
}

impl State {
    fn new() -> Self {
        State {
            registry: TokioMutex::new(HashMap::new()),
            store: TokioMutex::new(ContextStore::new()),
            pending: StdMutex::new(HashMap::new()),
            subs: TokioMutex::new(HashMap::new()),
            rate: TokioMutex::new(RateLimiter::new(RateConfig::default())),
            replay: TokioMutex::new(ReplayGuard::new()),
            taint: TaintPolicy::default(),
            session_counter: AtomicU64::new(1),
        }
    }
}

pub async fn serve(addr: &str) -> std::io::Result<()> {
    let listener = TcpListener::bind(addr).await?;
    println!("[broker] listening on {}", listener.local_addr()?);
    serve_with_listener(listener).await
}

pub async fn serve_with_listener(listener: TcpListener) -> std::io::Result<()> {
    let state = Arc::new(State::new());
    loop {
        let (sock, _peer) = listener.accept().await?;
        let st = state.clone();
        tokio::spawn(async move {
            if let Err(e) = handle_conn(sock, st).await {
                eprintln!("[broker] connection ended: {e}");
            }
        });
    }
}

fn err(out: &Tx, mid: u32, to: &str, op: &str, reason_k: &str, reason_v: &str) {
    let _ = out.send(encode_frame(&Message::new(
        MessageType::Err,
        mid,
        to,
        Payload::new(op).opt(reason_k, reason_v),
    )));
}

async fn handle_conn(sock: TcpStream, state: Arc<State>) -> std::io::Result<()> {
    let (mut rd, mut wr) = sock.into_split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();

    tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            if write_frame(&mut wr, frame.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    // --- SYN handshake ---
    let first = match read_frame(&mut rd).await? {
        Some(b) => b,
        None => return Ok(()),
    };
    let syn = match parse_frame(&String::from_utf8_lossy(&first)) {
        Ok(m) => m,
        Err(e) => {
            err(&out_tx, 0, "broker", "BADMSG", "reason", &e.to_string());
            return Ok(());
        }
    };
    if syn.mtype != MessageType::Syn {
        err(&out_tx, syn.msg_id, "broker", "BADMSG", "reason", "expected_SYN");
        return Ok(());
    }
    let agent_id = match syn.payload.get_opt("agent_id") {
        Some(a) if !a.is_empty() => a.to_string(),
        _ => {
            err(&out_tx, syn.msg_id, "broker", "BADMSG", "reason", "missing_agent_id");
            return Ok(());
        }
    };
    let session_id = format!("sess_{}", state.session_counter.fetch_add(1, Ordering::SeqCst));
    state.registry.lock().await.insert(agent_id.clone(), out_tx.clone());
    println!("[broker] {agent_id} connected (session {session_id})");

    let _ = out_tx.send(encode_frame(&Message::new(
        MessageType::Ack,
        syn.msg_id,
        &agent_id,
        Payload::new("SESSION")
            .opt("session_id", &session_id)
            .opt("agent_id", &agent_id)
            .opt("trust", "internal")
            .opt("version", "1"),
    )));

    // --- message loop ---
    loop {
        let bytes = match read_frame(&mut rd).await? {
            Some(b) => b,
            None => break,
        };
        let msg = match parse_frame(&String::from_utf8_lossy(&bytes)) {
            Ok(m) => m,
            Err(e) => {
                err(&out_tx, 0, "broker", "BADMSG", "reason", &e.to_string());
                continue;
            }
        };

        // Rate limit every inbound message.
        if !state.rate.lock().await.allow(&agent_id) {
            err(&out_tx, msg.msg_id, &agent_id, "RATE", "agent", &agent_id);
            continue;
        }
        // Replay rejection for state-affecting client messages.
        if matches!(msg.mtype, MessageType::Req | MessageType::Dlt)
            && !state.replay.lock().await.accept(&session_id, msg.msg_id)
        {
            err(&out_tx, msg.msg_id, &agent_id, "REPLAY", "msg_id", &msg.msg_id.to_string());
            continue;
        }

        route(&msg, &agent_id, &state, &out_tx).await;
        if msg.mtype == MessageType::Bye {
            break;
        }
    }

    state.registry.lock().await.remove(&agent_id);
    state.replay.lock().await.forget(&session_id);
    // Drop this agent's subscriptions.
    let mut subs = state.subs.lock().await;
    for v in subs.values_mut() {
        v.retain(|a| a != &agent_id);
    }
    drop(subs);
    println!("[broker] {agent_id} disconnected");
    Ok(())
}

async fn route(msg: &Message, src: &str, state: &Arc<State>, out_tx: &Tx) {
    match msg.mtype {
        MessageType::Dlt => broker_patch(msg, src, state, out_tx).await,
        MessageType::Req if msg.target == "broker" => broker_op(msg, src, state, out_tx).await,
        MessageType::Req => {
            // Taint policy: a restricted op cannot run against tainted context.
            {
                let store = state.store.lock().await;
                for a in &msg.payload.args {
                    if let Arg::Ctx(id) = a {
                        if let Some(e) = store.get(*id) {
                            if !state.taint.allows(&msg.payload.op, e.tainted, false) {
                                drop(store);
                                err(out_tx, msg.msg_id, src, "NOPERM", "reason", "tainted_context");
                                return;
                            }
                        }
                    }
                }
            }
            state.pending.lock().unwrap().insert(msg.msg_id, src.to_string());
            let reg = state.registry.lock().await;
            if let Some(tx) = reg.get(&msg.target) {
                let _ = tx.send(encode_frame(msg));
            } else {
                drop(reg);
                state.pending.lock().unwrap().remove(&msg.msg_id);
                err(out_tx, msg.msg_id, src, "NOAGENT", "target", &msg.target);
            }
        }
        MessageType::Res | MessageType::Err => {
            let requester = state.pending.lock().unwrap().remove(&msg.msg_id);
            if let Some(requester) = requester {
                let reg = state.registry.lock().await;
                if let Some(tx) = reg.get(&requester) {
                    let mut delivered = msg.clone();
                    delivered.target = requester;
                    let _ = tx.send(encode_frame(&delivered));
                }
            }
        }
        MessageType::Hbt => {
            let _ = out_tx.send(encode_frame(&Message::new(
                MessageType::Ack,
                msg.msg_id,
                src,
                Payload::new("PONG"),
            )));
        }
        MessageType::Bye => {
            let _ = out_tx.send(encode_frame(&Message::new(
                MessageType::Ack,
                msg.msg_id,
                src,
                Payload::new("CLOSED"),
            )));
        }
        _ => {}
    }
}

async fn broker_op(msg: &Message, src: &str, state: &Arc<State>, out_tx: &Tx) {
    match msg.payload.op.as_str() {
        "SET" => {
            let data = msg.payload.get_opt("data").unwrap_or("").as_bytes().to_vec();
            let tainted = msg.payload.get_opt("taint") != Some("false");
            let acl = Acl::parse(msg.payload.get_opt("acl").unwrap_or("*:r"));
            let ttl: u32 = msg.payload.get_opt("ttl").and_then(|s| s.parse().ok()).unwrap_or(0);
            let id = state.store.lock().await.set(src, data, tainted, acl, ttl);
            let _ = out_tx.send(encode_frame(&Message::new(
                MessageType::Res,
                msg.msg_id,
                src,
                Payload::new("OK").arg_ctx(id),
            )));
        }
        "GET" => {
            let Some(id) = first_ctx_arg(msg) else {
                return err(out_tx, msg.msg_id, src, "BADMSG", "reason", "GET_needs_ctx");
            };
            let store = state.store.lock().await;
            match store.get_for(id, src) {
                (Access::Ok, Some(e)) => {
                    let data = String::from_utf8_lossy(&e.data).to_string();
                    let _ = out_tx.send(encode_frame(&Message::new(
                        MessageType::Res,
                        msg.msg_id,
                        src,
                        Payload::new("OK").arg_ctx(id).opt("data", &data),
                    )));
                }
                (Access::NoPerm, _) => err(out_tx, msg.msg_id, src, "NOPERM", "ctx", &id.to_string()),
                _ => err(out_tx, msg.msg_id, src, "NOCTX", "ctx", &id.to_string()),
            }
        }
        "DEL" => {
            let Some(id) = first_ctx_arg(msg) else {
                return err(out_tx, msg.msg_id, src, "BADMSG", "reason", "DEL_needs_ctx");
            };
            // Taint: DEL is a restricted op.
            {
                let store = state.store.lock().await;
                if let Some(e) = store.get(id) {
                    if !state.taint.allows("DEL", e.tainted, false) {
                        drop(store);
                        return err(out_tx, msg.msg_id, src, "NOPERM", "reason", "tainted_context");
                    }
                }
            }
            let acc = state.store.lock().await.delete_for(id, src);
            match acc {
                Access::Ok => {
                    let _ = out_tx.send(encode_frame(&Message::new(
                        MessageType::Res, msg.msg_id, src, Payload::new("OK").arg_ctx(id),
                    )));
                }
                Access::NoPerm => err(out_tx, msg.msg_id, src, "NOPERM", "ctx", &id.to_string()),
                Access::NoCtx => err(out_tx, msg.msg_id, src, "NOCTX", "ctx", &id.to_string()),
            }
        }
        "SUB" => {
            let Some(id) = first_ctx_arg(msg) else {
                return err(out_tx, msg.msg_id, src, "BADMSG", "reason", "SUB_needs_ctx");
            };
            state.subs.lock().await.entry(id).or_default().push(src.to_string());
            let _ = out_tx.send(encode_frame(&Message::new(
                MessageType::Res, msg.msg_id, src, Payload::new("OK").arg_ctx(id),
            )));
        }
        _ => err(out_tx, msg.msg_id, src, "UNSUP", "op", &msg.payload.op),
    }
}

/// `DLT|id|broker|PATCH(CTX:n)?field=..&value=..` — apply a field delta and notify subscribers.
async fn broker_patch(msg: &Message, src: &str, state: &Arc<State>, out_tx: &Tx) {
    let Some(id) = first_ctx_arg(msg) else {
        return err(out_tx, msg.msg_id, src, "BADMSG", "reason", "PATCH_needs_ctx");
    };
    let field = msg.payload.get_opt("field").unwrap_or("").to_string();
    let value = msg.payload.get_opt("value").unwrap_or("").to_string();
    if field.is_empty() {
        return err(out_tx, msg.msg_id, src, "BADMSG", "reason", "PATCH_needs_field");
    }

    let (acc, version) = state.store.lock().await.patch_for(id, src, &field, &value);
    match acc {
        Access::Ok => {
            let _ = out_tx.send(encode_frame(&Message::new(
                MessageType::Ack,
                msg.msg_id,
                src,
                Payload::new("OK").arg_ctx(id).opt("version", &version.to_string()),
            )));
            // Fan out an EVT to subscribers (except the patcher).
            let subs = state.subs.lock().await;
            if let Some(list) = subs.get(&id) {
                let targets: Vec<String> = list.iter().filter(|a| *a != src).cloned().collect();
                drop(subs);
                let reg = state.registry.lock().await;
                for t in targets {
                    if let Some(tx) = reg.get(&t) {
                        let _ = tx.send(encode_frame(&Message::new(
                            MessageType::Evt,
                            0,
                            &t,
                            Payload::new("CHANGED")
                                .arg_ctx(id)
                                .opt("field", &field)
                                .opt("value", &value)
                                .opt("version", &version.to_string()),
                        )));
                    }
                }
            }
        }
        Access::NoPerm => err(out_tx, msg.msg_id, src, "NOPERM", "ctx", &id.to_string()),
        Access::NoCtx => err(out_tx, msg.msg_id, src, "NOCTX", "ctx", &id.to_string()),
    }
}

fn first_ctx_arg(msg: &Message) -> Option<u32> {
    msg.payload.args.iter().find_map(|a| match a {
        Arg::Ctx(id) => Some(*id),
        _ => None,
    })
}
