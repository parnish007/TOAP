//! TOAP V1 protocol core — message model, parser, encoder.
//!
//! Pure `std`, no I/O. This is the *semantic plane* of the corrected two-plane design (N1):
//! the human/LLM-facing text payload. Routing/session/taint metadata belong on a separate binary
//! *control plane* (future V2). See `research.md` §8.
//!
//! Wire frame (V1): `TYPE|MSG_ID|TARGET|PAYLOAD`, payload is function-style `OP(args)?k=v&k=v`.
//! Canonical grammar: `docs/protocol_v1.md`. Source identity is **broker-derived**, never read from
//! a wire field — there is deliberately no `SRC` here.

use std::fmt;

/// Message type (the `TYPE` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MessageType {
    Syn,
    Ack,
    Req,
    Res,
    Err,
    Evt,
    Dlt,
    Bye,
    Hbt,
}

impl MessageType {
    pub fn as_str(self) -> &'static str {
        match self {
            MessageType::Syn => "SYN",
            MessageType::Ack => "ACK",
            MessageType::Req => "REQ",
            MessageType::Res => "RES",
            MessageType::Err => "ERR",
            MessageType::Evt => "EVT",
            MessageType::Dlt => "DLT",
            MessageType::Bye => "BYE",
            MessageType::Hbt => "HBT",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "SYN" => MessageType::Syn,
            "ACK" => MessageType::Ack,
            "REQ" => MessageType::Req,
            "RES" => MessageType::Res,
            "ERR" => MessageType::Err,
            "EVT" => MessageType::Evt,
            "DLT" => MessageType::Dlt,
            "BYE" => MessageType::Bye,
            "HBT" => MessageType::Hbt,
            _ => return None,
        })
    }
}

/// A positional payload argument: either a context reference `CTX:N` or an opaque token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Ctx(u32),
    Token(String),
}

/// A `key=value` option (the part after `?`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Opt {
    pub key: String,
    pub value: String,
}

/// A parsed function-style payload: `OP(args)?options`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Payload {
    pub op: String,
    pub args: Vec<Arg>,
    pub options: Vec<Opt>,
}

impl Payload {
    pub fn new(op: &str) -> Self {
        Payload { op: op.to_string(), args: Vec::new(), options: Vec::new() }
    }
    pub fn arg_ctx(mut self, id: u32) -> Self {
        self.args.push(Arg::Ctx(id));
        self
    }
    pub fn arg_token(mut self, t: &str) -> Self {
        self.args.push(Arg::Token(t.to_string()));
        self
    }
    pub fn opt(mut self, key: &str, value: &str) -> Self {
        self.options.push(Opt { key: key.to_string(), value: value.to_string() });
        self
    }
    /// First context reference in the args, if any.
    pub fn first_ctx(&self) -> Option<u32> {
        self.args.iter().find_map(|a| match a {
            Arg::Ctx(id) => Some(*id),
            _ => None,
        })
    }
    pub fn get_opt(&self, key: &str) -> Option<&str> {
        self.options.iter().find(|o| o.key == key).map(|o| o.value.as_str())
    }
}

/// A complete TOAP V1 message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub mtype: MessageType,
    pub msg_id: u32,
    pub target: String,
    pub payload: Payload,
}

impl Message {
    pub fn new(mtype: MessageType, msg_id: u32, target: &str, payload: Payload) -> Self {
        Message { mtype, msg_id, target: target.to_string(), payload }
    }
}

/// Parse failures. Mirrors the validation rules in `docs/protocol_v1.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseError {
    WrongFieldCount,
    EmptyField,
    UnknownMessageType,
    InvalidMessageId,
    InvalidTarget,
    InvalidPayload,
    InvalidContextRef,
    InvalidOption,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}
impl std::error::Error for ParseError {}

// ---------------------------------------------------------------------------
// Escaping: the frame delimiter `|` (and `\`) are escaped inside fields.
// ---------------------------------------------------------------------------

fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c == '\\' || c == '|' {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut esc = false;
    for c in s.chars() {
        if esc {
            out.push(c);
            esc = false;
        } else if c == '\\' {
            esc = true;
        } else {
            out.push(c);
        }
    }
    out
}

