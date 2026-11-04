// SPDX-License-Identifier: GPL-3.0-or-later
#ifdef NECTARLINK_BUILTIN_VIDEO
#include "linux_video_builtin.h"
#endif
#include "linux_video_codec.h"

extern "C" {
#include <libavcodec/avcodec.h>
#include <libavutil/channel_layout.h>
#include <libavutil/dict.h>
#include <libavutil/hwcontext.h>
#include <libavutil/opt.h>
}

#include <algorithm>
#include <array>
#include <cstdio>
#include <cstring>
#include <dlfcn.h>
#include <filesystem>
#include <limits>
#include <memory>
#include <stdexcept>
#include <string>
#include <system_error>
#include <utility>
#include <vector>

#define STRINGIFY_IMPL(value) #value
#define STRINGIFY(value) STRINGIFY_IMPL(value)

namespace {

struct CloseLibrary
{
    void operator()(void* handle) const { dlclose(handle); }
};
using Library = std::unique_ptr<void, CloseLibrary>;

template<typename T>
T symbol(void* library, const char* name)
{
    auto function = reinterpret_cast<T>(dlsym(library, name));
    if (!function) throw std::runtime_error(std::string("Missing symbol: ") + name);
    return function;
}

// Compile against the installed headers and load only their matching ABI at runtime.
struct Api
{
    Library codec{dlopen("libavcodec.so." STRINGIFY(LIBAVCODEC_VERSION_MAJOR), RTLD_NOW | RTLD_LOCAL)};
    Library util{dlopen("libavutil.so." STRINGIFY(LIBAVUTIL_VERSION_MAJOR), RTLD_NOW | RTLD_LOCAL)};
    decltype(&avcodec_find_encoder_by_name) find_encoder;
    decltype(&avcodec_find_decoder_by_name) find_decoder;
    decltype(&avcodec_get_hw_config) hw_config;
    decltype(&avcodec_alloc_context3) alloc_context;
    decltype(&avcodec_free_context) free_context;
    decltype(&avcodec_open2) open;
    decltype(&avcodec_send_frame) send;
    decltype(&avcodec_receive_packet) receive;
    decltype(&avcodec_send_packet) send_packet;
    decltype(&avcodec_receive_frame) receive_frame;
    decltype(&avcodec_flush_buffers) flush;
    decltype(&av_packet_alloc) packet_alloc;
    decltype(&av_packet_free) packet_free;
    decltype(&av_packet_unref) packet_unref;
    decltype(&av_new_packet) new_packet;
    decltype(&av_frame_alloc) frame_alloc;
    decltype(&av_frame_free) frame_free;
    decltype(&av_frame_unref) frame_unref;
    decltype(&av_frame_get_buffer) frame_buffer;
    decltype(&av_frame_make_writable) frame_writable;
    decltype(&av_frame_copy_props) copy_properties;
    decltype(&av_frame_apply_cropping) crop;
    decltype(&av_hwdevice_ctx_create) device_create;
    decltype(&av_hwdevice_get_type_name) device_name;
    decltype(&av_hwframe_ctx_alloc) frames_alloc;
    decltype(&av_hwframe_ctx_init) frames_init;
    decltype(&av_hwframe_get_buffer) hardware_buffer;
    decltype(&av_hwframe_transfer_data) transfer;
    decltype(&av_buffer_unref) buffer_unref;
    decltype(&av_buffer_ref) buffer_ref;
    decltype(&av_opt_set) option;
    decltype(&av_opt_find) find_option;
    decltype(&av_opt_set_int) integer_option;
    decltype(&av_strerror) error_string;
    decltype(&av_dict_set) dict_set;
    decltype(&av_dict_free) dict_free;
    decltype(&av_channel_layout_default) channel_layout_default;

