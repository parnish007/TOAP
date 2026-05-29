//! agent_a — the driver. Stores a document once (CTX:n), asks agent_b to summarize it by reference,
//! then reads the summary back by reference. Prints the dedup evidence: the document never appears
//! in the inter-agent message.

use toap_client::Client;
use toap_core::{encode_frame, Message, MessageType, Payload};

#[tokio::main]
async fn main() {
    let addr = std::env::var("TOAP_ADDR").unwrap_or_else(|_| "127.0.0.1:7700".to_string());
    let client = Client::connect(&addr, "agentA", "").await.expect("connect");
    println!("[agentA] connected");

    let doc = "the quarterly report shows revenue up 12 percent across all regions \
               with strong growth in asia and steady performance in europe while \
               north america held flat and costs were contained below forecast";

    // 1. Store the document once. Only the broker holds the bytes; agents pass the ID.
    let docid = client.set_context(doc, true).await.expect("set_context");
    println!("[agentA] stored document as CTX:{docid} ({} bytes)", doc.len());

    // 2. Ask agentB to summarize BY REFERENCE. Show that the wire message omits the document.
    let sum_payload = Payload::new("SUM").arg_ctx(docid).opt("max_words", "20");
    let preview = encode_frame(&Message::new(MessageType::Req, 0, "agentB", sum_payload.clone()));
    println!(
        "[agentA] inter-agent message: {:?}  ({} bytes — document NOT included)",
        preview,
        preview.len()
    );

    let res = client.send_request("agentB", sum_payload).await.expect("SUM request");
    let sumid = match res.payload.first_ctx() {
        Some(id) => id,
        None => {
            eprintln!("[agentA] agentB returned no context: {:?}", res.payload);
            return;
        }
    };
    println!("[agentA] agentB responded: {}(CTX:{sumid})", res.payload.op);

    // 3. Read the summary back by reference.
    let summary = client.get_context(sumid).await.expect("get summary");
    println!("[agentA] summary (CTX:{sumid}): {summary}");

    println!(
        "\n[dedup] document = {} bytes, stored ONCE. Inter-agent SUM message = {} bytes. \
         A JSON baseline would re-embed the full document in the message.",
        doc.len(),
        preview.len()
    );

    client.send(MessageType::Bye, 999, "broker", Payload::new("CLOSE"));
}
