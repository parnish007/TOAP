#pragma once

#include <cstdint>
#include <string>
#include <vector>

namespace toap {

enum class MessageType {
    Syn,
    Ack,
    Req,
    Res,
    Err,
    Evt,
    Dlt,
    Bye,
    Hbt
};

enum class ParseErrorCode {
    None,
    WrongFieldCount,
    EmptyField,
    UnknownMessageType,
    InvalidMessageId,
    InvalidTarget,
    InvalidPayload,
    InvalidContextRef,
    InvalidOption
};

struct ParseError {
    ParseErrorCode code = ParseErrorCode::None;
    std::string message;
};

struct PayloadArg {
    std::string raw;
    bool is_context_ref = false;
    std::uint32_t context_id = 0;
};

struct PayloadOption {
    std::string key;
    std::string value;
};

struct ParsedPayload {
    std::string op;
    std::vector<PayloadArg> args;
    std::vector<PayloadOption> options;
};

struct ToapMessage {
    MessageType type = MessageType::Req;
    std::uint32_t msg_id = 0;
    std::string target;
    ParsedPayload payload;
    std::string raw_payload;
};

struct ParsePayloadResult {
    bool ok = false;
    ParsedPayload payload;
    ParseError error;
};

struct ParseFrameResult {
    bool ok = false;
    ToapMessage message;
    ParseError error;
};

std::string to_string(MessageType type);
bool parse_message_type(const std::string& value, MessageType& out);

} // namespace toap