    Api()
    {
        if (!codec || !util) throw std::runtime_error("Matching FFmpeg runtime libraries are unavailable");
#define LOAD_CODEC(member, function) member = symbol<decltype(member)>(codec.get(), #function)
#define LOAD_UTIL(member, function) member = symbol<decltype(member)>(util.get(), #function)
        LOAD_CODEC(find_encoder, avcodec_find_encoder_by_name);
        LOAD_CODEC(find_decoder, avcodec_find_decoder_by_name);
        LOAD_CODEC(hw_config, avcodec_get_hw_config);
        LOAD_CODEC(alloc_context, avcodec_alloc_context3);
        LOAD_CODEC(free_context, avcodec_free_context);
        LOAD_CODEC(open, avcodec_open2);
        LOAD_CODEC(send, avcodec_send_frame);
        LOAD_CODEC(receive, avcodec_receive_packet);
        LOAD_CODEC(send_packet, avcodec_send_packet);
        LOAD_CODEC(receive_frame, avcodec_receive_frame);
        LOAD_CODEC(flush, avcodec_flush_buffers);
        LOAD_CODEC(packet_alloc, av_packet_alloc);
        LOAD_CODEC(packet_free, av_packet_free);
        LOAD_CODEC(packet_unref, av_packet_unref);
        LOAD_CODEC(new_packet, av_new_packet);
        LOAD_UTIL(frame_alloc, av_frame_alloc);
        LOAD_UTIL(frame_free, av_frame_free);
        LOAD_UTIL(frame_unref, av_frame_unref);
        LOAD_UTIL(frame_buffer, av_frame_get_buffer);
        LOAD_UTIL(frame_writable, av_frame_make_writable);
        LOAD_UTIL(copy_properties, av_frame_copy_props);
        LOAD_UTIL(crop, av_frame_apply_cropping);
        LOAD_UTIL(device_create, av_hwdevice_ctx_create);
        LOAD_UTIL(device_name, av_hwdevice_get_type_name);
        LOAD_UTIL(frames_alloc, av_hwframe_ctx_alloc);
        LOAD_UTIL(frames_init, av_hwframe_ctx_init);
        LOAD_UTIL(hardware_buffer, av_hwframe_get_buffer);
        LOAD_UTIL(transfer, av_hwframe_transfer_data);
        LOAD_UTIL(buffer_unref, av_buffer_unref);
        LOAD_UTIL(buffer_ref, av_buffer_ref);
        LOAD_UTIL(option, av_opt_set);
        LOAD_UTIL(find_option, av_opt_find);
        LOAD_UTIL(integer_option, av_opt_set_int);
        LOAD_UTIL(error_string, av_strerror);
        LOAD_UTIL(dict_set, av_dict_set);
        LOAD_UTIL(dict_free, av_dict_free);
        LOAD_UTIL(channel_layout_default, av_channel_layout_default);
#undef LOAD_CODEC
#undef LOAD_UTIL
    }

    void check(int result, const char* operation) const
    {
        if (result >= 0) return;
        char message[AV_ERROR_MAX_STRING_SIZE]{};
        error_string(result, message, sizeof(message));
        throw std::runtime_error(std::string(operation) + ": " + message);
    }

    void set_optional_option(void* object, const char* name, const char* value) const
    {
        if (find_option(object, name, nullptr, 0, 0))
            check(option(object, name, value, 0), name);
    }
};

const Api& api()
{
    static const Api instance;
    return instance;
}

std::vector<std::string> video_devices(AVHWDeviceType type)
{
    std::vector<std::string> devices;
    if (type == AV_HWDEVICE_TYPE_CUDA) {
        Library driver{dlopen("libcuda.so.1", RTLD_NOW | RTLD_LOCAL)};
        if (driver) {
            const auto initialize = symbol<int (*)(unsigned int)>(driver.get(), "cuInit");
            const auto count_devices = symbol<int (*)(int*)>(driver.get(), "cuDeviceGetCount");
            int count = 0;
            if (initialize(0) == 0 && count_devices(&count) == 0)
                for (int i = 0; i < count; ++i) devices.push_back(std::to_string(i));
        }
    } else if (type == AV_HWDEVICE_TYPE_VAAPI || type == AV_HWDEVICE_TYPE_QSV) {
        std::error_code error;
        std::filesystem::directory_iterator entries("/dev/dri", error), end;
        for (; !error && entries != end; entries.increment(error)) {
            const auto name = entries->path().filename().string();
            if (name.rfind("renderD", 0) == 0 && name.size() > 7
                && name.find_first_not_of("0123456789", 7) == std::string::npos)
                devices.push_back(entries->path().string());
        }
        std::sort(devices.begin(), devices.end());
    }
    if (devices.empty()) devices.emplace_back();
    return devices;
}

struct DeviceOptions
{
    const Api& functions;
    AVDictionary* values = nullptr;
    ~DeviceOptions() { functions.dict_free(&values); }
    void set(const char* key, const char* value) { functions.check(functions.dict_set(&values, key, value, 0), key); }
};

void create_device(const Api& functions, AVHWDeviceType type, const std::string& id, AVBufferRef** device)
{
    DeviceOptions options{functions};
    const char* selected = id.empty() ? nullptr : id.c_str();
    if (type == AV_HWDEVICE_TYPE_QSV) {
        options.set("child_device_type", "vaapi");
        if (selected) options.set("child_device", selected);
        selected = "hw_any";
    }
    functions.check(functions.device_create(device, type, selected, options.values, 0), "Create hardware device");
}

struct Encoder
{
    const Api& functions = api();
    AVCodecContext* context = nullptr;
    AVBufferRef* device = nullptr;
    AVFrame* software = nullptr;
    AVFrame* hardware = nullptr;
    AVPacket* packet = nullptr;
    AVHWDeviceType device_type = AV_HWDEVICE_TYPE_NONE;
    bool keyframe_requested = true;

    ~Encoder()
    {
        functions.packet_free(&packet);
        functions.frame_free(&hardware);
        functions.frame_free(&software);
        functions.free_context(&context);
        functions.buffer_unref(&device);
    }

