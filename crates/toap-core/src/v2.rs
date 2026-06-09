//! V2 binary control plane (the two-plane design, N1).
//!
//! A V2 frame separates the **control plane** (version, type, msg_id, flags, target) — encoded as a
//! compact binary header read only by the broker — from the **semantic plane** (the `OP(args)?opts`
//! payload) which stays text/bytes read by the LLM. This realizes the architectural separation: the
//! broker never tokenizes the payload, and the LLM never sees the control header.
//!
//! Frame layout:
//! ```text
//! byte 0      : version (0x02)
//! byte 1      : message type (enum, 1 byte)
//! byte 2..6   : msg_id (u32, big-endian)
//! byte 6      : flags (bit0 = tainted/control hint; rest reserved)
//! byte 7      : target length (u8)
//! byte 8..    : target (UTF-8)
//! next 2      : payload length (u16, big-endian)
//! next N      : payload bytes (semantic plane: the function-style text)
//! ```
//! V2 is wire-compatible with V1 semantics: it round-trips the same `Message`.

use crate::{encode_payload, parse_payload, Message, MessageType, ParseError};

pub const V2_VERSION: u8 = 0x02;
pub const FLAG_TAINTED: u8 = 0b0000_0001;

fn type_code(t: MessageType) -> u8 {
    match t {
        MessageType::Syn => 0x01,
        MessageType::Ack => 0x02,
        MessageType::Req => 0x03,
        MessageType::Res => 0x04,
        MessageType::Err => 0x05,
        MessageType::Evt => 0x06,
        MessageType::Dlt => 0x07,
        MessageType::Bye => 0x08,
        MessageType::Hbt => 0x09,
    }
}

fn type_from(b: u8) -> Option<MessageType> {
    Some(match b {
        0x01 => MessageType::Syn,
        0x02 => MessageType::Ack,
        0x03 => MessageType::Req,
        0x04 => MessageType::Res,
        0x05 => MessageType::Err,
        0x06 => MessageType::Evt,
        0x07 => MessageType::Dlt,
        0x08 => MessageType::Bye,
        0x09 => MessageType::Hbt,
        _ => return None,
    })
}

/// Encode a message as a V2 binary frame. `flags` carries control-plane metadata (e.g. taint).
pub fn encode_v2_flags(m: &Message, flags: u8) -> Vec<u8> {
    let payload = encode_payload(&m.payload);
    let payload_bytes = payload.as_bytes();
    let target_bytes = m.target.as_bytes();
    let mut out = Vec::with_capacity(10 + target_bytes.len() + payload_bytes.len());
    out.push(V2_VERSION);
    out.push(type_code(m.mtype));
    out.extend_from_slice(&m.msg_id.to_be_bytes());
    out.push(flags);
    out.push(target_bytes.len().min(255) as u8);
    out.extend_from_slice(&target_bytes[..target_bytes.len().min(255)]);
    let plen = payload_bytes.len().min(u16::MAX as usize) as u16;
    out.extend_from_slice(&plen.to_be_bytes());
    out.extend_from_slice(&payload_bytes[..plen as usize]);
    out
}

pub fn encode_v2(m: &Message) -> Vec<u8> {
    encode_v2_flags(m, 0)
}

/// Decode a V2 binary frame back into a `Message` (plus the flags byte).
pub fn decode_v2(b: &[u8]) -> Result<(Message, u8), ParseError> {
    // Need at least: version(1)+type(1)+id(4)+flags(1)+tlen(1) = 8 bytes.
    if b.len() < 8 {
        return Err(ParseError::WrongFieldCount);
    }
    if b[0] != V2_VERSION {
        return Err(ParseError::UnknownMessageType);
    }
    let mtype = type_from(b[1]).ok_or(ParseError::UnknownMessageType)?;
    let msg_id = u32::from_be_bytes([b[2], b[3], b[4], b[5]]);
    let flags = b[6];
    let tlen = b[7] as usize;
    let t_start = 8;
    let t_end = t_start + tlen;
    if b.len() < t_end + 2 {
        return Err(ParseError::WrongFieldCount);
    }
    let target = std::str::from_utf8(&b[t_start..t_end]).map_err(|_| ParseError::InvalidTarget)?;
    if target.is_empty() {
        return Err(ParseError::EmptyField);
    }
    let plen = u16::from_be_bytes([b[t_end], b[t_end + 1]]) as usize;
    let p_start = t_end + 2;
    let p_end = p_start + plen;
    if b.len() < p_end {
        return Err(ParseError::WrongFieldCount);
    }
    let payload_str = std::str::from_utf8(&b[p_start..p_end]).map_err(|_| ParseError::InvalidPayload)?;
    let payload = parse_payload(payload_str)?;
    Ok((Message { mtype, msg_id, target: target.to_string(), payload }, flags))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{parse_frame, Payload};

    fn roundtrip(v1: &str) {
        let m = parse_frame(v1).unwrap();
        let bin = encode_v2(&m);
        let (back, _flags) = decode_v2(&bin).unwrap();
        assert_eq!(back, m, "V2 round-trip must preserve the message");
    }

    #[test]
    fn v2_roundtrips_messages() {
        roundtrip("REQ|100|agentB|SUM(CTX:42)?max_words=150&lang=en");
        roundtrip("RES|100|agentA|OK(CTX:87)");
        roundtrip("DLT|101|broker|PATCH(CTX:42)?field=status&value=approved");
        roundtrip("SYN|1|broker|HELLO()?agent_id=agentA&caps=SUM,GEN&version=1");
    }

    #[test]
    fn v2_carries_flags_and_is_binary() {
        let m = parse_frame("REQ|7|agentB|SUM(CTX:42)").unwrap();
        let bin = encode_v2_flags(&m, FLAG_TAINTED);
        assert_eq!(bin[0], V2_VERSION);
        assert_eq!(bin[6], FLAG_TAINTED); // flags byte in the control plane
        let (back, flags) = decode_v2(&bin).unwrap();
        assert_eq!(back.msg_id, 7);
        assert_eq!(flags & FLAG_TAINTED, FLAG_TAINTED);
    }

    #[test]
    fn v2_rejects_bad_input() {
        assert_eq!(decode_v2(&[0u8; 3]).unwrap_err(), ParseError::WrongFieldCount);
        let mut bad = encode_v2(&Message::new(MessageType::Req, 1, "a", Payload::new("SUM").arg_ctx(1)));
        bad[0] = 0x09; // wrong version
        assert_eq!(decode_v2(&bad).unwrap_err(), ParseError::UnknownMessageType);
    }

    #[test]
    fn v2_control_header_is_compact() {
        // For a tiny message the binary control header is a handful of bytes (vs ASCII fields).
        let m = parse_frame("HBT|9|broker|PING()").unwrap();
        let bin = encode_v2(&m);
        assert!(bin.len() < 32);
    }
}
