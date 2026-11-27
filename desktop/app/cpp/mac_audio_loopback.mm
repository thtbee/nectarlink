// SPDX-License-Identifier: GPL-3.0-or-later
// macOS system audio loopback capture via ScreenCaptureKit (SCStream).

#import <AVFoundation/AVFoundation.h>
#import <CoreAudio/CoreAudio.h>
#import <CoreGraphics/CoreGraphics.h>
#import <CoreMedia/CoreMedia.h>
#import <Foundation/Foundation.h>
#import <ScreenCaptureKit/ScreenCaptureKit.h>

#include <algorithm>
#include <atomic>
#include <cstdint>
#include <cstdio>
#include <cstring>
#include <vector>

#include <dispatch/dispatch.h>

extern "C" {

typedef void (*MacAudioCallback)(
    void *ctx,
    const float *samples,
    size_t frame_count,
    uint32_t sample_rate,
    uint32_t channels);

typedef void (*MacAudioStopCallback)(void *ctx, int32_t code, const char *msg);

} // extern "C"

namespace {

void write_err(char *buf, size_t len, const char *msg)
{
    if (!buf || len == 0)
        return;
    if (!msg) {
        buf[0] = '\0';
        return;
    }
    std::snprintf(buf, len, "%s", msg);
}

} // namespace

API_AVAILABLE(macos(13.0))
@interface NectarLinkAudioStreamDelegate : NSObject <SCStreamDelegate, SCStreamOutput>
@property (nonatomic, assign) MacAudioCallback onAudio;
@property (nonatomic, assign) MacAudioStopCallback onStop;
@property (nonatomic, assign) void *audioCtx;
@property (nonatomic, assign) void *stopCtx;
@end

@implementation NectarLinkAudioStreamDelegate {
    std::atomic<bool> _stopped;
    std::vector<float> _scratch;
}

- (void)markStopped
{
    _stopped.store(true, std::memory_order_release);
}

- (void)stream:(SCStream *)stream
    didOutputSampleBuffer:(CMSampleBufferRef)sampleBuffer
                   ofType:(SCStreamOutputType)type
{
    (void)stream;
    if (_stopped.load(std::memory_order_acquire) || type != SCStreamOutputTypeAudio)
        return;
    if (!sampleBuffer || !CMSampleBufferIsValid(sampleBuffer) || !CMSampleBufferDataIsReady(sampleBuffer))
        return;

    CMFormatDescriptionRef fmt = CMSampleBufferGetFormatDescription(sampleBuffer);
    if (!fmt)
        return;
    const AudioStreamBasicDescription *asbd = CMAudioFormatDescriptionGetStreamBasicDescription(fmt);
    if (!asbd || asbd->mFormatID != kAudioFormatLinearPCM)
        return;
    if ((asbd->mFormatFlags & kAudioFormatFlagIsFloat) == 0 || asbd->mBitsPerChannel != 32)
        return;

    const CMItemCount numFrames = CMSampleBufferGetNumSamples(sampleBuffer);
    if (numFrames <= 0)
        return;

    CMBlockBufferRef blockBuffer = nullptr;
    size_t ablSize = 0;
    OSStatus status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        sampleBuffer,
        &ablSize,
        nullptr,
        0,
        nullptr,
        nullptr,
        0,
        &blockBuffer);
    if (status != noErr || ablSize == 0) {
        if (blockBuffer)
            CFRelease(blockBuffer);
        return;
    }

    std::vector<uint8_t> ablStorage(ablSize);
    auto *abl = reinterpret_cast<AudioBufferList *>(ablStorage.data());
    status = CMSampleBufferGetAudioBufferListWithRetainedBlockBuffer(
        sampleBuffer,
        nullptr,
        abl,
        ablSize,
        nullptr,
        nullptr,
        kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment,
        &blockBuffer);
    if (status != noErr) {
        if (blockBuffer)
            CFRelease(blockBuffer);
        return;
    }

    const uint32_t channels = asbd->mChannelsPerFrame;
    const uint32_t sampleRate = static_cast<uint32_t>(asbd->mSampleRate);
    const bool nonInterleaved = (asbd->mFormatFlags & kAudioFormatFlagIsNonInterleaved) != 0;

    if (_onAudio && channels >= 1) {
        if (!nonInterleaved && abl->mNumberBuffers >= 1 && abl->mBuffers[0].mData) {
            const size_t availFrames = abl->mBuffers[0].mDataByteSize / (sizeof(float) * channels);
            const size_t frames = std::min<size_t>(static_cast<size_t>(numFrames), availFrames);
            if (frames > 0) {
                _onAudio(
                    _audioCtx,
                    static_cast<const float *>(abl->mBuffers[0].mData),
                    frames,
                    sampleRate,
                    channels);
            }
        } else if (nonInterleaved && channels >= 2 && abl->mNumberBuffers >= 2
                   && abl->mBuffers[0].mData && abl->mBuffers[1].mData) {
            const auto *left = static_cast<const float *>(abl->mBuffers[0].mData);
            const auto *right = static_cast<const float *>(abl->mBuffers[1].mData);
            const size_t availL = abl->mBuffers[0].mDataByteSize / sizeof(float);
            const size_t availR = abl->mBuffers[1].mDataByteSize / sizeof(float);
            const size_t frames = std::min<size_t>(static_cast<size_t>(numFrames), std::min(availL, availR));
            if (frames > 0) {
                _scratch.resize(frames * 2);
                for (size_t i = 0; i < frames; ++i) {
                    _scratch[i * 2] = left[i];
                    _scratch[i * 2 + 1] = right[i];
                }
                _onAudio(_audioCtx, _scratch.data(), frames, sampleRate, 2);
            }
        } else if (nonInterleaved && channels == 1 && abl->mNumberBuffers >= 1 && abl->mBuffers[0].mData) {
            const size_t avail = abl->mBuffers[0].mDataByteSize / sizeof(float);
            const size_t frames = std::min<size_t>(static_cast<size_t>(numFrames), avail);
            if (frames > 0) {
                _onAudio(
                    _audioCtx,
                    static_cast<const float *>(abl->mBuffers[0].mData),
                    frames,
                    sampleRate,
                    1);
            }
        }
    }

    if (blockBuffer)
        CFRelease(blockBuffer);
}