    void configure_options()
    {
        functions.set_optional_option(context->priv_data, "async_depth", "1");
        if (device_type == AV_HWDEVICE_TYPE_CUDA) {
            functions.set_optional_option(context->priv_data, "forced-idr", "1");
            functions.set_optional_option(context->priv_data, "preset", "p1");
            functions.set_optional_option(context->priv_data, "tune", "ull");
            functions.set_optional_option(context->priv_data, "zerolatency", "1");
            functions.set_optional_option(context->priv_data, "delay", "0");
            functions.set_optional_option(context->priv_data, "rc-lookahead", "0");
        } else if (device_type == AV_HWDEVICE_TYPE_QSV) {
            functions.set_optional_option(context->priv_data, "preset", "veryfast");
            functions.set_optional_option(context->priv_data, "forced_idr", "1");
            functions.set_optional_option(context->priv_data, "look_ahead", "0");
            functions.set_optional_option(context->priv_data, "look_ahead_depth", "0");
        }
    }

    void initialize(const AVCodec* codec, int width, int height, int fps, int bitrate, bool low_power = false,
                    const std::string& device_id = {})
    {
        const AVCodecHWConfig* configuration = nullptr;
        for (int i = 0; (configuration = functions.hw_config(codec, i)); ++i) {
            if (configuration->device_type == AV_HWDEVICE_TYPE_QSV) {
                // FFmpeg handles QSV's surface alignment when supplied NV12 and a hardware device.
                if (configuration->pix_fmt == AV_PIX_FMT_NV12
                    && (configuration->methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX)) break;
            } else if (configuration->methods & AV_CODEC_HW_CONFIG_METHOD_HW_FRAMES_CTX) {
                break;
            }
        }
        if (!configuration) throw std::runtime_error("No hardware encoder configuration");
        device_type = configuration->device_type;
        create_device(functions, device_type, device_id, &device);
        context = functions.alloc_context(codec);
        if (!context) throw std::bad_alloc();
        context->width = width;
        context->height = height;
        context->time_base = AVRational{1, 1000000};
        context->framerate = AVRational{fps, 1};
        context->sample_aspect_ratio = AVRational{1, 1};
        context->pix_fmt = configuration->pix_fmt;
        context->bit_rate = bitrate;
        context->rc_max_rate = bitrate;
        context->rc_buffer_size = bitrate / 2;
        context->gop_size = fps * 2;
        context->max_b_frames = 0;
        context->flags |= AV_CODEC_FLAG_LOW_DELAY;
#if LIBAVCODEC_VERSION_MAJOR >= 61
        context->profile = codec->id == AV_CODEC_ID_HEVC ? AV_PROFILE_HEVC_MAIN : AV_PROFILE_H264_CONSTRAINED_BASELINE;
#else
        context->profile = codec->id == AV_CODEC_ID_HEVC ? FF_PROFILE_HEVC_MAIN : FF_PROFILE_H264_CONSTRAINED_BASELINE;
#endif
        context->color_range = AVCOL_RANGE_MPEG;
        context->colorspace = AVCOL_SPC_BT709;
        context->color_primaries = AVCOL_PRI_BT709;
        context->color_trc = AVCOL_TRC_BT709;

        if (device_type == AV_HWDEVICE_TYPE_QSV) {
            context->hw_device_ctx = functions.buffer_ref(device);
            if (!context->hw_device_ctx) throw std::bad_alloc();
        } else {
            context->hw_frames_ctx = functions.frames_alloc(device);
            if (!context->hw_frames_ctx) throw std::bad_alloc();
            auto* frames = reinterpret_cast<AVHWFramesContext*>(context->hw_frames_ctx->data);
            frames->format = context->pix_fmt;
            frames->sw_format = AV_PIX_FMT_NV12;
            frames->width = width;
            frames->height = height;
            // VAAPI uses a fixed pool; reserve space for reference and in-flight frames.
            frames->initial_pool_size = device_type == AV_HWDEVICE_TYPE_VAAPI ? 20 : 4;
            functions.check(functions.frames_init(context->hw_frames_ctx), "Create hardware frame pool");
        }

        configure_options();
        if (configuration->device_type == AV_HWDEVICE_TYPE_VAAPI)
            functions.set_optional_option(context->priv_data, "low_power", low_power ? "1" : "0");
        functions.check(functions.open(context, codec, nullptr), "Open video encoder");

        software = functions.frame_alloc();
        hardware = functions.frame_alloc();
        packet = functions.packet_alloc();
        if (!software || !hardware || !packet) throw std::bad_alloc();
        software->format = AV_PIX_FMT_NV12;
        software->width = width;
        software->height = height;
        functions.check(functions.frame_buffer(software, 32), "Allocate NV12 frame");
    }

