// SPDX-License-Identifier: GPL-3.0-or-later
#include "../cpp/linux_video_codec.cpp"
#include <cassert>
#include <cstring>

int main()
{
    const uint8_t keyframe[] = {
        0x00, 0x00, 0x00, 0x01, 0x67, 0x42, 0xc0, 0x0a, 0xd9, 0x04, 0x26, 0xc0, 0x44, 0x00, 0x00, 0x03, 0x00,
        0x04, 0x00, 0x00, 0x03, 0x00, 0xc8, 0x3c, 0x48, 0x99, 0x20, 0x00, 0x00, 0x00, 0x01, 0x68, 0xcb, 0x83,
        0xcb, 0x20, 0x00, 0x00, 0x01, 0x65, 0x88, 0x84, 0x0a, 0xf1, 0x18, 0xa0, 0x00, 0x27, 0xa7, 0x1c, 0x00,
        0x04, 0xbe, 0x23, 0x80, 0x00, 0x82, 0xec, 0x9c, 0x9c, 0x9d, 0x75, 0xd7, 0x5d, 0x75, 0xd7, 0x5d, 0x75,
        0xd7, 0x5e,
    };
    Decoder decoder;
    const AVCodec* codec = decoder.functions.find_decoder("h264");
    assert(codec);
    decoder.initialize_software(codec);
    decoder.functions.check(decoder.functions.new_packet(decoder.packet, sizeof(keyframe)), "Allocate packet");
    std::memcpy(decoder.packet->data, keyframe, sizeof(keyframe));
    decoder.functions.check(decoder.functions.send_packet(decoder.context, decoder.packet), "Send packet");
    decoder.functions.check(decoder.functions.receive_frame(decoder.context, decoder.hardware), "Receive picture");
    const auto picture = decoded_picture(decoder, decoder.hardware);
    assert(picture.width == 64 && picture.height == 64);
    assert(picture.y && picture.uv && picture.stride_y >= 64 && picture.stride_uv >= 64);

    AVFrame* planar = decoder.functions.frame_alloc();
    assert(planar);
    uint8_t y[] = {1, 2, 3, 4, 0, 0, 5, 6, 7, 8, 0, 0};
    uint8_t u[] = {11, 12, 0};
    uint8_t v[] = {21, 22, 0};
    planar->width = 5;
    planar->height = 3;
    planar->format = AV_PIX_FMT_YUV420P;
    planar->data[0] = y;
    planar->data[1] = u;
    planar->data[2] = v;
    planar->linesize[0] = 6;
    planar->linesize[1] = 3;
    planar->linesize[2] = 3;
    const auto converted = decoded_picture(decoder, planar);
    assert(converted.width == 4 && converted.height == 2);
    assert(converted.stride_y == 6 && converted.stride_uv == 4);
    assert(converted.uv[0] == 11 && converted.uv[1] == 21);
    assert(converted.uv[2] == 12 && converted.uv[3] == 22);
    uint8_t nv12[] = {11, 21, 12, 22, 0, 0};
    planar->format = AV_PIX_FMT_NV12;
    planar->data[1] = nv12;
    planar->data[2] = nullptr;
    planar->linesize[1] = 6;
    const auto packed = decoded_picture(decoder, planar);
    assert(packed.width == 4 && packed.height == 2 && packed.stride_uv == 6);
    assert(packed.uv == nv12);
    decoder.functions.frame_free(&planar);

    char error[256]{};
    void* aac_enc = nectarlink_video_aac_encoder_create(48000, 2, error, sizeof(error));
    void* aac_dec = nectarlink_video_aac_decoder_create(error, sizeof(error));
    assert(aac_enc && aac_dec);
    std::vector<int16_t> pcm(4096 * 2, 1000);
    assert(nectarlink_video_aac_encoder_send(aac_enc, pcm.data(), 4096, error, sizeof(error)) == 0);
    assert(nectarlink_video_aac_encoder_send(aac_enc, nullptr, 0, error, sizeof(error)) == 0);
    const uint8_t* aac_bytes = nullptr;
    size_t aac_len = 0;
    int packets = 0;
    size_t total_decoded_frames = 0;
    while (nectarlink_video_aac_encoder_receive(aac_enc, &aac_bytes, &aac_len, error, sizeof(error)) == 1) {
        assert(aac_bytes && aac_len > 0);
        ++packets;
        const uint32_t full_len = static_cast<uint32_t>(7 + aac_len);
        // ADTS header for AAC-LC (profile=1), 48 kHz (freq_idx=3), 2 channels (chan_cfg=2).
        std::vector<uint8_t> adts = {
            0xFF,
            0xF1,
            static_cast<uint8_t>((1 << 6) | (3 << 2)),
            static_cast<uint8_t>((2 << 6) | ((full_len >> 11) & 0x03)),
            static_cast<uint8_t>((full_len >> 3) & 0xFF),
            static_cast<uint8_t>(((full_len & 0x07) << 5) | 0x1F),
            0xFC,
        };
        adts.insert(adts.end(), aac_bytes, aac_bytes + aac_len);
        assert(nectarlink_video_aac_decoder_send(aac_dec, adts.data(), adts.size(), error, sizeof(error)) == 0);
        const int16_t* dec_pcm = nullptr;
        size_t dec_frames = 0;
        int sr = 0, ch = 0;
        while (nectarlink_video_aac_decoder_receive(aac_dec, &dec_pcm, &dec_frames, &sr, &ch, error, sizeof(error)) == 1) {
            assert(dec_pcm && dec_frames > 0 && sr == 48000 && ch == 2);
            total_decoded_frames += dec_frames;
        }
    }
    assert(packets > 0 && total_decoded_frames > 0);
    nectarlink_video_aac_encoder_destroy(aac_enc);
    nectarlink_video_aac_decoder_destroy(aac_dec);
}
