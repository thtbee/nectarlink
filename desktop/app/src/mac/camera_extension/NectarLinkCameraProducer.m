// SPDX-License-Identifier: GPL-3.0-or-later
// Compiles into the existing Rust/Qt application, not into a replacement app.
#import <Foundation/Foundation.h>
#import <CoreMediaIO/CoreMediaIO.h>
#import <CoreVideo/CoreVideo.h>
#import <Accelerate/Accelerate.h>
#include <stdlib.h>
#include <string.h>
#include <math.h>
#include "NectarLinkCamera.h"

struct NLCameraProducer {
    CMIODeviceID device;
    CMIOStreamID sink;
    CMSimpleQueueRef queue;
    CVPixelBufferPoolRef pool;
    CMVideoFormatDescriptionRef format;
    int64_t lastPTS;
    uint32_t width, height;
    BOOL started;
};

static CMIOObjectPropertyAddress address(CMIOObjectPropertySelector selector,
                                         CMIOObjectPropertyScope scope) {
    return (CMIOObjectPropertyAddress){selector, scope, kCMIOObjectPropertyElementMain};
}

static BOOL copyIDs(CMIOObjectID object, CMIOObjectPropertyAddress a,
                    UInt32 **out, UInt32 *count) {
    UInt32 size = 0;
    if (CMIOObjectGetPropertyDataSize(object, &a, 0, NULL, &size) != noErr ||
        size == 0 || size % sizeof(UInt32) != 0 || size > 65536) return NO;
    UInt32 *ids = calloc(1, size);
    if (!ids) return NO;
    UInt32 used = 0;
    if (CMIOObjectGetPropertyData(object, &a, 0, NULL, size, &used, ids) != noErr ||
        used > size || used % sizeof(UInt32) != 0) { free(ids); return NO; }
    *out = ids; *count = used / sizeof(UInt32); return YES;
}

static BOOL deviceUIDMatches(CMIODeviceID device) {
    CMIOObjectPropertyAddress a = address(kCMIODevicePropertyDeviceUID, kCMIOObjectPropertyScopeGlobal);
    CFStringRef uid = NULL;
    UInt32 used = 0;
    OSStatus status = CMIOObjectGetPropertyData(device, &a, 0, NULL, sizeof(uid), &used, &uid);
    BOOL matches = status == noErr && used == sizeof(uid) && uid &&
        CFGetTypeID(uid) == CFStringGetTypeID() &&
        CFEqual(uid, CFSTR(NL_CAMERA_DEVICE_UUID));
    if (uid) CFRelease(uid);
    return matches;
}

static CFTypeRef copyProperty(CMIOObjectID object, CMIOObjectPropertySelector selector,
                              CFTypeID expectedType);

// Require the exact source/sink formats, not only the public device UUID.
static BOOL nativeFormatMatches(CMFormatDescriptionRef format, NSUInteger index) {
    if (!format || CFGetTypeID(format) != CMFormatDescriptionGetTypeID() ||
        CMFormatDescriptionGetMediaType(format) != kCMMediaType_Video ||
        CMFormatDescriptionGetMediaSubType(format) != kCVPixelFormatType_32BGRA) return NO;
    const int widths[] = {1280, 1920, NL_CAMERA_WIDTH};
    const int heights[] = {720, 1080, NL_CAMERA_HEIGHT};
    CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(format);
    return index < 3 && size.width == widths[index] && size.height == heights[index];
}
static BOOL nativeStreamMatches(CMIOStreamID stream, BOOL sink) {
    CFArrayRef formats = (CFArrayRef)copyProperty(stream, kCMIOStreamPropertyFormatDescriptions,
                                                 CFArrayGetTypeID());
    if (!formats) return NO;
    BOOL valid = CFArrayGetCount(formats) == (sink ? 1 : 3);
    for (CFIndex i = 0; valid && i < CFArrayGetCount(formats); ++i)
        valid = nativeFormatMatches((CMFormatDescriptionRef)CFArrayGetValueAtIndex(formats, i), sink ? 2 : i);
    CFRelease(formats);
    CMFormatDescriptionRef current = (CMFormatDescriptionRef)copyProperty(stream,
        kCMIOStreamPropertyFormatDescription, CMFormatDescriptionGetTypeID());
    BOOL currentValid = sink ? nativeFormatMatches(current, 2) :
        (nativeFormatMatches(current, 0) || nativeFormatMatches(current, 1) || nativeFormatMatches(current, 2));
    if (current) CFRelease(current);
    return valid && currentValid;
}

