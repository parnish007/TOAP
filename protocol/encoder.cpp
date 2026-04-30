#include "toap/encoder.h"

#include <sstream>

namespace toap {

std::string encode_payload(const ParsedPayload& payload)
{
    std::ostringstream out;
    out << payload.op << '(';
    for (std::size_t i = 0; i < payload.args.size(); ++i) {
        if (i != 0) {
            out << ',';
        }
        out << payload.args[i].raw;
    }
    out << ')';

    if (!payload.options.empty()) {
        out << '?';
        for (std::size_t i = 0; i < payload.options.size(); ++i) {
            if (i != 0) {
                out << '&';
            }
            out << payload.options[i].key << '=' << payload.options[i].value;
        }
    }

    return out.str();
}

std::string encode_frame(const ToapMessage& message)
{
    std::ostringstream out;
    out << to_string(message.type)
        << '|'
        << message.msg_id
        << '|'
        << message.target
        << '|'
        << encode_payload(message.payload);

    return out.str();
}

} // namespace toap
