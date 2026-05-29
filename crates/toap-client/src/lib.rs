//! TOAP client SDK.
//!
//! Connects to the broker, performs SYN/ACK, then runs a background reader that:
//!   * correlates `RES`/`ERR`/`ACK` to outstanding requests by `msg_id`,
//!   * routes inbound `REQ` to the application (handler loop),
//!   * routes `EVT` to a separate events channel (subscriptions).
//! Identity is established at SYN and owned by the broker thereafter.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::net::TcpStream;
use tokio::sync::{mpsc, oneshot, Mutex as TokioMutex};

use toap_core::{encode_frame, parse_frame, Message, MessageType, Payload};
use toap_wire::{read_frame, write_frame};

type Pending = Arc<StdMutex<HashMap<u32, oneshot::Sender<Message>>>>;

pub struct Client {
    pub agent_id: String,
    out_tx: mpsc::UnboundedSender<String>,
    pending: Pending,
    inbound_rx: TokioMutex<mpsc::UnboundedReceiver<Message>>,
    events_rx: TokioMutex<mpsc::UnboundedReceiver<Message>>,
    next_id: AtomicU32,
}

impl Client {
    pub async fn connect(addr: &str, agent_id: &str, caps: &str) -> std::io::Result<Arc<Client>> {
        let stream = TcpStream::connect(addr).await?;
        let (mut rd, mut wr) = stream.into_split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<String>();
        tokio::spawn(async move {
            while let Some(frame) = out_rx.recv().await {
                if write_frame(&mut wr, frame.as_bytes()).await.is_err() {
                    break;
                }
            }
        });

        let syn = Message::new(
            MessageType::Syn,
            1,
            "broker",
            Payload::new("HELLO").opt("agent_id", agent_id).opt("caps", caps).opt("version", "1"),
        );
        let _ = out_tx.send(encode_frame(&syn));

        match read_frame(&mut rd).await? {
            Some(bytes) => {
                let s = String::from_utf8_lossy(&bytes);
                match parse_frame(&s) {
                    Ok(m) if m.mtype == MessageType::Ack => {}
                    _ => {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            format!("expected ACK, got: {s}"),
                        ))
                    }
                }
            }
            None => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "connection closed during handshake",
                ))
            }
        }

        let pending: Pending = Arc::new(StdMutex::new(HashMap::new()));
        let (in_tx, in_rx) = mpsc::unbounded_channel::<Message>();
        let (evt_tx, evt_rx) = mpsc::unbounded_channel::<Message>();

        {
            let pending = pending.clone();
            tokio::spawn(async move {
                loop {
                    match read_frame(&mut rd).await {
                        Ok(Some(bytes)) => {
                            let s = match String::from_utf8(bytes) {
                                Ok(s) => s,
                                Err(_) => continue,
                            };
                            let msg = match parse_frame(&s) {
                                Ok(m) => m,
                                Err(_) => continue,
                            };
                            match msg.mtype {
                                MessageType::Res | MessageType::Err | MessageType::Ack => {
                                    if let Some(tx) = pending.lock().unwrap().remove(&msg.msg_id) {
                                        let _ = tx.send(msg);
                                    }
                                }
                                MessageType::Evt => {
                                    let _ = evt_tx.send(msg);
                                }
                                _ => {
                                    let _ = in_tx.send(msg);
                                }
                            }
                        }
                        _ => break,
                    }
                }
            });
        }

        Ok(Arc::new(Client {
            agent_id: agent_id.to_string(),
            out_tx,
            pending,
            inbound_rx: TokioMutex::new(in_rx),
            events_rx: TokioMutex::new(evt_rx),
            next_id: AtomicU32::new(100),
        }))
    }

    fn next_id(&self) -> u32 {
        self.next_id.fetch_add(1, Ordering::SeqCst)
    }

    /// Send a message and await its correlated reply (RES/ERR/ACK), 5s timeout.
    async fn round_trip(
        &self,
        mtype: MessageType,
        target: &str,
        payload: Payload,
    ) -> Result<Message, String> {
        let id = self.next_id();
        let (tx, rx) = oneshot::channel();
        self.pending.lock().unwrap().insert(id, tx);
        let msg = Message::new(mtype, id, target, payload);
        self.out_tx.send(encode_frame(&msg)).map_err(|e| e.to_string())?;
        match tokio::time::timeout(Duration::from_secs(5), rx).await {
            Ok(Ok(resp)) => Ok(resp),
            Ok(Err(_)) => Err("response channel closed".to_string()),
            Err(_) => {
                self.pending.lock().unwrap().remove(&id);
                Err("request timed out".to_string())
            }
        }
    }

    pub async fn send_request(&self, target: &str, payload: Payload) -> Result<Message, String> {
        self.round_trip(MessageType::Req, target, payload).await
    }

    /// Fire-and-forget (responses, events).
    pub fn send(&self, mtype: MessageType, msg_id: u32, target: &str, payload: Payload) {
        let _ = self.out_tx.send(encode_frame(&Message::new(mtype, msg_id, target, payload)));
    }

    /// Next inbound request (handler loop). None when the connection closes.
    pub async fn recv(&self) -> Option<Message> {
        self.inbound_rx.lock().await.recv().await
    }

    /// Next inbound event (subscriptions). None when the connection closes.
    pub async fn recv_event(&self) -> Option<Message> {
        self.events_rx.lock().await.recv().await
    }

    // ---- context store convenience ----------------------------------------

    pub async fn set_context(&self, data: &str, tainted: bool) -> Result<u32, String> {
        self.set_context_full(data, tainted, "*:r", 0).await
    }

    pub async fn set_context_full(
        &self,
        data: &str,
        tainted: bool,
        acl: &str,
        ttl: u32,
    ) -> Result<u32, String> {
        let payload = Payload::new("SET")
            .opt("data", data)
            .opt("taint", if tainted { "true" } else { "false" })
            .opt("acl", acl)
            .opt("ttl", &ttl.to_string());
        let res = self.send_request("broker", payload).await?;
        if res.mtype == MessageType::Err {
            return Err(format!("SET failed: {}", res.payload.op));
        }
        res.payload.first_ctx().ok_or_else(|| "SET returned no context id".to_string())
    }

    pub async fn get_context(&self, id: u32) -> Result<String, String> {
        let res = self.send_request("broker", Payload::new("GET").arg_ctx(id)).await?;
        if res.mtype == MessageType::Err {
            return Err(format!("GET failed: {}", res.payload.op));
        }
        res.payload.get_opt("data").map(|s| s.to_string()).ok_or_else(|| "no data".to_string())
    }

    /// Field-level delta. Returns the new context version.
    pub async fn patch(&self, ctx: u32, field: &str, value: &str) -> Result<u32, String> {
        let payload = Payload::new("PATCH").arg_ctx(ctx).opt("field", field).opt("value", value);
        let res = self.round_trip(MessageType::Dlt, "broker", payload).await?;
        if res.mtype == MessageType::Err {
            return Err(format!("PATCH failed: {}", res.payload.op));
        }
        Ok(res.payload.get_opt("version").and_then(|s| s.parse().ok()).unwrap_or(0))
    }

    /// Subscribe to a context's change events (delivered via `recv_event`).
    pub async fn subscribe(&self, ctx: u32) -> Result<(), String> {
        let res = self.send_request("broker", Payload::new("SUB").arg_ctx(ctx)).await?;
        if res.mtype == MessageType::Err {
            return Err(format!("SUB failed: {}", res.payload.op));
        }
        Ok(())
    }
}
