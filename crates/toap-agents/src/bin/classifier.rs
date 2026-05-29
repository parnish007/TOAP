//! classifier — a rule-based sentiment/topic classifier (no LLM). Handles `CLS(CTX:n)`:
//! fetches the context, classifies deterministically, stores the label, responds with its ID.

use toap_client::Client;
use toap_core::{MessageType, Payload};

fn classify(text: &str) -> String {
    let t = text.to_lowercase();
    let pos = ["growth", "improved", "strong", "up", "rose", "lowest churn", "renewed"]
        .iter()
        .filter(|w| t.contains(*w))
        .count();
    let neg = ["risk", "flat", "slipped", "headwinds", "longer sales cycle", "fell"]
        .iter()
        .filter(|w| t.contains(*w))
        .count();
    let label = if pos > neg { "positive" } else if neg > pos { "negative" } else { "neutral" };
    format!("sentiment {} (pos {} neg {})", label, pos, neg)
}

#[tokio::main]
async fn main() {
    let addr = std::env::var("TOAP_ADDR").unwrap_or_else(|_| "127.0.0.1:7700".to_string());
    let client = Client::connect(&addr, "classifier", "CLS").await.expect("connect");
    println!("[classifier] connected, waiting for CLS requests...");

    while let Some(req) = client.recv().await {
        if req.payload.op == "CLS" {
            match req.payload.first_ctx() {
                Some(cid) => match client.get_context(cid).await {
                    Ok(content) => {
                        let label = classify(&content);
                        match client.set_context(&label, false).await {
                            Ok(rid) => {
                                println!("[classifier] CLS(CTX:{cid}) -> {label} (CTX:{rid})");
                                client.send(MessageType::Res, req.msg_id, "broker",
                                    Payload::new("OK").arg_ctx(rid));
                            }
                            Err(e) => client.send(MessageType::Err, req.msg_id, "broker",
                                Payload::new("BADMSG").opt("reason", &e)),
                        }
                    }
                    Err(_) => client.send(MessageType::Err, req.msg_id, "broker",
                        Payload::new("NOCTX").arg_ctx(cid)),
                },
                None => client.send(MessageType::Err, req.msg_id, "broker",
                    Payload::new("BADMSG").opt("reason", "CLS_needs_ctx")),
            }
        } else {
            client.send(MessageType::Err, req.msg_id, "broker",
                Payload::new("UNSUP").opt("op", &req.payload.op));
        }
    }
}
