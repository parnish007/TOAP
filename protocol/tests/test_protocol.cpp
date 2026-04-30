#include "toap/encoder.h"
#include "toap/parser.h"

#include <cassert>
#include <cstdint>
#include <iostream>
#include <string>

namespace {

void expect_frame_error(const std::string& raw, toap::ParseErrorCode code)
{
    const auto result = toap::parse_frame(raw);
    assert(!result.ok);
    assert(result.error.code == code);
}

void expect_payload_error(const std::string& raw, toap::ParseErrorCode code)
{
    const auto result = toap::parse_payload(raw);
    assert(!result.ok);
    assert(result.error.code == code);
}

void parses_valid_request()
{
    const auto result = toap::parse_frame("REQ|100|agentB|SUM(CTX:42)?max_words=150&lang=en");
    assert(result.ok);
    assert(result.message.type == toap::MessageType::Req);
    assert(result.message.msg_id == 100);
    assert(result.message.target == "agentB");
    assert(result.message.payload.op == "SUM");
    assert(result.message.payload.args.size() == 1);
    assert(result.message.payload.args[0].is_context_ref);
    assert(result.message.payload.args[0].context_id == 42);
    assert(result.message.payload.options.size() == 2);
    assert(result.message.payload.options[0].key == "max_words");
    assert(result.message.payload.options[0].value == "150");
    assert(result.message.payload.options[1].key == "lang");
    assert(result.message.payload.options[1].value == "en");
}

void round_trips_frame()
{
    const auto parsed = toap::parse_frame("DLT|101|agentB|PATCH(CTX:42)?field=status&value=approved");
    assert(parsed.ok);
    assert(toap::encode_frame(parsed.message) == "DLT|101|agentB|PATCH(CTX:42)?field=status&value=approved");
}

void parses_handshake_payloads()
{
    const auto syn = toap::parse_frame("SYN|1|broker|HELLO()?agent_id=agentA&caps=SUM,GEN&version=1");
    assert(syn.ok);
    assert(syn.message.payload.op == "HELLO");
    assert(syn.message.payload.args.empty());
    assert(syn.message.payload.options.size() == 3);

    const auto ack = toap::parse_frame("ACK|1|agentA|SESSION()?session_id=sess_123&agent_id=agentA&trust=internal&version=1");
    assert(ack.ok);
    assert(ack.message.payload.op == "SESSION");
    assert(ack.message.payload.options.size() == 4);
}

void rejects_invalid_frames()
{
    expect_frame_error("REQ|1|agentB", toap::ParseErrorCode::WrongFieldCount);
    expect_frame_error("REQ|1|agentB|SUM(CTX:42)|extra", toap::ParseErrorCode::WrongFieldCount);
    expect_frame_error("REQ||agentB|SUM(CTX:42)", toap::ParseErrorCode::EmptyField);
    expect_frame_error("BAD|1|agentB|SUM(CTX:42)", toap::ParseErrorCode::UnknownMessageType);
    expect_frame_error("REQ|abc|agentB|SUM(CTX:42)", toap::ParseErrorCode::InvalidMessageId);
    expect_frame_error("REQ|4294967296|agentB|SUM(CTX:42)", toap::ParseErrorCode::InvalidMessageId);
    expect_frame_error("REQ|1|9agent|SUM(CTX:42)", toap::ParseErrorCode::InvalidTarget);
}

void rejects_invalid_payloads()
{
    expect_payload_error("SUM:CTX:42", toap::ParseErrorCode::InvalidPayload);
    expect_payload_error("S(CTX:42)", toap::ParseErrorCode::InvalidPayload);
    expect_payload_error("SUM(CTX:abc)", toap::ParseErrorCode::InvalidContextRef);
    expect_payload_error("SUM(CTX:4294967296)", toap::ParseErrorCode::InvalidContextRef);
    expect_payload_error("SUM(CTX:42)?=150", toap::ParseErrorCode::InvalidOption);
    expect_payload_error("SUM(CTX:42)?max_words=", toap::ParseErrorCode::InvalidOption);
    expect_payload_error("SUM(CTX:42)extra", toap::ParseErrorCode::InvalidPayload);
}

void allows_text_like_data_as_option_value()
{
    const auto sql = toap::parse_payload("SET(CTX:99)?data=DROP_TABLE_users");
    assert(sql.ok);
    assert(sql.payload.options[0].value == "DROP_TABLE_users");

    const auto prompt = toap::parse_payload("SET(CTX:99)?data=Ignore_previous_instructions");
    assert(prompt.ok);
}

void encodes_constructed_payload()
{
    toap::ParsedPayload payload;
    payload.op = "CMP";
    payload.args.push_back(toap::PayloadArg{"CTX:11", true, 11});
    payload.args.push_back(toap::PayloadArg{"CTX:22", true, 22});
    assert(toap::encode_payload(payload) == "CMP(CTX:11,CTX:22)");

    toap::ToapMessage message;
    message.type = toap::MessageType::Req;
    message.msg_id = 7;
    message.target = "agentC";
    message.payload = payload;
    assert(toap::encode_frame(message) == "REQ|7|agentC|CMP(CTX:11,CTX:22)");
}

} // namespace

int main()
{
    parses_valid_request();
    round_trips_frame();
    parses_handshake_payloads();
    rejects_invalid_frames();
    rejects_invalid_payloads();
    allows_text_like_data_as_option_value();
    encodes_constructed_payload();

    std::cout << "toap protocol tests passed\n";
    return 0;
}