- (void)stream:(SCStream *)stream didStopWithError:(NSError *)error
{
    (void)stream;
    if (_stopped.exchange(true, std::memory_order_acq_rel))
        return;
    if (_onStop) {
        const int32_t code = error ? static_cast<int32_t>(error.code) : 0;
        const char *msg = error ? error.localizedDescription.UTF8String : "stream stopped";
        _onStop(_stopCtx, code, msg ? msg : "stream stopped");
    }
}

@end

namespace {

struct MacAudioLoopbackHandle {
    SCStream *stream API_AVAILABLE(macos(13.0)) = nil;
    NectarLinkAudioStreamDelegate *delegate API_AVAILABLE(macos(13.0)) = nil;
    dispatch_queue_t audioQueue = nullptr;
};

} // namespace

extern "C" {

bool mac_audio_loopback_has_permission(void)
{
    return CGPreflightScreenCaptureAccess();
}

void *mac_audio_loopback_start(
    MacAudioCallback on_audio,
    MacAudioStopCallback on_stop,
    void *audio_ctx,
    void *stop_ctx,
    int32_t *out_err_code,
    char *err_buf,
    size_t err_buf_len)
{
    if (out_err_code)
        *out_err_code = 0;

    if (@available(macOS 13.0, *)) {
        @autoreleasepool {
            __block SCShareableContent *content = nil;
            __block NSError *contentErr = nil;
            dispatch_semaphore_t contentSem = dispatch_semaphore_create(0);

            [SCShareableContent getShareableContentExcludingDesktopWindows:NO
                                                      onScreenWindowsOnly:NO
                                                        completionHandler:^(SCShareableContent *sc, NSError *err) {
                                                            content = sc;
                                                            contentErr = err;
                                                            dispatch_semaphore_signal(contentSem);
                                                        }];

            const dispatch_time_t timeout = dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC);
            if (dispatch_semaphore_wait(contentSem, timeout) != 0) {
                write_err(err_buf, err_buf_len, "timed out waiting for ScreenCaptureKit shareable content");
                return nullptr;
            }

            if (contentErr || !content) {
                const int32_t code = contentErr ? static_cast<int32_t>(contentErr.code) : -1;
                if (out_err_code)
                    *out_err_code = code;
                const char *msg = contentErr ? contentErr.localizedDescription.UTF8String
                                             : "ScreenCaptureKit shareable content unavailable";
                write_err(err_buf, err_buf_len, msg);
                return nullptr;
            }

            SCDisplay *display = content.displays.firstObject;
            if (!display) {
                write_err(err_buf, err_buf_len, "no display available for ScreenCaptureKit audio capture");
                return nullptr;
            }

            SCContentFilter *filter = [[SCContentFilter alloc] initWithDisplay:display
                                                         excludingApplications:@[]
                                                              exceptingWindows:@[]];

            SCStreamConfiguration *config = [[SCStreamConfiguration alloc] init];
            config.capturesAudio = YES;
            config.excludesCurrentProcessAudio = YES;
            config.sampleRate = 48000;
            config.channelCount = 2;
            config.width = 2;
            config.height = 2;
            config.minimumFrameInterval = CMTimeMake(3600, 1);

            auto *delegate = [[NectarLinkAudioStreamDelegate alloc] init];
            delegate.onAudio = on_audio;
            delegate.onStop = on_stop;
            delegate.audioCtx = audio_ctx;
            delegate.stopCtx = stop_ctx;

            SCStream *stream = [[SCStream alloc] initWithFilter:filter configuration:config delegate:delegate];
            dispatch_queue_t audioQueue =
                dispatch_queue_create("com.nectarlink.pc_audio_loopback", DISPATCH_QUEUE_SERIAL);

            NSError *addErr = nil;
            if (![stream addStreamOutput:delegate
                                    type:SCStreamOutputTypeAudio
                      sampleHandlerQueue:audioQueue
                                   error:&addErr]) {
                const int32_t code = addErr ? static_cast<int32_t>(addErr.code) : -1;
                if (out_err_code)
                    *out_err_code = code;
                const char *msg = addErr ? addErr.localizedDescription.UTF8String
                                         : "failed to add ScreenCaptureKit audio output";
                write_err(err_buf, err_buf_len, msg);
                return nullptr;
            }
            // Attach a no-op screen output so ScreenCaptureKit does not log dropped-frame warnings.
            NSError *screenErr = nil;
            [stream addStreamOutput:delegate
                               type:SCStreamOutputTypeScreen
                 sampleHandlerQueue:audioQueue
                              error:&screenErr];

            __block NSError *startErr = nil;
            dispatch_semaphore_t startSem = dispatch_semaphore_create(0);
            [stream startCaptureWithCompletionHandler:^(NSError *err) {
                startErr = err;
                dispatch_semaphore_signal(startSem);
            }];

            const dispatch_time_t startTimeout = dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC);
            if (dispatch_semaphore_wait(startSem, startTimeout) != 0) {
                [delegate markStopped];
                [stream stopCaptureWithCompletionHandler:^(NSError *err) { (void)err; }];
                write_err(err_buf, err_buf_len, "timed out starting ScreenCaptureKit audio stream");
                return nullptr;
            }

            if (startErr) {
                [delegate markStopped];
                const int32_t code = static_cast<int32_t>(startErr.code);
                if (out_err_code)
                    *out_err_code = code;
                const char *msg = startErr.localizedDescription.UTF8String;
                write_err(err_buf, err_buf_len, msg ? msg : "failed to start ScreenCaptureKit audio stream");
                return nullptr;
            }

            auto *handle = new MacAudioLoopbackHandle();
            handle->stream = stream;
            handle->delegate = delegate;
            handle->audioQueue = audioQueue;
            return handle;
        }
    }

    write_err(err_buf, err_buf_len, "ScreenCaptureKit audio capture requires macOS 13.0 or later");
    return nullptr;
}

