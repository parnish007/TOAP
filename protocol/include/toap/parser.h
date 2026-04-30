#pragma once

#include "toap/message.h"

#include <string>

namespace toap {

ParsePayloadResult parse_payload(const std::string& raw_payload);
ParseFrameResult parse_frame(const std::string& raw_frame);

} // namespace toap
