//! toap-mcp — the adoption wedge: exposes TOAP's shared context store as an MCP server.
//!
//! Minimal MCP over stdio (newline-delimited JSON-RPC 2.0). An MCP host (e.g. Claude Desktop/Code)
//! can `toap_set` content once and `toap_get` it by id — giving any host a shared, taint-aware,
//! deduplicated context store. This deliberately exposes only the context-store slice of TOAP;
//! MCP is agent↔tool (vertical), whereas TOAP's wire protocol is agent↔agent (horizontal).
//!
//! Supported methods: initialize, tools/list, tools/call (toap_set, toap_get), ping.
//! Try it:  echo '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | toap-mcp

use std::io::{self, BufRead, Write};

use serde_json::{json, Value};
use toap_context::{Access, Acl, ContextStore};

const SERVER_NAME: &str = "toap-mcp";
const SERVER_VERSION: &str = env!("CARGO_PKG_VERSION");
const PROTOCOL_VERSION: &str = "2024-11-05";

fn main() {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = stdout.lock();
    let mut store = ContextStore::new();

    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        // Tolerate a leading UTF-8 BOM and surrounding whitespace (some hosts/pipes add one).
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() {
            continue;
        }
        let req: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(_) => {
                write_msg(&mut out, &error_response(Value::Null, -32700, "parse error"));
                continue;
            }
        };

        let id = req.get("id").cloned();
        let method = req.get("method").and_then(Value::as_str).unwrap_or("");

        // Notifications (no id) get no response.
        let is_notification = id.is_none();

        let response = match method {
            "initialize" => Some(ok(id.clone(), json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": SERVER_NAME, "version": SERVER_VERSION }
            }))),
            "ping" => Some(ok(id.clone(), json!({}))),
            "tools/list" => Some(ok(id.clone(), json!({ "tools": tool_specs() }))),
            "tools/call" => Some(handle_tool_call(id.clone(), &req, &mut store)),
            "notifications/initialized" => None,
            _ if is_notification => None,
            _ => Some(error_response(id.clone().unwrap_or(Value::Null), -32601, "method not found")),
        };

        if let Some(resp) = response {
            write_msg(&mut out, &resp);
        }
    }
}

fn tool_specs() -> Value {
    json!([
        {
            "name": "toap_set",
            "description": "Store content in the shared TOAP context store; returns a numeric context id (CTX:N). Store a document once, then reference it by id.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "data": { "type": "string", "description": "the content to store" },
                    "tainted": { "type": "boolean", "description": "true if user/external-originated (default true)" }
                },
                "required": ["data"]
            }
        },
        {
            "name": "toap_get",
            "description": "Fetch content from the shared TOAP context store by numeric id.",
            "inputSchema": {
                "type": "object",
                "properties": { "id": { "type": "integer", "description": "the context id" } },
                "required": ["id"]
            }
        }
    ])
}

fn handle_tool_call(id: Option<Value>, req: &Value, store: &mut ContextStore) -> Value {
    let id = id.unwrap_or(Value::Null);
    let params = req.get("params").cloned().unwrap_or(Value::Null);
    let name = params.get("name").and_then(Value::as_str).unwrap_or("");
    let args = params.get("arguments").cloned().unwrap_or(json!({}));

    match name {
        "toap_set" => {
            let data = args.get("data").and_then(Value::as_str).unwrap_or("");
            let tainted = args.get("tainted").and_then(Value::as_bool).unwrap_or(true);
            let cid = store.set("mcp", data.as_bytes().to_vec(), tainted, Acl::parse("*:r"), 0);
            tool_text(id, &format!("stored as CTX:{cid}"), false)
        }
        "toap_get" => {
            let cid = args.get("id").and_then(Value::as_u64).unwrap_or(0) as u32;
            match store.get_for(cid, "mcp") {
                (Access::Ok, Some(e)) => {
                    tool_text(id, &String::from_utf8_lossy(&e.data), false)
                }
                (Access::NoPerm, _) => tool_text(id, &format!("CTX:{cid} access denied"), true),
                _ => tool_text(id, &format!("CTX:{cid} not found"), true),
            }
        }
        other => error_response(id, -32602, &format!("unknown tool: {other}")),
    }
}

fn tool_text(id: Value, text: &str, is_error: bool) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": { "content": [{ "type": "text", "text": text }], "isError": is_error }
    })
}

fn ok(id: Option<Value>, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id.unwrap_or(Value::Null), "result": result })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn write_msg<W: Write>(out: &mut W, msg: &Value) {
    let _ = writeln!(out, "{}", serde_json::to_string(msg).unwrap());
    let _ = out.flush();
}
