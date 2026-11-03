// SPDX-License-Identifier: GPL-3.0-or-later
#include "linux_video_builtin.h"
#include "linux_video_codec.h"

#undef nectarlink_video_decoder_create
#undef nectarlink_video_decoder_destroy
#undef nectarlink_video_decoder_next_device
#undef nectarlink_video_decoder_backend
#undef nectarlink_video_decoder_send
#undef nectarlink_video_decoder_receive
#undef nectarlink_video_decoder_flush
#undef nectarlink_video_encoder_create
#undef nectarlink_video_encoder_destroy
#undef nectarlink_video_encoder_next_device
#undef nectarlink_video_encoder_backend
#undef nectarlink_video_encoder_send
#undef nectarlink_video_encoder_receive
#undef nectarlink_video_encoder_request_keyframe
#undef nectarlink_video_encoder_set_bitrate
#undef nectarlink_video_aac_encoder_create
#undef nectarlink_video_aac_encoder_destroy
#undef nectarlink_video_aac_encoder_send
#undef nectarlink_video_aac_encoder_receive
#undef nectarlink_video_aac_decoder_create
#undef nectarlink_video_aac_decoder_destroy
#undef nectarlink_video_aac_decoder_send
#undef nectarlink_video_aac_decoder_receive

#include <array>
#include <cstdio>
#include <dlfcn.h>
#include <exception>
#include <filesystem>
#include <memory>
#include <new>
#include <string>
#include <system_error>