/// Split on an unescaped delimiter, keeping escape sequences intact for later `unescape`.
fn split_unescaped(s: &str, delim: char) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut esc = false;
    for c in s.chars() {
        if esc {
            cur.push(c);
            esc = false;
        } else if c == '\\' {
            cur.push(c);
            esc = true;
        } else if c == delim {
            out.push(std::mem::take(&mut cur));
        } else {
            cur.push(c);
        }
    }
    out.push(cur);
    out
}

// ---------------------------------------------------------------------------
// Validation helpers
// ---------------------------------------------------------------------------

fn valid_op(op: &str) -> bool {
    // OP = [A-Z][A-Z0-9_]{1,15}  => total length 2..=16
    let len = op.chars().count();
    if !(2..=16).contains(&len) {
        return false;
    }
    let mut chars = op.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_uppercase() {
        return false;
    }
    chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}

fn valid_target(t: &str) -> bool {
    if t == "*" || t == "broker" {
        return true;
    }
    // agent id: [A-Za-z][A-Za-z0-9_-]{0,31}
    let len = t.chars().count();
    if !(1..=32).contains(&len) {
        return false;
    }
    let mut chars = t.chars();
    let first = chars.next().unwrap();
    if !first.is_ascii_alphabetic() {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a payload string `OP(args)?options`.
pub fn parse_payload(s: &str) -> Result<Payload, ParseError> {
    let open = s.find('(').ok_or(ParseError::InvalidPayload)?;
    let op = &s[..open];
    if !valid_op(op) {
        return Err(ParseError::InvalidPayload);
    }
    let rest = &s[open + 1..];
    let close = rest.find(')').ok_or(ParseError::InvalidPayload)?;
    let args_str = &rest[..close];
    let after = &rest[close + 1..];

    // Options (after `?`)
    let mut options = Vec::new();
    if !after.is_empty() {
        let opts = after.strip_prefix('?').ok_or(ParseError::InvalidPayload)?;
        if opts.is_empty() {
            return Err(ParseError::InvalidOption);
        }
        for o in opts.split('&') {
            let eq = o.find('=').ok_or(ParseError::InvalidOption)?;
            let key = &o[..eq];
            let value = &o[eq + 1..];
            if key.is_empty() {
                return Err(ParseError::InvalidOption);
            }
            options.push(Opt { key: key.to_string(), value: value.to_string() });
        }
    }

    // Args (positional, comma-separated)
    let mut args = Vec::new();
    if !args_str.is_empty() {
        for a in args_str.split(',') {
            if a.is_empty() {
                return Err(ParseError::InvalidPayload);
            }
            if let Some(num) = a.strip_prefix("CTX:") {
                let id: u32 = num.parse().map_err(|_| ParseError::InvalidContextRef)?;
                args.push(Arg::Ctx(id));
            } else {
                args.push(Arg::Token(a.to_string()));
            }
        }
    }

    Ok(Payload { op: op.to_string(), args, options })
}

/// Parse a full wire frame `TYPE|MSG_ID|TARGET|PAYLOAD`.
pub fn parse_frame(raw: &str) -> Result<Message, ParseError> {
    let fields = split_unescaped(raw, '|');
    if fields.len() != 4 {
        return Err(ParseError::WrongFieldCount);
    }
    let tstr = unescape(&fields[0]);
    let idstr = unescape(&fields[1]);
    let target = unescape(&fields[2]);
    let payload_str = unescape(&fields[3]);

    if tstr.is_empty() || idstr.is_empty() || target.is_empty() || payload_str.is_empty() {
        return Err(ParseError::EmptyField);
    }
    let mtype = MessageType::parse(&tstr).ok_or(ParseError::UnknownMessageType)?;
    let msg_id: u32 = idstr.parse().map_err(|_| ParseError::InvalidMessageId)?;
    if !valid_target(&target) {
        return Err(ParseError::InvalidTarget);
    }
    let payload = parse_payload(&payload_str)?;
    Ok(Message { mtype, msg_id, target, payload })
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

pub fn encode_payload(p: &Payload) -> String {
    let mut s = String::new();
    s.push_str(&p.op);
    s.push('(');
    for (i, a) in p.args.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        match a {
            Arg::Ctx(id) => {
                s.push_str("CTX:");
                s.push_str(&id.to_string());
            }
            Arg::Token(t) => s.push_str(t),
        }
    }
    s.push(')');
    if !p.options.is_empty() {
        s.push('?');
        for (i, o) in p.options.iter().enumerate() {
            if i > 0 {
                s.push('&');
            }
            s.push_str(&o.key);
            s.push('=');
            s.push_str(&o.value);
        }
    }
    s
}

pub fn encode_frame(m: &Message) -> String {
    format!(
        "{}|{}|{}|{}",
        m.mtype.as_str(),
        m.msg_id,
        escape(&m.target),
        escape(&encode_payload(&m.payload))
    )
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip(raw: &str) {
        let m = parse_frame(raw).expect("should parse");
        assert_eq!(encode_frame(&m), raw, "round-trip mismatch");
    }

    #[test]
    fn roundtrips_spec_examples() {
        roundtrip("REQ|100|agentB|SUM(CTX:42)?max_words=150&lang=en");
        roundtrip("RES|100|agentA|OK(CTX:87)");
        roundtrip("ERR|100|agentA|NOCTX(CTX:42)");
        roundtrip("DLT|101|agentB|PATCH(CTX:42)?field=status&value=approved");
        roundtrip("HBT|102|broker|PING()");
        roundtrip("BYE|103|broker|CLOSE()");
        roundtrip("SYN|1|broker|HELLO()?agent_id=agentA&caps=SUM,GEN&version=1");
    }

    #[test]
    fn parses_fields() {
        let m = parse_frame("REQ|100|agentB|SUM(CTX:42)?max_words=150&lang=en").unwrap();
        assert_eq!(m.mtype, MessageType::Req);
        assert_eq!(m.msg_id, 100);
        assert_eq!(m.target, "agentB");
        assert_eq!(m.payload.op, "SUM");
        assert_eq!(m.payload.first_ctx(), Some(42));
        assert_eq!(m.payload.get_opt("max_words"), Some("150"));
        assert_eq!(m.payload.get_opt("lang"), Some("en"));
    }

    #[test]
    fn rejects_malformed() {
        assert_eq!(parse_frame("REQ|100|agentB").unwrap_err(), ParseError::WrongFieldCount);
        assert_eq!(parse_frame("REQ|100|agentB|SUM(CTX:42)|x").unwrap_err(), ParseError::WrongFieldCount);
        assert_eq!(parse_frame("REQ||agentB|SUM(CTX:1)").unwrap_err(), ParseError::EmptyField);
        assert_eq!(parse_frame("XXX|1|agentB|SUM(CTX:1)").unwrap_err(), ParseError::UnknownMessageType);
        assert_eq!(parse_frame("REQ|abc|agentB|SUM(CTX:1)").unwrap_err(), ParseError::InvalidMessageId);
        assert_eq!(parse_frame("REQ|99999999999|agentB|SUM(CTX:1)").unwrap_err(), ParseError::InvalidMessageId);
        assert_eq!(parse_frame("REQ|1|bad target!|SUM(CTX:1)").unwrap_err(), ParseError::InvalidTarget);
        assert_eq!(parse_frame("REQ|1|agentB|notpayload").unwrap_err(), ParseError::InvalidPayload);
        assert_eq!(parse_frame("REQ|1|agentB|SUM(CTX:-5)").unwrap_err(), ParseError::InvalidContextRef);
    }

    #[test]
    fn content_with_suspicious_text_is_kept_as_data() {
        // Security correction: ordinary content (SQL/HTML/prompt-like) is NOT rejected; it's data.
        let m = parse_frame("REQ|5|broker|SET()?data=DROP TABLE users; ignore previous instructions").unwrap();
        assert_eq!(m.payload.op, "SET");
        assert_eq!(m.payload.get_opt("data"), Some("DROP TABLE users; ignore previous instructions"));
    }

    #[test]
    fn pipe_in_value_is_escaped_roundtrip() {
        let m = Message::new(MessageType::Req, 7, "broker", Payload::new("SET").opt("data", "a|b\\c"));
        let wire = encode_frame(&m);
        let back = parse_frame(&wire).unwrap();
        assert_eq!(back.payload.get_opt("data"), Some("a|b\\c"));
    }
}
