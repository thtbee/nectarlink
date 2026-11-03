// SPDX-License-Identifier: GPL-3.0-or-later
#pragma once

#include <cstddef>
#include <cstdint>

extern "C" {
struct NectarlinkDecodedNv12
{
    const uint8_t* y;
    const uint8_t* uv;
    int width;
    int height;
    int stride_y;
    int stride_uv;
    bool bt601;
    bool full_range;
};

void* nectarlink_video_decoder_create(bool hevc, char* error, size_t capacity);
void nectarlink_video_decoder_destroy(void* decoder);
// After a successful switch, resume from a keyframe. False means all candidates failed.
bool nectarlink_video_decoder_next_device(void* decoder, char* error, size_t capacity);
const char* nectarlink_video_decoder_backend(const void* decoder);
int nectarlink_video_decoder_send(void* decoder, const uint8_t* bytes, size_t length,
                                 char* error, size_t capacity);
// The returned planes remain valid until receive, flush, or next_device is called again.
int nectarlink_video_decoder_receive(void* decoder, NectarlinkDecodedNv12* frame,
                                    char* error, size_t capacity);
void nectarlink_video_decoder_flush(void* decoder);

void* nectarlink_video_encoder_create(bool hevc, int width, int height, int fps, int bitrate,
                                     char* error, size_t capacity);
void nectarlink_video_encoder_destroy(void* encoder);
bool nectarlink_video_encoder_next_device(void* encoder, char* error, size_t capacity);
const char* nectarlink_video_encoder_backend(const void* encoder);
// Drain available output before sending another frame. Returns zero on success, negative on failure.
int nectarlink_video_encoder_send(void* encoder, const uint8_t* nv12, size_t length, int64_t time_us,
                                  char* error, size_t capacity);
// Packet storage belongs to the encoder and remains valid until the next receive call.
int nectarlink_video_encoder_receive(void* encoder, const uint8_t** bytes, size_t* length, bool* keyframe,
                                     char* error, size_t capacity);
void nectarlink_video_encoder_request_keyframe(void* encoder);
bool nectarlink_video_encoder_set_bitrate(void* encoder, int bitrate);

void* nectarlink_video_aac_encoder_create(int sample_rate, int channels, char* error, size_t capacity);
void nectarlink_video_aac_encoder_destroy(void* encoder);
// Pass nullptr or zero frames to flush remaining samples and signal end-of-stream.
int nectarlink_video_aac_encoder_send(void* encoder, const int16_t* pcm, size_t frames,
                                      char* error, size_t capacity);
// Packet storage belongs to the encoder and remains valid until the next receive call.
int nectarlink_video_aac_encoder_receive(void* encoder, const uint8_t** bytes, size_t* length,
                                         char* error, size_t capacity);

void* nectarlink_video_aac_decoder_create(char* error, size_t capacity);
void nectarlink_video_aac_decoder_destroy(void* decoder);
// Pass nullptr or zero length to signal end-of-stream.
int nectarlink_video_aac_decoder_send(void* decoder, const uint8_t* bytes, size_t length,
                                      char* error, size_t capacity);
// Sample storage belongs to the decoder and remains valid until the next receive call.
int nectarlink_video_aac_decoder_receive(void* decoder, const int16_t** pcm, size_t* frames,
                                         int* sample_rate, int* channels, char* error, size_t capacity);
}