void mac_audio_loopback_stop(void *raw_handle)
{
    if (!raw_handle)
        return;
    auto *handle = static_cast<MacAudioLoopbackHandle *>(raw_handle);
    if (@available(macOS 13.0, *)) {
        @autoreleasepool {
            [handle->delegate markStopped];
            dispatch_semaphore_t stopSem = dispatch_semaphore_create(0);
            [handle->stream stopCaptureWithCompletionHandler:^(NSError *err) {
                (void)err;
                dispatch_semaphore_signal(stopSem);
            }];
            const dispatch_time_t timeout = dispatch_time(DISPATCH_TIME_NOW, 2 * NSEC_PER_SEC);
            dispatch_semaphore_wait(stopSem, timeout);
            // Drain any already-queued audio callback before freeing `audio_ctx` and `stop_ctx` on the Rust side.
            if (handle->audioQueue) {
                dispatch_sync(handle->audioQueue, ^{});
            }
            handle->delegate.onAudio = nullptr;
            handle->delegate.onStop = nullptr;
            handle->delegate.audioCtx = nullptr;
            handle->delegate.stopCtx = nullptr;
        }
    }
    delete handle;
}

// Whether this app may record the microphone, as an AVAuthorizationStatus: 0
// not asked yet, 1 restricted, 2 denied, 3 allowed. When the person hasn't
// answered, it asks and waits for the answer (a couple of minutes at most).
// -1 without an NSMicrophoneUsageDescription, as when run from a terminal
// instead of the app bundle: macOS judges the terminal then, and asking
// would end the process.
int32_t mac_microphone_access(void)
{
    @autoreleasepool {
        if (![NSBundle.mainBundle objectForInfoDictionaryKey:@"NSMicrophoneUsageDescription"])
            return -1;
        AVAuthorizationStatus status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
        if (status == AVAuthorizationStatusNotDetermined) {
            dispatch_semaphore_t answered = dispatch_semaphore_create(0);
            [AVCaptureDevice requestAccessForMediaType:AVMediaTypeAudio
                                     completionHandler:^(BOOL) { dispatch_semaphore_signal(answered); }];
            dispatch_semaphore_wait(answered, dispatch_time(DISPATCH_TIME_NOW, 120 * NSEC_PER_SEC));
            status = [AVCaptureDevice authorizationStatusForMediaType:AVMediaTypeAudio];
        }
        return static_cast<int32_t>(status);
    }
}

} // extern "C"