    void send(const uint8_t* bytes, size_t length, int64_t time_us)
    {
        const size_t width = context->width;
        const size_t height = context->height;
        if (length != width * height * 3 / 2) throw std::runtime_error("Invalid NV12 frame length");
        functions.check(functions.frame_writable(software), "Make NV12 frame writable");
        for (size_t y = 0; y < height; ++y)
            std::memcpy(software->data[0] + y * software->linesize[0], bytes + y * width, width);
        for (size_t y = 0; y < height / 2; ++y)
            std::memcpy(software->data[1] + y * software->linesize[1], bytes + width * height + y * width, width);
        AVFrame* input = software;
        if (context->hw_frames_ctx) {
            functions.frame_unref(hardware);
            functions.check(functions.hardware_buffer(context->hw_frames_ctx, hardware, 0), "Allocate GPU frame");
            functions.check(functions.transfer(hardware, software, 0), "Upload NV12 frame");
            input = hardware;
        }
        input->pts = time_us;
        input->pict_type = keyframe_requested ? AV_PICTURE_TYPE_I : AV_PICTURE_TYPE_NONE;
        functions.check(functions.send(context, input), "Encode video frame");
        keyframe_requested = false;
    }
};

void report(char* buffer, size_t capacity, const std::exception& error)
{
    if (capacity) std::snprintf(buffer, capacity, "%s", error.what());
}

struct EncoderCandidate
{
    const AVCodec* codec;
    std::string device;
    bool low_power;
};

struct EncoderBackend
{
    AVHWDeviceType device_type;
    const char* h264;
    const char* hevc;
};

constexpr std::array<EncoderBackend, 3> encoder_backends = {{
    {AV_HWDEVICE_TYPE_CUDA, "h264_nvenc", "hevc_nvenc"},
    {AV_HWDEVICE_TYPE_VAAPI, "h264_vaapi", "hevc_vaapi"},
    {AV_HWDEVICE_TYPE_QSV, "h264_qsv", "hevc_qsv"},
}};

struct EncoderSelection
{
    const Api& functions = api();
    int width;
    int height;
    int fps;
    int bitrate;
    std::vector<EncoderCandidate> candidates;
    size_t next_candidate = 0;
    std::unique_ptr<Encoder> current;

    EncoderSelection(bool hevc, int width, int height, int fps, int bitrate)
        : width(width), height(height), fps(fps), bitrate(bitrate)
    {
        for (const auto& backend : encoder_backends) {
            const AVCodec* codec = functions.find_encoder(hevc ? backend.hevc : backend.h264);
            if (!codec) continue;
            for (const auto& device : video_devices(backend.device_type)) {
                candidates.push_back({codec, device, false});
                if (backend.device_type == AV_HWDEVICE_TYPE_VAAPI)
                    candidates.push_back({codec, device, true});
            }
        }
        select_next();
    }

    void select_next()
    {
        current.reset();
        std::string failures;
        while (next_candidate < candidates.size()) {
            const auto& candidate = candidates[next_candidate++];
            try {
                auto encoder = std::make_unique<Encoder>();
                encoder->initialize(candidate.codec, width, height, fps, bitrate, candidate.low_power, candidate.device);
                current = std::move(encoder);
                return;
            } catch (const std::exception& failure) {
                failures += std::string(candidate.codec->name) + "[" + candidate.device + "]: " + failure.what() + "; ";
            }
        }
        throw std::runtime_error(failures.empty() ? "No hardware encoder is available" : failures);
    }

    Encoder& active()
    {
        if (!current) throw std::runtime_error("Hardware encoding devices exhausted");
        return *current;
    }
};

struct Decoder
{
    const Api& functions = api();
    AVCodecContext* context = nullptr;
    AVBufferRef* device = nullptr;
    AVFrame* software = nullptr;
    AVFrame* hardware = nullptr;
    AVPacket* packet = nullptr;
    AVPixelFormat format = AV_PIX_FMT_NONE;
    std::vector<uint8_t> interleaved_uv;

    ~Decoder()
    {
        functions.packet_free(&packet);
        functions.frame_free(&hardware);
        functions.frame_free(&software);
        functions.free_context(&context);
        functions.buffer_unref(&device);
    }

    static AVPixelFormat choose_format(AVCodecContext* context, const AVPixelFormat* offered)
    {
        const auto wanted = static_cast<Decoder*>(context->opaque)->format;
        for (; *offered != AV_PIX_FMT_NONE; ++offered)
            if (*offered == wanted) return wanted;
        return AV_PIX_FMT_NONE;
    }

    void open(const AVCodec* codec)
    {
        context->thread_count = 1;
        context->flags |= AV_CODEC_FLAG_LOW_DELAY;
        functions.check(functions.open(context, codec, nullptr), "Open video decoder");
        software = functions.frame_alloc();
        hardware = functions.frame_alloc();
        packet = functions.packet_alloc();
        if (!software || !hardware || !packet) throw std::bad_alloc();
    }

    void initialize(const AVCodec* codec, const AVCodecHWConfig& configuration, const std::string& device_id = {})
    {
        create_device(functions, configuration.device_type, device_id, &device);
        context = functions.alloc_context(codec);
        if (!context) throw std::bad_alloc();
        context->hw_device_ctx = functions.buffer_ref(device);
        if (!context->hw_device_ctx) throw std::bad_alloc();
        format = configuration.pix_fmt;
        context->opaque = this;
        context->get_format = choose_format;
        open(codec);
    }

