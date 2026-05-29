//! agent_b — a rule-based summarizer (no LLM). Handles `SUM(CTX:n)`:
//! fetches the context, summarizes deterministically, stores the result, and responds with the
//! new context ID — so the summary travels as a reference, not as inline text.

use toap_client::Client;
use toap_core::{MessageType, Payload};

fn summarize(text: &str) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let head = words.iter().take(8).cloned().collect::<Vec<_>>().join(" ");
    format!("SUMMARY[{} words in source]: {} ...", words.len(), head)
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("TOAP_ADDR").unwrap_or_else(|_| "127.0.0.1:7700".to_string());
    let client = Client::connect(&addr, "agentB", "SUM").await.expect("connect");
    println!("[agentB] connected, waiting for SUM requests...");

    while let Some(req) = client.recv().await {
        if req.payload.op == "SUM" {
            match req.payload.first_ctx() {
                Some(cid) => match client.get_context(cid).await {
                    Ok(content) => {
                        let summary = summarize(&content);
                        match client.set_context(&summary, false).await {
                            Ok(sid) => {
                                println!("[agentB] SUM(CTX:{cid}) -> stored summary as CTX:{sid}");
                                client.send(
                                    MessageType::Res,
                                    req.msg_id,
                                    "broker",
                                    Payload::new("OK").arg_ctx(sid),
                                );
                            }
                            Err(e) => client.send(
                                MessageType::Err,
                                req.msg_id,
                                "broker",
                                Payload::new("BADMSG").opt("reason", &e),
                            ),
                        }
                    }
                    Err(_) => client.send(
                        MessageType::Err,
                        req.msg_id,
                        "broker",
                        Payload::new("NOCTX").arg_ctx(cid),
                    ),
                },
                None => client.send(
                    MessageType::Err,
                    req.msg_id,
                    "broker",
                    Payload::new("BADMSG").opt("reason", "SUM_needs_ctx"),
                ),
            }
        } else {
            client.send(
                MessageType::Err,
                req.msg_id,
                "broker",
                Payload::new("UNSUP").opt("op", &req.payload.op),
            );
        }
    }
    println!("[agentB] connection closed");
}