int32_t nl_camera_is_registered(void) {
    UInt32 *devices = NULL, count = 0, matches = 0;
    CMIODeviceID device = 0;
    if (!copyIDs(kCMIOObjectSystemObject,
        address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal), &devices, &count)) return 0;
    for (UInt32 i = 0; i < count; ++i) if (deviceUIDMatches(devices[i])) { device = devices[i]; ++matches; }
    free(devices);
    if (matches != 1) return 0;
    UInt32 *streams = NULL, streamCount = 0;
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeInput),
        &streams, &streamCount)) return 0;
    BOOL sourceValid = streamCount == 1 && nativeStreamMatches(streams[0], NO);
    free(streams);
    if (!sourceValid) return 0;
    streams = NULL; streamCount = 0;
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeOutput),
        &streams, &streamCount)) return 0;
    BOOL sinkValid = streamCount == 1 && nativeStreamMatches(streams[0], YES);
    free(streams);
    return sinkValid ? 1 : 0;
}

int32_t nl_camera_open(NLCameraProducer **out) {
    if (!out) return NL_CAMERA_INVALID;
    *out = NULL;
    if (nl_camera_is_registered() != 1) return NL_CAMERA_NOT_INSTALLED;
    UInt32 *devices = NULL, count = 0;
    if (!copyIDs(kCMIOObjectSystemObject,
                 address(kCMIOHardwarePropertyDevices, kCMIOObjectPropertyScopeGlobal),
                 &devices, &count)) return NL_CAMERA_NOT_INSTALLED;
    CMIODeviceID device = 0;
    for (UInt32 i = 0; i < count; ++i) {
        if (deviceUIDMatches(devices[i])) { device = devices[i]; break; }
    }
    free(devices);
    if (!device) return NL_CAMERA_NOT_INSTALLED;
    UInt32 *streams = NULL, streamCount = 0;
    // A CMIO sink is an output stream from the producer application's view.
    if (!copyIDs(device, address(kCMIODevicePropertyStreams, kCMIODevicePropertyScopeOutput),
                 &streams, &streamCount)) return NL_CAMERA_NOT_INSTALLED;
    // This first extension publishes exactly one output stream. Fail closed
    // rather than accidentally writing to an unrelated/ambiguous stream.
    CMIOStreamID sink = streamCount == 1 ? streams[0] : 0;
    free(streams);
    if (!sink) return NL_CAMERA_NOT_INSTALLED;
    NLCameraProducer *p = calloc(1, sizeof(*p));
    if (!p) return NL_CAMERA_OS_ERROR;
    p->device = device; p->sink = sink; p->lastPTS = -1;
    p->width = NL_CAMERA_WIDTH; p->height = NL_CAMERA_HEIGHT;
    NSDictionary *attrs = @{
        (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA),
        (id)kCVPixelBufferWidthKey: @(NL_CAMERA_WIDTH),
        (id)kCVPixelBufferHeightKey: @(NL_CAMERA_HEIGHT),
        (id)kCVPixelBufferIOSurfacePropertiesKey: @{}
    };
    OSStatus status = CVPixelBufferPoolCreate(kCFAllocatorDefault, NULL,
                                              (__bridge CFDictionaryRef)attrs, &p->pool);
    if (status == noErr) status = CMVideoFormatDescriptionCreate(kCFAllocatorDefault,
        kCVPixelFormatType_32BGRA, NL_CAMERA_WIDTH, NL_CAMERA_HEIGHT, NULL, &p->format);
    if (status == noErr) status = CMIOStreamCopyBufferQueue(sink, NULL, NULL, &p->queue);
    if (status == noErr && p->queue) status = CMIODeviceStartStream(device, sink);
    else if (status == noErr) status = -1;
    if (status != noErr) {
        if (p->queue) CFRelease(p->queue);
        if (p->format) CFRelease(p->format);
        if (p->pool) CFRelease(p->pool);
        free(p); return NL_CAMERA_OS_ERROR;
    }
    p->started = YES;
    *out = p; return NL_CAMERA_OK;
}

static CFTypeRef copyProperty(CMIOObjectID object, CMIOObjectPropertySelector selector,
                              CFTypeID expectedType) {
    CMIOObjectPropertyAddress a = address(selector, kCMIOObjectPropertyScopeGlobal);
    CFTypeRef value = NULL;
    UInt32 used = 0;
    OSStatus status = CMIOObjectGetPropertyData(object, &a, 0, NULL, sizeof(value), &used, &value);
    if (status != noErr || used != sizeof(value) || !value || CFGetTypeID(value) != expectedType) {
        if (value) CFRelease(value);
        return NULL;
    }
    return value;
}

static BOOL producerTargetCurrent(NLCameraProducer *p) {
    return deviceUIDMatches(p->device) && nativeStreamMatches(p->sink, YES);
}
static int32_t producerDropResult(NLCameraProducer *p) {
    return producerTargetCurrent(p) ? NL_CAMERA_DROPPED : NL_CAMERA_NOT_INSTALLED;
}

