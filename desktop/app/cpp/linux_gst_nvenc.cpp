// SPDX-License-Identifier: GPL-3.0-or-later
// Runs only in the PC mirror worker process. GStreamer is optional at runtime.
#include <gst/app/gstappsink.h>
#include <gst/video/video-event.h>
#include <dlfcn.h>
#include <poll.h>
#include <unistd.h>
#include <chrono>
#include <cstdint>
#include <cstdio>
#include <string>

namespace {
struct Gst {
#define FN(name) decltype(&::name) name = reinterpret_cast<decltype(&::name)>(dlsym(RTLD_DEFAULT, #name))
    FN(gst_init);
    FN(gst_parse_launch);
    FN(gst_bin_get_by_name);
    FN(gst_element_get_static_pad);
    FN(gst_pad_add_probe);
    FN(gst_mini_object_make_writable);
    FN(gst_element_set_state);
    FN(gst_app_sink_try_pull_sample);
    FN(gst_sample_get_buffer);
    FN(gst_buffer_map);
    FN(gst_buffer_unmap);
    FN(gst_mini_object_unref);
    FN(gst_video_event_new_upstream_force_key_unit);
    FN(gst_pad_send_event);
    FN(g_object_set);
#undef FN

    bool ready() const {
        return gst_init && gst_parse_launch && gst_bin_get_by_name && gst_element_get_static_pad
            && gst_pad_add_probe && gst_mini_object_make_writable && gst_element_set_state
            && gst_app_sink_try_pull_sample && gst_sample_get_buffer && gst_buffer_map && gst_buffer_unmap
            && gst_mini_object_unref && gst_video_event_new_upstream_force_key_unit && gst_pad_send_event
            && g_object_set;
    }
};

struct Pace {
    Gst& gst;
    uint64_t period;
    uint64_t start = 0;
    uint64_t due = 0;
};

GstPadProbeReturn pace(GstPad*, GstPadProbeInfo* info, gpointer opaque) {
    auto& clock = *static_cast<Pace*>(opaque);
    const auto now = static_cast<uint64_t>(std::chrono::duration_cast<std::chrono::nanoseconds>(
        std::chrono::steady_clock::now().time_since_epoch()).count());
    if (now < clock.due) return GST_PAD_PROBE_DROP;
    if (clock.start == 0) clock.start = clock.due = now;
    clock.due += ((now - clock.due) / clock.period + 1) * clock.period;
    auto* buffer = reinterpret_cast<GstBuffer*>(clock.gst.gst_mini_object_make_writable(
        GST_MINI_OBJECT_CAST(info->data)));
    info->data = buffer;
    if (!buffer) return GST_PAD_PROBE_DROP;
    GST_BUFFER_PTS(buffer) = now - clock.start;
    GST_BUFFER_DTS(buffer) = GST_CLOCK_TIME_NONE;
    GST_BUFFER_DURATION(buffer) = clock.period;
    return GST_PAD_PROBE_OK;
}
} // namespace