namespace {
struct Backend {
    decltype(&builtin_video_decoder_create) decoder_create;
    decltype(&builtin_video_decoder_destroy) decoder_destroy;
    decltype(&builtin_video_decoder_next_device) decoder_next_device;
    decltype(&builtin_video_decoder_backend) decoder_backend;
    decltype(&builtin_video_decoder_send) decoder_send;
    decltype(&builtin_video_decoder_receive) decoder_receive;
    decltype(&builtin_video_decoder_flush) decoder_flush;
    decltype(&builtin_video_encoder_create) encoder_create;
    decltype(&builtin_video_encoder_destroy) encoder_destroy;
    decltype(&builtin_video_encoder_next_device) encoder_next_device;
    decltype(&builtin_video_encoder_backend) encoder_backend;
    decltype(&builtin_video_encoder_send) encoder_send;
    decltype(&builtin_video_encoder_receive) encoder_receive;
    decltype(&builtin_video_encoder_request_keyframe) encoder_request_keyframe;
    decltype(&builtin_video_encoder_set_bitrate) encoder_set_bitrate;
    decltype(&builtin_video_aac_encoder_create) aac_encoder_create;
    decltype(&builtin_video_aac_encoder_destroy) aac_encoder_destroy;
    decltype(&builtin_video_aac_encoder_send) aac_encoder_send;
    decltype(&builtin_video_aac_encoder_receive) aac_encoder_receive;
    decltype(&builtin_video_aac_decoder_create) aac_decoder_create;
    decltype(&builtin_video_aac_decoder_destroy) aac_decoder_destroy;
    decltype(&builtin_video_aac_decoder_send) aac_decoder_send;
    decltype(&builtin_video_aac_decoder_receive) aac_decoder_receive;
};

const Backend builtin = {
    builtin_video_decoder_create, builtin_video_decoder_destroy, builtin_video_decoder_next_device,
    builtin_video_decoder_backend, builtin_video_decoder_send, builtin_video_decoder_receive,
    builtin_video_decoder_flush, builtin_video_encoder_create, builtin_video_encoder_destroy,
    builtin_video_encoder_next_device, builtin_video_encoder_backend, builtin_video_encoder_send,
    builtin_video_encoder_receive, builtin_video_encoder_request_keyframe, builtin_video_encoder_set_bitrate,
    builtin_video_aac_encoder_create, builtin_video_aac_encoder_destroy, builtin_video_aac_encoder_send,
    builtin_video_aac_encoder_receive, builtin_video_aac_decoder_create, builtin_video_aac_decoder_destroy,
    builtin_video_aac_decoder_send, builtin_video_aac_decoder_receive,
};

template<typename T> bool load_symbol(void* library, T& target, const char* name)
{
    target = reinterpret_cast<T>(dlsym(library, name));
    return target != nullptr;
}

struct CloseLibrary {
    void operator()(void* library) const { dlclose(library); }
};

std::unique_ptr<Backend> load_backend(const std::filesystem::path& path)
{
    std::unique_ptr<void, CloseLibrary> library(dlopen(path.c_str(), RTLD_NOW | RTLD_LOCAL));
    if (!library) return {};
    auto result = std::make_unique<Backend>();
#define LOAD(name) load_symbol(library.get(), result->name, "nectarlink_video_" #name)
    if (!(LOAD(decoder_create) && LOAD(decoder_destroy) && LOAD(decoder_next_device)
        && LOAD(decoder_backend) && LOAD(decoder_send) && LOAD(decoder_receive) && LOAD(decoder_flush)
        && LOAD(encoder_create) && LOAD(encoder_destroy) && LOAD(encoder_next_device)
        && LOAD(encoder_backend) && LOAD(encoder_send) && LOAD(encoder_receive)
        && LOAD(encoder_request_keyframe) && LOAD(encoder_set_bitrate)
        && LOAD(aac_encoder_create) && LOAD(aac_encoder_destroy)
        && LOAD(aac_encoder_send) && LOAD(aac_encoder_receive)
        && LOAD(aac_decoder_create) && LOAD(aac_decoder_destroy)
        && LOAD(aac_decoder_send) && LOAD(aac_decoder_receive))) {
        return {};
    }
#undef LOAD
    // Keep the library loaded for the lifetime of its function pointers.
    library.release();
    return result;
}

const auto& alternates()
{
    static const auto backends = [] {
        std::array<std::unique_ptr<Backend>, 3> result;
        std::error_code error;
        const auto executable = std::filesystem::read_symlink("/proc/self/exe", error);
        if (error) return result;
        for (size_t i = 0; i < result.size(); ++i) {
            const auto name = "libnectarlink-video-ffmpeg" + std::to_string(i + 7) + ".so";
            result[i] = load_backend(executable.parent_path() / name);
        }
        return result;
    }();
    return backends;
}

struct Handle {
    const Backend* backend;
    void* object;
};

template<typename Create, typename Destroy>
void* create(Create create, Destroy destroy, char* error, size_t capacity)
{
    auto attempt = [&](const Backend* backend) -> void* {
        if (!backend) return nullptr;
        void* object = create(*backend);
        if (!object) return nullptr;
        auto* handle = new (std::nothrow) Handle{backend, object};
        if (handle) return handle;
        destroy(*backend, object);
        if (capacity) std::snprintf(error, capacity, "Out of memory");
        return nullptr;
    };
    try {
        const auto& candidates = alternates();
        for (auto it = candidates.rbegin(); it != candidates.rend(); ++it) {
            if (auto* handle = attempt(it->get())) return handle;
        }
        return attempt(&builtin);
    } catch (const std::exception& failure) {
        if (capacity) std::snprintf(error, capacity, "%s", failure.what());
    }
    return nullptr;
}
}

extern "C" void* nectarlink_video_decoder_create(bool hevc, char* error, size_t capacity)
{
    return create([&](const Backend& b) { return b.decoder_create(hevc, error, capacity); },
                  [](const Backend& b, void* p) { b.decoder_destroy(p); }, error, capacity);
}
extern "C" void nectarlink_video_decoder_destroy(void* p)
{
    auto* h = static_cast<Handle*>(p);
    h->backend->decoder_destroy(h->object);
    delete h;
}
extern "C" bool nectarlink_video_decoder_next_device(void* p, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->decoder_next_device(h->object, error, n);
}
extern "C" const char* nectarlink_video_decoder_backend(const void* p)
{
    auto* h = static_cast<const Handle*>(p);
    return h->backend->decoder_backend(h->object);
}
extern "C" int nectarlink_video_decoder_send(void* p, const uint8_t* data, size_t len, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->decoder_send(h->object, data, len, error, n);
}
extern "C" int nectarlink_video_decoder_receive(void* p, NectarlinkDecodedNv12* frame, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->decoder_receive(h->object, frame, error, n);
}
extern "C" void nectarlink_video_decoder_flush(void* p)
{
    auto* h = static_cast<Handle*>(p);
    h->backend->decoder_flush(h->object);
}

