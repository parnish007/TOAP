//! End-to-end integration test: broker + two in-process agents, full REQ/RES flow over real TCP.
//! Proves: handshake, broker-derived routing, context store, response correlation, and that the
//! inter-agent message carries only the reference (not the document) — TOAP's core dedup claim.

use std::time::Duration;

use tokio::net::TcpListener;

use toap_client::Client;
use toap_core::{encode_frame, Message, MessageType, Payload};

#[tokio::test]
async fn full_flow_dedup() {
    // Bind an ephemeral port and run the broker in the background.
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let _ = toap_broker::serve_with_listener(listener).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    // agentB: rule-based summarizer loop.
    let b = Client::connect(&addr, "agentB", "SUM").await.unwrap();
    {
        let b = b.clone();
        tokio::spawn(async move {
            while let Some(req) = b.recv().await {
                if req.payload.op == "SUM" {
                    if let Some(cid) = req.payload.first_ctx() {
                        if let Ok(content) = b.get_context(cid).await {
                            let head =
                                content.split_whitespace().take(4).collect::<Vec<_>>().join(" ");
                            let summary = format!("SUMMARY: {head}");
                            if let Ok(sid) = b.set_context(&summary, false).await {
                                b.send(
                                    MessageType::Res,
                                    req.msg_id,
                                    "broker",
                                    Payload::new("OK").arg_ctx(sid),
                                );
                            }
                        }
                    }
                }
            }
        });
    }
    tokio::time::sleep(Duration::from_millis(50)).await;

    // agentA: store doc, request summary by reference, read it back.
    let a = Client::connect(&addr, "agentA", "").await.unwrap();
    let doc = "alpha beta gamma delta epsilon zeta eta theta";
    let docid = a.set_context(doc, true).await.unwrap();
    assert_eq!(docid, 1, "first context id should be 1");

    let sum_payload = Payload::new("SUM").arg_ctx(docid);
    let wire = encode_frame(&Message::new(MessageType::Req, 0, "agentB", sum_payload.clone()));
    assert!(
        !wire.contains("alpha"),
        "inter-agent message must NOT contain the document text; got: {wire}"
    );

    let res = a.send_request("agentB", sum_payload).await.unwrap();
    assert_eq!(res.payload.op, "OK", "expected OK response");
    let sid = res.payload.first_ctx().expect("response carries a context id");

    let summary = a.get_context(sid).await.unwrap();
    assert!(summary.starts_with("SUMMARY"), "got: {summary}");
    assert!(summary.contains("alpha"), "summary should derive from the doc; got: {summary}");
}

#[tokio::test]
async fn unknown_target_errors() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move {
        let _ = toap_broker::serve_with_listener(listener).await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    let a = Client::connect(&addr, "agentA", "").await.unwrap();
    let res = a.send_request("ghost", Payload::new("SUM").arg_ctx(1)).await.unwrap();
    assert_eq!(res.mtype, MessageType::Err);
    assert_eq!(res.payload.op, "NOAGENT");
}