    void initialize_software(const AVCodec* codec)
    {
        context = functions.alloc_context(codec);
        if (!context) throw std::bad_alloc();
        open(codec);
    }
};

NectarlinkDecodedNv12 decoded_picture(Decoder& decoder, AVFrame* picture)
{
    decoder.functions.check(decoder.functions.crop(picture, 0), "Crop decoded picture");
    const int width = picture->width & ~1;
    const int height = picture->height & ~1;
    if (width <= 0 || height <= 0 || !picture->data[0] || !picture->data[1] || picture->linesize[0] < width)
        throw std::runtime_error("Unsupported decoded picture layout");
    const uint8_t* uv = picture->data[1];
    int uv_stride = picture->linesize[1];
    if (picture->format == AV_PIX_FMT_YUV420P || picture->format == AV_PIX_FMT_YUVJ420P) {
        const int half_width = width / 2;
        if (!picture->data[2] || picture->linesize[1] < half_width || picture->linesize[2] < half_width)
            throw std::runtime_error("Unsupported decoded picture layout");
        decoder.interleaved_uv.resize(static_cast<size_t>(width) * (height / 2));
        for (int row = 0; row < height / 2; ++row) {
            const auto* u = picture->data[1] + static_cast<size_t>(row) * picture->linesize[1];
            const auto* v = picture->data[2] + static_cast<size_t>(row) * picture->linesize[2];
            auto* destination = decoder.interleaved_uv.data() + static_cast<size_t>(row) * width;
            for (int x = 0; x < half_width; ++x) {
                destination[2 * x] = u[x];
                destination[2 * x + 1] = v[x];
            }
        }
        uv = decoder.interleaved_uv.data();
        uv_stride = width;
    } else if (picture->format != AV_PIX_FMT_NV12 || uv_stride < width) {
        throw std::runtime_error("Unsupported decoded picture layout");
    }
    return {picture->data[0], uv, width, height,
            picture->linesize[0], uv_stride,
            picture->colorspace == AVCOL_SPC_BT470BG || picture->colorspace == AVCOL_SPC_SMPTE170M,
            picture->color_range == AVCOL_RANGE_JPEG || picture->format == AV_PIX_FMT_YUVJ420P};
}

struct DecoderCandidate
{
    AVCodecHWConfig configuration;
    std::string device;
};

struct DecoderSelection
{
    const Api& functions = api();
    const AVCodec* codec;
    std::vector<DecoderCandidate> candidates;
    size_t next_candidate = 0;
    bool software_attempted = false;
    std::unique_ptr<Decoder> current;

    explicit DecoderSelection(bool hevc) : codec(functions.find_decoder(hevc ? "hevc" : "h264"))
    {
        if (!codec) throw std::runtime_error("Video decoder is unavailable");
        for (int i = 0; const auto* configuration = functions.hw_config(codec, i); ++i) {
            if (!(configuration->methods & AV_CODEC_HW_CONFIG_METHOD_HW_DEVICE_CTX)) continue;
            if (configuration->device_type != AV_HWDEVICE_TYPE_CUDA
                && configuration->device_type != AV_HWDEVICE_TYPE_VAAPI
                && configuration->device_type != AV_HWDEVICE_TYPE_QSV) continue;
            for (const auto& device : video_devices(configuration->device_type))
                candidates.push_back({*configuration, device});
        }
        select_next(true);
    }

    void select_next(bool allow_software = false)
    {
        current.reset();
        std::string failures;
        while (next_candidate < candidates.size()) {
            const auto& candidate = candidates[next_candidate++];
            try {
                auto decoder = std::make_unique<Decoder>();
                decoder->initialize(codec, candidate.configuration, candidate.device);
                current = std::move(decoder);
                return;
            } catch (const std::exception& failure) {
                failures += candidate.device + ": " + failure.what() + "; ";
            }
        }
        if (allow_software && !software_attempted) {
            software_attempted = true;
            auto decoder = std::make_unique<Decoder>();
            decoder->initialize_software(codec);
            current = std::move(decoder);
            return;
        }
        throw std::runtime_error(failures.empty() ? "No hardware decoder is available" : failures);
    }

    Decoder& active()
    {
        if (!current) throw std::runtime_error("Video decoding options exhausted");
        return *current;
    }
};

struct AacEncoder
{
    const Api& functions = api();
    AVCodecContext* context = nullptr;
    AVFrame* frame = nullptr;
    AVPacket* packet = nullptr;
    int channels = 0;
    int frame_size = 1024;
    std::vector<int16_t> pending;
    size_t pending_offset = 0;
    bool flushing = false;
    bool eof_sent = false;
    int64_t next_pts = 0;

