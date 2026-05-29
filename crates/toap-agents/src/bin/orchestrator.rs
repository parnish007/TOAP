//! orchestrator — demonstrates the fan-out pattern. Stores a document once, dispatches it BY
//! REFERENCE to a summarizer (agentB) and a classifier in parallel, then merges their results.
//! The document is transmitted to the store once; workers receive only `CTX:n`.

use std::time::Instant;

use toap_client::Client;
use toap_core::{encode_frame, Message, MessageType, Payload};

#[tokio::main]
async fn main() {
    let addr = std::env::var("TOAP_ADDR").unwrap_or_else(|_| "127.0.0.1:7700".to_string());
    let client = Client::connect(&addr, "orchestrator", "").await.expect("connect");
    println!("[orchestrator] connected");

    let doc = "Quarterly review: revenue rose 12 percent with strong growth in Asia, \
               margins improved, and churn fell to the lowest in three years; risks include \
               currency headwinds and a longer sales cycle for the largest deals";

    let docid = client.set_context(doc, true).await.expect("set_context");
    println!("[orchestrator] stored document once as CTX:{docid} ({} bytes)", doc.len());

    let started = Instant::now();

    // Fan out BY REFERENCE — neither dispatch carries the document.
    let sum_req = Payload::new("SUM").arg_ctx(docid);
    let cls_req = Payload::new("CLS").arg_ctx(docid);
    let sum_wire = encode_frame(&Message::new(MessageType::Req, 0, "agentB", sum_req.clone()));
    let cls_wire = encode_frame(&Message::new(MessageType::Req, 0, "classifier", cls_req.clone()));
    println!("[orchestrator] dispatch -> agentB:     {sum_wire:?} ({} bytes)", sum_wire.len());
    println!("[orchestrator] dispatch -> classifier: {cls_wire:?} ({} bytes)", cls_wire.len());

    let (sum_res, cls_res) = tokio::join!(
        client.send_request("agentB", sum_req),
        client.send_request("classifier", cls_req),
    );

    let sum_ctx = sum_res.expect("SUM").payload.first_ctx().expect("sum ctx");
    let cls_ctx = cls_res.expect("CLS").payload.first_ctx().expect("cls ctx");

    let summary = client.get_context(sum_ctx).await.expect("get summary");
    let label = client.get_context(cls_ctx).await.expect("get label");

    // Merge the two results into a final report context (avoid | & = in the merged text).
    let merged = format!("REPORT // summary: {summary} // classification: {label}");
    let report_id = client.set_context(&merged, false).await.expect("merge");

    println!("[orchestrator] merged into CTX:{report_id} in {} ms", started.elapsed().as_millis());
    println!("[orchestrator] final report: {}", client.get_context(report_id).await.unwrap());

    client.send(MessageType::Bye, 999, "broker", Payload::new("CLOSE"));
}