extern "C" int nectarlink_run_nvenc_worker(int fd, unsigned node, unsigned w, unsigned h,
    unsigned fps, unsigned bitrate) {
    for (auto lib : {"libgstreamer-1.0.so.0", "libgstapp-1.0.so.0", "libgstvideo-1.0.so.0"}) {
        if (!dlopen(lib, RTLD_NOW | RTLD_GLOBAL)) {
            std::fprintf(stderr, "%s\n", dlerror());
            return 1;
        }
    }
    Gst gst;
    if (!gst.ready()) return 1;
    gst.gst_init(nullptr, nullptr);
    // A separate CUDA surface releases the imported PipeWire buffer before NVENC.
    constexpr auto rgba = "video/x-raw(memory:GLMemory),format=RGBA";
    const auto desc = "pipewiresrc name=source do-timestamp=true keepalive-time=1000 fd=" + std::to_string(fd)
        + " path=" + std::to_string(node) + " ! video/x-raw(memory:DMABuf),format=DMA_DRM ! glupload ! glcolorconvert ! "
        + rgba + " ! glcolorscale ! " + rgba + ",width=" + std::to_string(w) + ",height=" + std::to_string(h)
        + " ! capssetter caps=video/x-raw,framerate=" + std::to_string(fps) + "/1 ! cudaupload ! "
        "video/x-raw(memory:CUDAMemory),format=RGBA,framerate=" + std::to_string(fps)
        + "/1 ! nvh265enc name=encoder preset=p1 tune=ultra-low-latency rc-mode=cbr bframes=0 "
        "zerolatency=true repeat-sequence-header=true bitrate=" + std::to_string(bitrate / 1000)
        + " gop-size=" + std::to_string(fps * 2)
        + " ! video/x-h265,profile=main,stream-format=byte-stream,alignment=au ! "
          "appsink name=sink sync=false max-buffers=4";
    GError* error = nullptr;
    auto* pipe = gst.gst_parse_launch(desc.c_str(), &error);
    if (!pipe || error) {
        if (error) std::fprintf(stderr, "%s\n", error->message);
        return 1;
    }
    auto* source = gst.gst_bin_get_by_name(reinterpret_cast<GstBin*>(pipe), "source");
    auto* encoder = gst.gst_bin_get_by_name(reinterpret_cast<GstBin*>(pipe), "encoder");
    auto* sink = gst.gst_bin_get_by_name(reinterpret_cast<GstBin*>(pipe), "sink");
    auto* source_pad = gst.gst_element_get_static_pad(source, "src");
    auto* encoder_pad = gst.gst_element_get_static_pad(encoder, "src");
    Pace clock{gst, static_cast<uint64_t>(GST_SECOND / fps)};
    gst.gst_pad_add_probe(source_pad, GST_PAD_PROBE_TYPE_BUFFER, pace, &clock, nullptr);
    if (gst.gst_element_set_state(pipe, GST_STATE_PLAYING) == GST_STATE_CHANGE_FAILURE) _exit(1);
    // The parent bounds startup/stalls and kills this process even if a driver
    // hangs in teardown. EOF on stdin asks a healthy worker to exit.
    for (;;) {
        pollfd control{STDIN_FILENO, POLLIN, 0};
        if (poll(&control, 1, 0) > 0) {
            uint32_t command;
            if (read(STDIN_FILENO, &command, sizeof(command)) != sizeof(command)) _exit(0);
            if (command == 0) {
                gst.gst_pad_send_event(encoder_pad,
                    gst.gst_video_event_new_upstream_force_key_unit(GST_CLOCK_TIME_NONE, TRUE, 0));
            } else {
                gst.g_object_set(encoder, "bitrate", command / 1000, nullptr);
            }
        }
        auto* sample = gst.gst_app_sink_try_pull_sample(reinterpret_cast<GstAppSink*>(sink), 20 * GST_MSECOND);
        if (!sample) continue;
        auto* buffer = gst.gst_sample_get_buffer(sample);
        GstMapInfo map{};
        if (!buffer || !gst.gst_buffer_map(buffer, &map, GST_MAP_READ)) _exit(1);
        if (map.size == 0 || map.size > 8 * 1024 * 1024) _exit(1);
        const uint32_t size = static_cast<uint32_t>(map.size);
        const uint8_t key = !GST_BUFFER_FLAG_IS_SET(buffer, GST_BUFFER_FLAG_DELTA_UNIT);
        const uint64_t pts = GST_BUFFER_PTS(buffer);
        if (pts == GST_CLOCK_TIME_NONE) _exit(1);
        if (std::fwrite(&size, sizeof(size), 1, stdout) != 1
            || std::fwrite(&key, 1, 1, stdout) != 1
            || std::fwrite(&pts, sizeof(pts), 1, stdout) != 1
            || std::fwrite(map.data, 1, size, stdout) != size || std::fflush(stdout) != 0) _exit(1);
        gst.gst_buffer_unmap(buffer, &map);
        gst.gst_mini_object_unref(reinterpret_cast<GstMiniObject*>(sample));
    }
}
