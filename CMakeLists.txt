cmake_minimum_required(VERSION 3.20)

project(TOAP
    VERSION 0.1.0
    DESCRIPTION "Token-Optimized Agent Protocol"
    LANGUAGES CXX
)

set(CMAKE_CXX_STANDARD 17)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_CXX_EXTENSIONS OFF)

option(TOAP_BUILD_TESTS "Build TOAP tests" ON)

add_library(toap_project_options INTERFACE)
target_compile_features(toap_project_options INTERFACE cxx_std_17)

if(MSVC)
    target_compile_options(toap_project_options INTERFACE /W4 /permissive-)
else()
    target_compile_options(toap_project_options INTERFACE -Wall -Wextra -Wpedantic)
endif()

add_library(toap_protocol
    protocol/encoder.cpp
    protocol/parser.cpp
)

target_include_directories(toap_protocol
    PUBLIC
        ${CMAKE_CURRENT_SOURCE_DIR}/protocol/include
)

target_link_libraries(toap_protocol
    PUBLIC
        toap_project_options
)

if(TOAP_BUILD_TESTS)
    enable_testing()

    add_executable(toap_protocol_tests
        protocol/tests/test_protocol.cpp
    )

target_link_libraries(toap_protocol_tests
    PRIVATE
        toap_protocol
        toap_project_options
)

    if(WIN32 AND CMAKE_CXX_COMPILER_ID STREQUAL "GNU")
        target_link_options(toap_protocol_tests
            PRIVATE
                -static-libgcc
                -static-libstdc++
                -static
        )
    endif()

    add_test(NAME toap_protocol_tests COMMAND toap_protocol_tests)
endif()

message(STATUS "TOAP phase 2 protocol core configured.")
