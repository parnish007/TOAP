#include "toap/parser.h"

#include <algorithm>
#include <cctype>
#include <limits>
#include <sstream>

namespace toap {
namespace {

ParsePayloadResult payload_error(ParseErrorCode code, std::string message)
{
    ParsePayloadResult result;
    result.ok = false;
    result.error = ParseError{code, std::move(message)};
    return result;
}

ParseFrameResult frame_error(ParseErrorCode code, std::string message)
{
    ParseFrameResult result;
    result.ok = false;
    result.error = ParseError{code, std::move(message)};
    return result;
}

bool is_digit_string(const std::string& value)
{
    return !value.empty()
        && std::all_of(value.begin(), value.end(), [](unsigned char ch) {
               return std::isdigit(ch) != 0;
           });
}

bool parse_uint32(const std::string& value, std::uint32_t& out)
{
    if (!is_digit_string(value)) {
        return false;
    }

    unsigned long long parsed = 0;
    for (const char ch : value) {
        parsed = parsed * 10 + static_cast<unsigned long long>(ch - '0');
        if (parsed > std::numeric_limits<std::uint32_t>::max()) {
            return false;
        }
    }

    out = static_cast<std::uint32_t>(parsed);
    return true;
}

bool is_valid_target(const std::string& target)
{
    if (target == "*" || target == "broker") {
        return true;
    }

    if (target.empty() || target.size() > 32) {
        return false;
    }

    const auto first = static_cast<unsigned char>(target.front());
    if (std::isalpha(first) == 0) {
        return false;
    }

    return std::all_of(target.begin(), target.end(), [](unsigned char ch) {
        return std::isalnum(ch) != 0 || ch == '_' || ch == '-';
    });
}

bool is_valid_op(const std::string& op)
{
    if (op.size() < 2 || op.size() > 16) {
        return false;
    }

    const auto first = static_cast<unsigned char>(op.front());
    if (first < 'A' || first > 'Z') {
        return false;
    }

    return std::all_of(op.begin(), op.end(), [](unsigned char ch) {
        return (ch >= 'A' && ch <= 'Z') || std::isdigit(ch) != 0 || ch == '_';
    });
}

bool is_valid_option_key(const std::string& key)
{
    if (key.empty() || key.size() > 32) {
        return false;
    }

    const auto first = static_cast<unsigned char>(key.front());
    if (std::isalpha(first) == 0) {
        return false;
    }

    return std::all_of(key.begin(), key.end(), [](unsigned char ch) {
        return std::isalnum(ch) != 0 || ch == '_';
    });
}

bool has_forbidden_arg_char(const std::string& value)
{
    return value.find_first_of("|(),?&=") != std::string::npos;
}

bool has_forbidden_option_value_char(const std::string& value)
{
    return value.find_first_of("|&") != std::string::npos;
}

std::vector<std::string> split(const std::string& value, char delimiter)
{
    std::vector<std::string> parts;
    std::string current;
    std::istringstream stream(value);
    while (std::getline(stream, current, delimiter)) {
        parts.push_back(current);
    }

    if (!value.empty() && value.back() == delimiter) {
        parts.emplace_back();
    }

    return parts;
}

ParsePayloadResult parse_arg(const std::string& raw, PayloadArg& out)
{
    if (raw.empty() || has_forbidden_arg_char(raw)) {
        return payload_error(ParseErrorCode::InvalidPayload, "invalid payload argument");
    }

    out.raw = raw;

    constexpr const char* context_prefix = "CTX:";
    if (raw.rfind(context_prefix, 0) == 0) {
        std::uint32_t context_id = 0;
        if (!parse_uint32(raw.substr(4), context_id)) {
            return payload_error(ParseErrorCode::InvalidContextRef, "invalid context reference");
        }
        out.is_context_ref = true;
        out.context_id = context_id;
    }

    return ParsePayloadResult{true, ParsedPayload{}, ParseError{}};
}

ParsePayloadResult parse_options(const std::string& raw_options, ParsedPayload& payload)
{
    if (raw_options.empty()) {
        return payload_error(ParseErrorCode::InvalidOption, "empty option section");
    }

    for (const auto& option_text : split(raw_options, '&')) {
        const auto equals = option_text.find('=');
        if (equals == std::string::npos || equals == 0 || equals == option_text.size() - 1) {
            return payload_error(ParseErrorCode::InvalidOption, "invalid option");
        }

        auto key = option_text.substr(0, equals);
        auto value = option_text.substr(equals + 1);
        if (!is_valid_option_key(key) || has_forbidden_option_value_char(value)) {
            return payload_error(ParseErrorCode::InvalidOption, "invalid option key or value");
        }

        payload.options.push_back(PayloadOption{std::move(key), std::move(value)});
    }

    return ParsePayloadResult{true, ParsedPayload{}, ParseError{}};
}

} // namespace

std::string to_string(MessageType type)
{
    switch (type) {
    case MessageType::Syn:
        return "SYN";
    case MessageType::Ack:
        return "ACK";
    case MessageType::Req:
        return "REQ";
    case MessageType::Res:
        return "RES";
    case MessageType::Err:
        return "ERR";
    case MessageType::Evt:
        return "EVT";
    case MessageType::Dlt:
        return "DLT";
    case MessageType::Bye:
        return "BYE";
    case MessageType::Hbt:
        return "HBT";
    }
    return {};
}

bool parse_message_type(const std::string& value, MessageType& out)
{
    if (value == "SYN") {
        out = MessageType::Syn;
    } else if (value == "ACK") {
        out = MessageType::Ack;
    } else if (value == "REQ") {
        out = MessageType::Req;
    } else if (value == "RES") {
        out = MessageType::Res;
    } else if (value == "ERR") {
        out = MessageType::Err;
    } else if (value == "EVT") {
        out = MessageType::Evt;
    } else if (value == "DLT") {
        out = MessageType::Dlt;
    } else if (value == "BYE") {
        out = MessageType::Bye;
    } else if (value == "HBT") {
        out = MessageType::Hbt;
    } else {
        return false;
    }
    return true;
}

ParsePayloadResult parse_payload(const std::string& raw_payload)
{
    const auto open = raw_payload.find('(');
    const auto close = raw_payload.rfind(')');
    if (open == std::string::npos || close == std::string::npos || close < open) {
        return payload_error(ParseErrorCode::InvalidPayload, "payload must use OP(args) syntax");
    }

    ParsedPayload payload;
    payload.op = raw_payload.substr(0, open);
    if (!is_valid_op(payload.op)) {
        return payload_error(ParseErrorCode::InvalidPayload, "invalid payload op");
    }

    const auto after_close = close + 1;
    if (after_close < raw_payload.size() && raw_payload[after_close] != '?') {
        return payload_error(ParseErrorCode::InvalidPayload, "unexpected text after payload arguments");
    }

    const auto args_text = raw_payload.substr(open + 1, close - open - 1);
    if (!args_text.empty()) {
        for (const auto& arg_text : split(args_text, ',')) {
            PayloadArg arg;
            auto arg_result = parse_arg(arg_text, arg);
            if (!arg_result.ok) {
                return arg_result;
            }
            payload.args.push_back(std::move(arg));
        }
    }

    if (after_close < raw_payload.size()) {
        auto options_result = parse_options(raw_payload.substr(after_close + 1), payload);
        if (!options_result.ok) {
            return options_result;
        }
    }

    return ParsePayloadResult{true, std::move(payload), ParseError{}};
}

ParseFrameResult parse_frame(const std::string& raw_frame)
{
    const auto fields = split(raw_frame, '|');
    if (fields.size() != 4) {
        return frame_error(ParseErrorCode::WrongFieldCount, "frame must contain exactly four fields");
    }

    if (std::any_of(fields.begin(), fields.end(), [](const std::string& field) {
            return field.empty();
        })) {
        return frame_error(ParseErrorCode::EmptyField, "frame contains an empty required field");
    }

    ToapMessage message;
    if (!parse_message_type(fields[0], message.type)) {
        return frame_error(ParseErrorCode::UnknownMessageType, "unknown message type");
    }

    if (!parse_uint32(fields[1], message.msg_id)) {
        return frame_error(ParseErrorCode::InvalidMessageId, "invalid message id");
    }

    if (!is_valid_target(fields[2])) {
        return frame_error(ParseErrorCode::InvalidTarget, "invalid target");
    }
    message.target = fields[2];
    message.raw_payload = fields[3];

    auto payload_result = parse_payload(fields[3]);
    if (!payload_result.ok) {
        return frame_error(payload_result.error.code, payload_result.error.message);
    }
    message.payload = std::move(payload_result.payload);

    return ParseFrameResult{true, std::move(message), ParseError{}};
}

} // namespace toap