int32_t nl_camera_push_bgrx(NLCameraProducer *p, const uint8_t *bytes,
                           size_t length, uint32_t width, uint32_t height) {
    if (!p || !p->started || !bytes || !width || !height || width > 8192 || height > 8192 ||
        length != (size_t)width * height * 4) return NL_CAMERA_INVALID;
    if (!producerTargetCurrent(p)) return NL_CAMERA_NOT_INSTALLED;
    // Drop before allocating/converting when the extension has not consumed.
    // Keep at most two in-flight images; never accumulate seconds of latency.
    if (CMSimpleQueueGetCount(p->queue) >= MIN(2, CMSimpleQueueGetCapacity(p->queue)))
        return NL_CAMERA_DROPPED;
    CMTime began = CMClockGetTime(CMClockGetHostTimeClock());
    CVPixelBufferRef pixel = NULL;
    NSDictionary *limit = @{(id)kCVPixelBufferPoolAllocationThresholdKey: @4};
    CVReturn cv = CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(kCFAllocatorDefault,
        p->pool, (__bridge CFDictionaryRef)limit, &pixel);
    if (cv == kCVReturnWouldExceedAllocationThreshold) return producerDropResult(p);
    if (cv != kCVReturnSuccess || !pixel) {
        if (pixel) CFRelease(pixel);
        return NL_CAMERA_OS_ERROR;
    }
    if (CVPixelBufferLockBaseAddress(pixel, 0) != kCVReturnSuccess) {
        CFRelease(pixel); return NL_CAMERA_OS_ERROR;
    }
    size_t pitch = CVPixelBufferGetBytesPerRow(pixel);
    uint8_t *base = CVPixelBufferGetBaseAddress(pixel);
    if (!base || pitch < (size_t)p->width * 4) {
        CVPixelBufferUnlockBaseAddress(pixel, 0); CFRelease(pixel); return NL_CAMERA_OS_ERROR;
    }
    memset(base, 0, pitch * p->height);
    // Preserve portrait and landscape aspect ratio; all clients see one stable format.
    double scale = fmin((double)p->width / width, (double)p->height / height);
    size_t w = MAX(1, (size_t)(width * scale)), h = MAX(1, (size_t)(height * scale));
    size_t x = (p->width - w) / 2, y = (p->height - h) / 2;
    vImage_Buffer source = {(void *)bytes, height, width, (size_t)width * 4};
    vImage_Buffer target = {base + y * pitch + x * 4, h, w, pitch};
    vImage_Error imageStatus = vImageScale_ARGB8888(&source, &target, NULL, kvImageHighQualityResampling);
    // BGRX and BGRA share channel layout, but X is not a valid alpha value.
    for (size_t row = 0; row < p->height; ++row)
        for (size_t col = 0; col < p->width; ++col) base[row * pitch + col * 4 + 3] = 255;
    CVPixelBufferUnlockBaseAddress(pixel, 0);
    if (imageStatus != kvImageNoError) { CFRelease(pixel); return NL_CAMERA_OS_ERROR; }
    CMTime host = CMClockGetTime(CMClockGetHostTimeClock());
    if (!CMTIME_IS_NUMERIC(began) || !CMTIME_IS_NUMERIC(host) ||
        CMTimeCompare(CMTimeSubtract(host, began), CMTimeMake(NL_CAMERA_MAX_AGE_NS, 1000000000)) > 0) {
        CFRelease(pixel); return producerDropResult(p);
    }
    int64_t ns = CMTimeConvertScale(host, 1000000000, kCMTimeRoundingMethod_Default).value;
    if (ns <= p->lastPTS) ns = p->lastPTS + 1;
    p->lastPTS = ns;
    CMSampleTimingInfo timing = {CMTimeMake(1, NL_CAMERA_FPS), CMTimeMake(ns, 1000000000), kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    OSStatus status = CMSampleBufferCreateReadyWithImageBuffer(kCFAllocatorDefault, pixel,
        p->format, &timing, &sample);
    CFRelease(pixel);
    if (status != noErr || !sample) {
        if (sample) CFRelease(sample);
        return NL_CAMERA_OS_ERROR;
    }
    // The CMIO sink consumes the +1 sample reference on successful enqueue.
    // A CMSimpleQueue itself does not retain elements.
    if (!producerTargetCurrent(p)) { CFRelease(sample); return NL_CAMERA_NOT_INSTALLED; }
    status = CMSimpleQueueEnqueue(p->queue, sample);
    if (status != noErr) CFRelease(sample);
    return status == noErr ? NL_CAMERA_OK : producerDropResult(p);
}

void nl_camera_close(NLCameraProducer *p) {
    if (!p) return;
    p->started = NO;
    CMIODeviceStopStream(p->device, p->sink);
    // Do not dequeue/reset: CMIO is the queue's sole consumer.
    CFRelease(p->queue); CFRelease(p->format); CFRelease(p->pool);
    free(p);
}
