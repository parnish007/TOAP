#pragma once

#include "toap/message.h"

#include <string>

namespace toap {

std::string encode_payload(const ParsedPayload& payload);
std::string encode_frame(const ToapMessage& message);

} // namespace toap