    AacEncoder(int sample_rate, int channel_count) : channels(std::clamp(channel_count, 1, 2))
    {
        if (sample_rate <= 0) throw std::runtime_error("Invalid AAC sample rate");
        const AVCodec* codec = functions.find_encoder("aac");
        if (!codec) throw std::runtime_error("AAC encoder is unavailable");
        context = functions.alloc_context(codec);
        if (!context) throw std::bad_alloc();
        context->sample_fmt = AV_SAMPLE_FMT_FLTP;
        context->sample_rate = sample_rate;
        context->bit_rate = 128000;
        context->flags |= AV_CODEC_FLAG_GLOBAL_HEADER;
        functions.channel_layout_default(&context->ch_layout, channels);
        functions.check(functions.open(context, codec, nullptr), "Open AAC encoder");
        frame_size = context->frame_size > 0 ? context->frame_size : 1024;
        frame = functions.frame_alloc();
        packet = functions.packet_alloc();
        if (!frame || !packet) throw std::bad_alloc();
        frame->nb_samples = frame_size;
        frame->format = context->sample_fmt;
        frame->sample_rate = sample_rate;
        functions.channel_layout_default(&frame->ch_layout, channels);
        functions.check(functions.frame_buffer(frame, 0), "Allocate AAC frame");
    }

    ~AacEncoder()
    {
        functions.packet_free(&packet);
        functions.frame_free(&frame);
        functions.free_context(&context);
    }

    void send(const int16_t* pcm, size_t frames)
    {
        if (!pcm || frames == 0) {
            flushing = true;
            return;
        }
        pending.insert(pending.end(), pcm, pcm + frames * static_cast<size_t>(channels));
    }

    int receive(const uint8_t** bytes, size_t* length)
    {
        for (;;) {
            functions.packet_unref(packet);
            const int result = functions.receive(context, packet);
            if (result >= 0) {
                *bytes = packet->data;
                *length = static_cast<size_t>(packet->size);
                return 1;
            }
            if (result != AVERROR(EAGAIN)) {
                if (result == AVERROR_EOF) return 0;
                functions.check(result, "Receive AAC packet");
            }
            const size_t available_frames =
                (pending.size() - pending_offset) / static_cast<size_t>(channels);
            if (available_frames >= static_cast<size_t>(frame_size)
                || (flushing && available_frames > 0)) {
                const size_t chunk = std::min(available_frames, static_cast<size_t>(frame_size));
                functions.check(functions.frame_writable(frame), "Make AAC frame writable");
                for (int c = 0; c < channels; ++c) {
                    auto* plane = reinterpret_cast<float*>(frame->data[c]);
                    for (size_t i = 0; i < static_cast<size_t>(frame_size); ++i) {
                        if (i < chunk) {
                            const int16_t s = pending[pending_offset + i * static_cast<size_t>(channels) + c];
                            plane[i] = static_cast<float>(s) / 32768.0f;
                        } else {
                            plane[i] = 0.0f;
                        }
                    }
                }
                frame->pts = next_pts;
                next_pts += static_cast<int64_t>(chunk);
                pending_offset += chunk * static_cast<size_t>(channels);
                if (pending_offset == pending.size()) {
                    pending.clear();
                    pending_offset = 0;
                }
                functions.check(functions.send(context, frame), "Encode AAC frame");
                continue;
            }
            if (flushing && !eof_sent) {
                eof_sent = true;
                functions.check(functions.send(context, nullptr), "Flush AAC encoder");
                continue;
            }
            return 0;
        }
    }
};

struct AacDecoder
{
    const Api& functions = api();
    AVCodecContext* context = nullptr;
    AVFrame* frame = nullptr;
    AVPacket* packet = nullptr;
    std::vector<int16_t> samples;

    AacDecoder()
    {
        const AVCodec* codec = functions.find_decoder("aac");
        if (!codec) throw std::runtime_error("AAC decoder is unavailable");
        context = functions.alloc_context(codec);
        if (!context) throw std::bad_alloc();
        functions.check(functions.open(context, codec, nullptr), "Open AAC decoder");
        frame = functions.frame_alloc();
        packet = functions.packet_alloc();
        if (!frame || !packet) throw std::bad_alloc();
    }

    ~AacDecoder()
    {
        functions.packet_free(&packet);
        functions.frame_free(&frame);
        functions.free_context(&context);
    }

    void send(const uint8_t* bytes, size_t length)
    {
        if (!bytes || length == 0) {
            functions.check(functions.send_packet(context, nullptr), "Flush AAC decoder");
            return;
        }
        if (length > static_cast<size_t>(std::numeric_limits<int>::max()))
            throw std::runtime_error("AAC packet is too large");
        functions.packet_unref(packet);
        functions.check(functions.new_packet(packet, static_cast<int>(length)), "Allocate AAC packet");
        std::memcpy(packet->data, bytes, length);
        functions.check(functions.send_packet(context, packet), "Submit AAC packet");
    }