extern "C" void* nectarlink_video_encoder_create(bool hevc, int width, int height, int fps, int bitrate,
                                                   char* error, size_t capacity)
{
    return create([&](const Backend& b) { return b.encoder_create(hevc, width, height, fps, bitrate, error, capacity); },
                  [](const Backend& b, void* p) { b.encoder_destroy(p); }, error, capacity);
}
extern "C" void nectarlink_video_encoder_destroy(void* p)
{
    auto* h = static_cast<Handle*>(p);
    h->backend->encoder_destroy(h->object);
    delete h;
}
extern "C" bool nectarlink_video_encoder_next_device(void* p, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->encoder_next_device(h->object, error, n);
}
extern "C" const char* nectarlink_video_encoder_backend(const void* p)
{
    auto* h = static_cast<const Handle*>(p);
    return h->backend->encoder_backend(h->object);
}
extern "C" int nectarlink_video_encoder_send(void* p, const uint8_t* data, size_t len, int64_t time,
                                             char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->encoder_send(h->object, data, len, time, error, n);
}
extern "C" int nectarlink_video_encoder_receive(void* p, const uint8_t** data, size_t* len, bool* key,
                                                char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->encoder_receive(h->object, data, len, key, error, n);
}
extern "C" void nectarlink_video_encoder_request_keyframe(void* p)
{
    auto* h = static_cast<Handle*>(p);
    h->backend->encoder_request_keyframe(h->object);
}
extern "C" bool nectarlink_video_encoder_set_bitrate(void* p, int bitrate)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->encoder_set_bitrate(h->object, bitrate);
}

extern "C" void* nectarlink_video_aac_encoder_create(int sample_rate, int channels, char* error, size_t capacity)
{
    return create([&](const Backend& b) { return b.aac_encoder_create(sample_rate, channels, error, capacity); },
                  [](const Backend& b, void* p) { b.aac_encoder_destroy(p); }, error, capacity);
}
extern "C" void nectarlink_video_aac_encoder_destroy(void* p)
{
    auto* h = static_cast<Handle*>(p);
    h->backend->aac_encoder_destroy(h->object);
    delete h;
}
extern "C" int nectarlink_video_aac_encoder_send(void* p, const int16_t* pcm, size_t frames, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->aac_encoder_send(h->object, pcm, frames, error, n);
}
extern "C" int nectarlink_video_aac_encoder_receive(void* p, const uint8_t** bytes, size_t* len, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->aac_encoder_receive(h->object, bytes, len, error, n);
}

extern "C" void* nectarlink_video_aac_decoder_create(char* error, size_t capacity)
{
    return create([&](const Backend& b) { return b.aac_decoder_create(error, capacity); },
                  [](const Backend& b, void* p) { b.aac_decoder_destroy(p); }, error, capacity);
}
extern "C" void nectarlink_video_aac_decoder_destroy(void* p)
{
    auto* h = static_cast<Handle*>(p);
    h->backend->aac_decoder_destroy(h->object);
    delete h;
}
extern "C" int nectarlink_video_aac_decoder_send(void* p, const uint8_t* data, size_t len, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->aac_decoder_send(h->object, data, len, error, n);
}
extern "C" int nectarlink_video_aac_decoder_receive(void* p, const int16_t** pcm, size_t* frames,
                                                    int* sample_rate, int* channels, char* error, size_t n)
{
    auto* h = static_cast<Handle*>(p);
    return h->backend->aac_decoder_receive(h->object, pcm, frames, sample_rate, channels, error, n);
}
