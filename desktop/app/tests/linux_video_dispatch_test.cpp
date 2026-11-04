// SPDX-License-Identifier: GPL-3.0-or-later
#if defined(TEST_BUILTIN) || defined(TEST_PLUGIN)
#ifdef TEST_BUILTIN
#include "../cpp/linux_video_builtin.h"
#endif
#include "../cpp/linux_video_codec.h"

#ifndef TEST_LABEL
#define TEST_LABEL "test"
#endif
namespace { int object; }

extern "C" void* nectarlink_video_decoder_create(bool, char*, size_t)
{
#ifdef TEST_AVAILABLE
    return &object;
#else
    return nullptr;
#endif
}
extern "C" void nectarlink_video_decoder_destroy(void*) {}
extern "C" bool nectarlink_video_decoder_next_device(void* p, char*, size_t) { return p == &object; }
extern "C" const char* nectarlink_video_decoder_backend(const void*) { return TEST_LABEL; }
extern "C" int nectarlink_video_decoder_send(void* p, const uint8_t*, size_t n, char*, size_t)
{ return p == &object ? static_cast<int>(n) : -1; }
extern "C" int nectarlink_video_decoder_receive(void*, NectarlinkDecodedNv12*, char*, size_t) { return 0; }
extern "C" void nectarlink_video_decoder_flush(void*) {}
extern "C" void* nectarlink_video_encoder_create(bool, int, int, int, int, char*, size_t)
{
#ifdef TEST_AVAILABLE
    return &object;
#else
    return nullptr;
#endif
}
extern "C" void nectarlink_video_encoder_destroy(void*) {}
extern "C" bool nectarlink_video_encoder_next_device(void* p, char*, size_t) { return p == &object; }
extern "C" const char* nectarlink_video_encoder_backend(const void*) { return TEST_LABEL; }
extern "C" int nectarlink_video_encoder_send(void* p, const uint8_t*, size_t n, int64_t, char*, size_t)
{ return p == &object ? static_cast<int>(n) : -1; }
extern "C" int nectarlink_video_encoder_receive(void*, const uint8_t**, size_t*, bool*, char*, size_t) { return 0; }
extern "C" void nectarlink_video_encoder_request_keyframe(void*) {}
extern "C" bool nectarlink_video_encoder_set_bitrate(void* p, int bitrate) { return p == &object && bitrate == 123; }
extern "C" void* nectarlink_video_aac_encoder_create(int, int, char*, size_t)
{
#ifdef TEST_AVAILABLE
    return &object;
#else
    return nullptr;
#endif
}
extern "C" void nectarlink_video_aac_encoder_destroy(void*) {}
extern "C" int nectarlink_video_aac_encoder_send(void* p, const int16_t*, size_t n, char*, size_t)
{ return p == &object ? static_cast<int>(n) : -1; }
extern "C" int nectarlink_video_aac_encoder_receive(void*, const uint8_t**, size_t*, char*, size_t) { return 0; }
extern "C" void* nectarlink_video_aac_decoder_create(char*, size_t)
{
#ifdef TEST_AVAILABLE
    return &object;
#else
    return nullptr;
#endif
}
extern "C" void nectarlink_video_aac_decoder_destroy(void*) {}
extern "C" int nectarlink_video_aac_decoder_send(void* p, const uint8_t*, size_t n, char*, size_t)
{ return p == &object ? static_cast<int>(n) : -1; }
extern "C" int nectarlink_video_aac_decoder_receive(void*, const int16_t**, size_t*, int*, int*, char*, size_t)
{ return 0; }

#else
#include "../cpp/linux_video_codec.h"
#include <cassert>
#include <cstring>

int main(int argc, char** argv)
{
    char error[128]{};
    auto* encoder = nectarlink_video_encoder_create(false, 320, 240, 30, 123, error, sizeof(error));
    auto* decoder = nectarlink_video_decoder_create(false, error, sizeof(error));
    auto* aac_encoder = nectarlink_video_aac_encoder_create(48000, 2, error, sizeof(error));
    auto* aac_decoder = nectarlink_video_aac_decoder_create(error, sizeof(error));
    if (argc > 1 && std::strcmp(argv[1], "none") == 0) {
        assert(!encoder && !decoder && !aac_encoder && !aac_decoder);
        return 0;
    }
    assert(encoder && decoder && aac_encoder && aac_decoder);
    const char* expected = argc > 1 ? argv[1] : "test";
    assert(std::strcmp(nectarlink_video_encoder_backend(encoder), expected) == 0);
    assert(std::strcmp(nectarlink_video_decoder_backend(decoder), expected) == 0);
    const uint8_t data[] = {1, 2, 3};
    assert(nectarlink_video_encoder_send(encoder, data, sizeof(data), 0, error, sizeof(error)) == 3);
    assert(nectarlink_video_decoder_send(decoder, data, sizeof(data), error, sizeof(error)) == 3);
    const int16_t pcm[] = {10, 20, 30, 40};
    assert(nectarlink_video_aac_encoder_send(aac_encoder, pcm, 2, error, sizeof(error)) == 2);
    assert(nectarlink_video_aac_decoder_send(aac_decoder, data, sizeof(data), error, sizeof(error)) == 3);
    NectarlinkDecodedNv12 frame{};
    assert(nectarlink_video_decoder_receive(decoder, &frame, error, sizeof(error)) == 0);
    const uint8_t* packet = nullptr;
    size_t length = 0;
    bool keyframe = false;
    assert(nectarlink_video_encoder_receive(encoder, &packet, &length, &keyframe, error, sizeof(error)) == 0);
    assert(nectarlink_video_aac_encoder_receive(aac_encoder, &packet, &length, error, sizeof(error)) == 0);
    const int16_t* decoded_pcm = nullptr;
    size_t decoded_frames = 0;
    int sr = 0, ch = 0;
    assert(nectarlink_video_aac_decoder_receive(aac_decoder, &decoded_pcm, &decoded_frames, &sr, &ch, error, sizeof(error)) == 0);
    assert(nectarlink_video_encoder_set_bitrate(encoder, 123));
    assert(nectarlink_video_encoder_next_device(encoder, error, sizeof(error)));
    assert(nectarlink_video_decoder_next_device(decoder, error, sizeof(error)));
    nectarlink_video_encoder_request_keyframe(encoder);
    nectarlink_video_decoder_flush(decoder);
    nectarlink_video_encoder_destroy(encoder);
    nectarlink_video_decoder_destroy(decoder);
    nectarlink_video_aac_encoder_destroy(aac_encoder);
    nectarlink_video_aac_decoder_destroy(aac_decoder);
}
#endif