    int receive(const int16_t** pcm, size_t* frames, int* sample_rate, int* channels)
    {
        functions.frame_unref(frame);
        const int result = functions.receive_frame(context, frame);
        if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) return 0;
        functions.check(result, "Receive decoded AAC frame");
        const int n = frame->nb_samples;
        const int ch = std::clamp(frame->ch_layout.nb_channels, 1, 2);
        if (n <= 0) return 0;
        samples.resize(static_cast<size_t>(n) * static_cast<size_t>(ch));
        if (frame->format == AV_SAMPLE_FMT_FLTP) {
            for (int i = 0; i < n; ++i) {
                for (int c = 0; c < ch; ++c) {
                    const auto* plane = reinterpret_cast<const float*>(frame->data[c]);
                    const float v = plane ? std::clamp(plane[i], -1.0f, 1.0f) : 0.0f;
                    samples[static_cast<size_t>(i * ch + c)] = static_cast<int16_t>(v * 32767.0f);
                }
            }
        } else if (frame->format == AV_SAMPLE_FMT_S16P) {
            for (int i = 0; i < n; ++i) {
                for (int c = 0; c < ch; ++c) {
                    const auto* plane = reinterpret_cast<const int16_t*>(frame->data[c]);
                    samples[static_cast<size_t>(i * ch + c)] = plane ? plane[i] : 0;
                }
            }
        } else if (frame->format == AV_SAMPLE_FMT_FLT && frame->data[0]) {
            const auto* src = reinterpret_cast<const float*>(frame->data[0]);
            for (size_t i = 0; i < samples.size(); ++i)
                samples[i] = static_cast<int16_t>(std::clamp(src[i], -1.0f, 1.0f) * 32767.0f);
        } else if (frame->format == AV_SAMPLE_FMT_S16 && frame->data[0]) {
            std::memcpy(samples.data(), frame->data[0], samples.size() * sizeof(int16_t));
        } else {
            throw std::runtime_error("Unsupported decoded AAC sample format");
        }
        *pcm = samples.data();
        *frames = static_cast<size_t>(n);
        *sample_rate = frame->sample_rate;
        *channels = ch;
        return 1;
    }
};
} // namespace

extern "C" void* nectarlink_video_encoder_create(bool hevc, int width, int height, int fps, int bitrate,
                                               char* error, size_t capacity)
{
    try {
        if (width <= 0 || height <= 0 || width % 2 || height % 2 || fps <= 0 || bitrate <= 0)
            throw std::runtime_error("Invalid video encoder dimensions or rate");
        return new EncoderSelection(hevc, width, height, fps, bitrate);
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return nullptr;
    }
}

extern "C" void nectarlink_video_encoder_destroy(void* encoder) { delete static_cast<EncoderSelection*>(encoder); }
extern "C" bool nectarlink_video_encoder_next_device(void* pointer, char* error, size_t capacity)
{
    try {
        static_cast<EncoderSelection*>(pointer)->select_next();
        return true;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return false;
    }
}
extern "C" const char* nectarlink_video_encoder_backend(const void* encoder)
{
    const auto& selected = *static_cast<const EncoderSelection*>(encoder);
    return selected.current ? selected.current->context->codec->name : "unavailable";
}

extern "C" int nectarlink_video_encoder_send(void* encoder, const uint8_t* bytes, size_t length, int64_t time_us,
                                           char* error, size_t capacity)
{
    try {
        static_cast<EncoderSelection*>(encoder)->active().send(bytes, length, time_us);
        return 0;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}

extern "C" int nectarlink_video_encoder_receive(void* pointer, const uint8_t** bytes, size_t* length, bool* keyframe,
                                              char* error, size_t capacity)
{
    try {
        auto& encoder = static_cast<EncoderSelection*>(pointer)->active();
        encoder.functions.packet_unref(encoder.packet);
        const int result = encoder.functions.receive(encoder.context, encoder.packet);
        if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) return 0;
        encoder.functions.check(result, "Receive video packet");
        *bytes = encoder.packet->data;
        *length = encoder.packet->size;
        *keyframe = (encoder.packet->flags & AV_PKT_FLAG_KEY) != 0;
        return 1;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}

extern "C" void nectarlink_video_encoder_request_keyframe(void* pointer)
{
    auto& selected = *static_cast<EncoderSelection*>(pointer);
    if (selected.current) selected.current->keyframe_requested = true;
}

extern "C" bool nectarlink_video_encoder_set_bitrate(void* pointer, int bitrate)
{
    auto& selected = *static_cast<EncoderSelection*>(pointer);
    if (!selected.current) return false;
    auto& encoder = *selected.current;
    if (bitrate <= 0) return false;
    // NVENC and QSV reconfigure these fields on the next submitted frame.
    if (encoder.device_type == AV_HWDEVICE_TYPE_CUDA || encoder.device_type == AV_HWDEVICE_TYPE_QSV) {
        encoder.context->bit_rate = bitrate;
        encoder.context->rc_max_rate = bitrate;
        encoder.context->rc_buffer_size = bitrate / 2;
        selected.bitrate = bitrate;
        return true;
    }
    const auto* option = encoder.functions.find_option(encoder.context, "b", nullptr, 0, 0);
    const bool changed = option && (option->flags & AV_OPT_FLAG_RUNTIME_PARAM)
        && encoder.functions.integer_option(encoder.context, "b", bitrate, 0) >= 0;
    if (changed) selected.bitrate = bitrate;
    return changed;
}

extern "C" void* nectarlink_video_decoder_create(bool hevc, char* error, size_t capacity)
{
    try {
        return new DecoderSelection(hevc);
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return nullptr;
    }
}

extern "C" void nectarlink_video_decoder_destroy(void* decoder) { delete static_cast<DecoderSelection*>(decoder); }

extern "C" bool nectarlink_video_decoder_next_device(void* pointer, char* error, size_t capacity)
{
    try {
        static_cast<DecoderSelection*>(pointer)->select_next(true);
        return true;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return false;
    }
}

extern "C" const char* nectarlink_video_decoder_backend(const void* pointer)
{
    const auto& selected = *static_cast<const DecoderSelection*>(pointer);
    if (!selected.current) return "unavailable";
    const auto& decoder = *selected.current;
    if (!decoder.device) return "software";
    const auto* device = reinterpret_cast<const AVHWDeviceContext*>(decoder.device->data);
    const char* name = decoder.functions.device_name(device->type);
    return name ? name : "unknown";
}

extern "C" int nectarlink_video_decoder_send(void* pointer, const uint8_t* bytes, size_t length,
                                           char* error, size_t capacity)
{
    try {
        auto& decoder = static_cast<DecoderSelection*>(pointer)->active();
        if (length > std::numeric_limits<int>::max()) throw std::runtime_error("Video packet is too large");
        decoder.functions.packet_unref(decoder.packet);
        decoder.functions.check(decoder.functions.new_packet(decoder.packet, static_cast<int>(length)),
                                "Allocate compressed packet");
        std::memcpy(decoder.packet->data, bytes, length);
        decoder.functions.check(decoder.functions.send_packet(decoder.context, decoder.packet),
                                "Submit compressed packet");
        return 0;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}

extern "C" int nectarlink_video_decoder_receive(void* pointer, NectarlinkDecodedNv12* frame,
                                              char* error, size_t capacity)
{
    try {
        auto& decoder = static_cast<DecoderSelection*>(pointer)->active();
        decoder.functions.frame_unref(decoder.hardware);
        const int result = decoder.functions.receive_frame(decoder.context, decoder.hardware);
        if (result == AVERROR(EAGAIN) || result == AVERROR_EOF) return 0;
        decoder.functions.check(result, "Receive decoded picture");
        AVFrame* picture = decoder.hardware;
        if (decoder.device) {
            decoder.functions.frame_unref(decoder.software);
            decoder.software->format = AV_PIX_FMT_NV12;
            decoder.functions.check(decoder.functions.transfer(decoder.software, decoder.hardware, 0),
                                    "Download decoded picture");
            decoder.functions.check(decoder.functions.copy_properties(decoder.software, decoder.hardware),
                                    "Copy picture metadata");
            picture = decoder.software;
        }
        *frame = decoded_picture(decoder, picture);
        return 1;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}

extern "C" void nectarlink_video_decoder_flush(void* pointer)
{
    auto& selected = *static_cast<DecoderSelection*>(pointer);
    if (!selected.current) return;
    auto& decoder = *selected.current;
    decoder.functions.flush(decoder.context);
    decoder.functions.frame_unref(decoder.hardware);
    decoder.functions.frame_unref(decoder.software);
    decoder.functions.packet_unref(decoder.packet);
}

extern "C" void* nectarlink_video_aac_encoder_create(int sample_rate, int channels, char* error, size_t capacity)
{
    try {
        return new AacEncoder(sample_rate, channels);
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return nullptr;
    }
}

extern "C" void nectarlink_video_aac_encoder_destroy(void* encoder)
{
    delete static_cast<AacEncoder*>(encoder);
}

extern "C" int nectarlink_video_aac_encoder_send(void* encoder, const int16_t* pcm, size_t frames,
                                                 char* error, size_t capacity)
{
    try {
        static_cast<AacEncoder*>(encoder)->send(pcm, frames);
        return 0;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}

extern "C" int nectarlink_video_aac_encoder_receive(void* encoder, const uint8_t** bytes, size_t* length,
                                                    char* error, size_t capacity)
{
    try {
        return static_cast<AacEncoder*>(encoder)->receive(bytes, length);
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}

extern "C" void* nectarlink_video_aac_decoder_create(char* error, size_t capacity)
{
    try {
        return new AacDecoder();
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return nullptr;
    }
}

extern "C" void nectarlink_video_aac_decoder_destroy(void* decoder)
{
    delete static_cast<AacDecoder*>(decoder);
}

extern "C" int nectarlink_video_aac_decoder_send(void* decoder, const uint8_t* bytes, size_t length,
                                                 char* error, size_t capacity)
{
    try {
        static_cast<AacDecoder*>(decoder)->send(bytes, length);
        return 0;
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}

extern "C" int nectarlink_video_aac_decoder_receive(void* decoder, const int16_t** pcm, size_t* frames,
                                                    int* sample_rate, int* channels, char* error, size_t capacity)
{
    try {
        return static_cast<AacDecoder*>(decoder)->receive(pcm, frames, sample_rate, channels);
    } catch (const std::exception& failure) {
        report(error, capacity, failure);
        return -1;
    }
}
