// SPDX-License-Identifier: GPL-3.0-or-later
// Separate public CoreMediaIO Camera Extension executable (macOS 12.3+).
// No window server, local camera, phone transport, listener socket or child process.
#import <Foundation/Foundation.h>
#import <CoreMediaIO/CoreMediaIO.h>
#import <CoreVideo/CoreVideo.h>
#import <Security/Security.h>
#include <string.h>
#include "NectarLinkCamera.h"

static NSError *nlError(NSString *message) {
    return [NSError errorWithDomain:@"com.nectarlink.camera" code:1
                           userInfo:@{NSLocalizedDescriptionKey: message}];
}
static uint64_t hostNS(void) {
    return (uint64_t)CMTimeConvertScale(CMClockGetTime(CMClockGetHostTimeClock()),
        1000000000, kCMTimeRoundingMethod_Default).value;
}
static NSDictionary *durationDictionary(void) {
    return CFBridgingRelease(CMTimeCopyAsDictionary(CMTimeMake(1, NL_CAMERA_FPS), kCFAllocatorDefault));
}

@class NLDevice;
@interface NLStream : NSObject <CMIOExtensionStreamSource> {
    NSArray<CMIOExtensionStreamFormat *> *_formats;
}
@property(nonatomic, strong) CMIOExtensionStream *stream;
@property(nonatomic, weak) NLDevice *owner;
@property(nonatomic) BOOL sink;
@property(nonatomic) NSUInteger activeFormatIndex;
@property(atomic, readonly) NSArray<CMIOExtensionStreamFormat *> *formats;
- (instancetype)initWithFormats:(NSArray<CMIOExtensionStreamFormat *> *)formats;
@end

@interface NLDevice : NSObject <CMIOExtensionDeviceSource>
@property(nonatomic, strong) CMIOExtensionDevice *device;
@property(nonatomic, strong) NLStream *source;
@property(nonatomic, strong) NLStream *sink;
@property(nonatomic, strong) CMIOExtensionClient *producer;
@property(nonatomic, strong) dispatch_queue_t queue;
@property(nonatomic, strong) dispatch_source_t timer;
@property(nonatomic) NSUInteger sourceStarts;
@property(nonatomic) BOOL sinkRunning;
@property(nonatomic) BOOL consuming;
@property(nonatomic) uint64_t generation;
@property(nonatomic) CVPixelBufferRef latest;
@property(nonatomic) uint64_t latestTime;
@property(nonatomic) CVPixelBufferRef black;
@property(nonatomic) CMVideoFormatDescriptionRef format;
@property(nonatomic) CVPixelBufferPoolRef outputPool;
- (CVPixelBufferRef)copySourceImage;
- (BOOL)selectSourceFormat:(CMIOExtensionStreamFormat *)format error:(NSError **)error;
- (instancetype)initWithQueue:(dispatch_queue_t)queue error:(NSError **)error;
- (BOOL)authorizeProducer:(CMIOExtensionClient *)client;
- (void)stopProducer;
- (void)disconnect:(CMIOExtensionClient *)client;
- (void)ensureTimer;
- (void)tick;
@end

@implementation NLStream
- (instancetype)initWithFormats:(NSArray<CMIOExtensionStreamFormat *> *)formats {
    self = [super init];
    if (self) _formats = formats;
    return self;
}
- (NSArray<CMIOExtensionStreamFormat *> *)formats { return _formats; }
- (NSSet<CMIOExtensionProperty> *)availableProperties {
    NSMutableSet *properties = [NSMutableSet setWithArray:@[
        CMIOExtensionPropertyStreamActiveFormatIndex, CMIOExtensionPropertyStreamFrameDuration,
        CMIOExtensionPropertyStreamMaxFrameDuration]];
    if (self.sink) [properties addObjectsFromArray:@[
        CMIOExtensionPropertyStreamSinkBufferQueueSize,
        CMIOExtensionPropertyStreamSinkBuffersRequiredForStartup,
        CMIOExtensionPropertyStreamSinkBufferUnderrunCount,
        CMIOExtensionPropertyStreamSinkEndOfData]];
    return properties;
}
- (CMIOExtensionStreamProperties *)streamPropertiesForProperties:(NSSet<CMIOExtensionProperty> *)requested
                                                         error:(NSError **)error {
    (void)error;
    CMIOExtensionStreamProperties *p = [[CMIOExtensionStreamProperties alloc] initWithDictionary:@{}];
    if ([requested containsObject:CMIOExtensionPropertyStreamActiveFormatIndex]) p.activeFormatIndex = @(self.activeFormatIndex);
    if ([requested containsObject:CMIOExtensionPropertyStreamFrameDuration]) p.frameDuration = durationDictionary();
    if ([requested containsObject:CMIOExtensionPropertyStreamMaxFrameDuration]) p.maxFrameDuration = durationDictionary();
    if (self.sink) {
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkBufferQueueSize]) p.sinkBufferQueueSize = @2;
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkBuffersRequiredForStartup]) p.sinkBuffersRequiredForStartup = @1;
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkBufferUnderrunCount]) p.sinkBufferUnderrunCount = @0;
        if ([requested containsObject:CMIOExtensionPropertyStreamSinkEndOfData]) p.sinkEndOfData = @(!self.owner.sinkRunning);
    }
    return p;
}
- (BOOL)setStreamProperties:(CMIOExtensionStreamProperties *)properties error:(NSError **)error {
    // Sink remains UHD; source consumers may negotiate any advertised output.
    if ((properties.activeFormatIndex && properties.activeFormatIndex.unsignedIntegerValue >= self.formats.count) ||
        (properties.frameDuration && CMTimeCompare(CMTimeMakeFromDictionary(
            (__bridge CFDictionaryRef)properties.frameDuration), CMTimeMake(1, NL_CAMERA_FPS)) != 0) ||
        (properties.maxFrameDuration && CMTimeCompare(CMTimeMakeFromDictionary(
            (__bridge CFDictionaryRef)properties.maxFrameDuration), CMTimeMake(1, NL_CAMERA_FPS)) != 0)) {
        if (error) *error = nlError(@"Select an advertised format at 30 fps.");
        return NO;
    }
    if (properties.activeFormatIndex && !self.sink) {
        NSUInteger index = properties.activeFormatIndex.unsignedIntegerValue;
        if (![self.owner selectSourceFormat:self.formats[index] error:error]) return NO;
        self.activeFormatIndex = index;
    }
    if (self.sink && properties.sinkEndOfData.boolValue) [self.owner stopProducer];
    return YES;
}
- (BOOL)authorizedToStartStreamForClient:(CMIOExtensionClient *)client {
    // Source consumers remain subject to macOS camera privacy mediation.
    return self.sink ? [self.owner authorizeProducer:client] : YES;
}
- (BOOL)startStreamAndReturnError:(NSError **)error {
    if (self.sink) {
        if (!self.owner.producer || self.owner.sinkRunning) {
            if (error) *error = nlError(@"No exclusive authorized producer.");
            return NO;
        }
        self.owner.sinkRunning = YES;
        self.owner.generation += 1;
    } else {
        self.owner.sourceStarts += 1;
    }
    [self.owner ensureTimer];
    return YES;
}
- (BOOL)stopStreamAndReturnError:(NSError **)error {
    (void)error;
    if (self.sink) [self.owner stopProducer];
    else if (self.owner.sourceStarts > 0) {
        self.owner.sourceStarts -= 1;
        if (self.owner.sourceStarts == 0) {
            if (self.owner.latest) { CVPixelBufferRelease(self.owner.latest); self.owner.latest = NULL; }
        }
    }
    return YES;
}
@end

@implementation NLDevice
- (instancetype)initWithQueue:(dispatch_queue_t)queue error:(NSError **)error {
    self = [super init];
    if (!self) return nil;
    _queue = queue;
    NSMutableArray<CMIOExtensionStreamFormat *> *formats = [NSMutableArray array];
    CMTime duration = CMTimeMake(1, NL_CAMERA_FPS);
    const int widths[] = {1280, 1920, NL_CAMERA_WIDTH};
    const int heights[] = {720, 1080, NL_CAMERA_HEIGHT};
    for (NSUInteger i = 0; i < 3; ++i) {
        CMVideoFormatDescriptionRef description = NULL;
        if (CMVideoFormatDescriptionCreate(kCFAllocatorDefault, kCVPixelFormatType_32BGRA,
            widths[i], heights[i], NULL, &description) != noErr) {
            if (error) *error = nlError(@"Could not allocate camera format.");
            return nil;
        }
        [formats addObject:[[CMIOExtensionStreamFormat alloc] initWithFormatDescription:description
            maxFrameDuration:duration minFrameDuration:duration validFrameDurations:nil]];
        CFRelease(description);
    }
    if (![self selectSourceFormat:formats[0] error:error]) return nil;
    _device = [[CMIOExtensionDevice alloc] initWithLocalizedName:@"NectarLink Camera"
        deviceID:[[NSUUID alloc] initWithUUIDString:@NL_CAMERA_DEVICE_UUID] legacyDeviceID:nil source:self];
    _source = [[NLStream alloc] initWithFormats:formats]; _source.owner = self;
    _source.stream = [[CMIOExtensionStream alloc] initWithLocalizedName:@"NectarLink Camera"
        streamID:[[NSUUID alloc] initWithUUIDString:@NL_CAMERA_SOURCE_UUID]
        direction:CMIOExtensionStreamDirectionSource clockType:CMIOExtensionStreamClockTypeHostTime source:_source];
    _sink = [[NLStream alloc] initWithFormats:@[formats.lastObject]]; _sink.owner = self; _sink.sink = YES;
    _sink.stream = [[CMIOExtensionStream alloc] initWithLocalizedName:@"NectarLink Camera Producer"
        streamID:[[NSUUID alloc] initWithUUIDString:@NL_CAMERA_SINK_UUID]
        direction:CMIOExtensionStreamDirectionSink clockType:CMIOExtensionStreamClockTypeHostTime source:_sink];
    if (![_device addStream:_source.stream error:error] || ![_device addStream:_sink.stream error:error]) return nil;
    return self;
}
- (void)dealloc {
    if (_timer) dispatch_source_cancel(_timer);
    if (_latest) CVPixelBufferRelease(_latest);
    if (_black) CVPixelBufferRelease(_black);
    if (_format) CFRelease(_format);
    if (_outputPool) CVPixelBufferPoolRelease(_outputPool);
}
// Called on the provider's serialized queue. Allocate before replacing the active output.
- (BOOL)selectSourceFormat:(CMIOExtensionStreamFormat *)format error:(NSError **)error {
    CMVideoDimensions size = CMVideoFormatDescriptionGetDimensions(format.formatDescription);
    NSDictionary *attrs = @{(id)kCVPixelBufferIOSurfacePropertiesKey: @{},
        (id)kCVPixelBufferWidthKey: @(size.width), (id)kCVPixelBufferHeightKey: @(size.height),
        (id)kCVPixelBufferPixelFormatTypeKey: @(kCVPixelFormatType_32BGRA)};
    CVPixelBufferRef black = NULL;
    CVPixelBufferPoolRef output = NULL;
    if (CVPixelBufferCreate(kCFAllocatorDefault, size.width, size.height, kCVPixelFormatType_32BGRA,
        (__bridge CFDictionaryRef)attrs, &black) != kCVReturnSuccess ||
        CVPixelBufferPoolCreate(kCFAllocatorDefault, NULL, (__bridge CFDictionaryRef)attrs, &output) != kCVReturnSuccess ||
        CVPixelBufferLockBaseAddress(black, 0) != kCVReturnSuccess) {
        if (black) CVPixelBufferRelease(black);
        if (output) CVPixelBufferPoolRelease(output);
        if (error) *error = nlError(@"Could not allocate selected camera output.");
        return NO;
    }
    uint8_t *pixels = CVPixelBufferGetBaseAddress(black);
    size_t pitch = CVPixelBufferGetBytesPerRow(black);
    memset(pixels, 0, pitch * size.height);
    for (int y = 0; y < size.height; ++y)
        for (int x = 0; x < size.width; ++x) pixels[y * pitch + x * 4 + 3] = 255;
    CVPixelBufferUnlockBaseAddress(black, 0);
    if (_black) CVPixelBufferRelease(_black);
    if (_outputPool) CVPixelBufferPoolRelease(_outputPool);
    if (_format) CFRelease(_format);
    _black = black; _outputPool = output;
    _format = (CMVideoFormatDescriptionRef)CFRetain(format.formatDescription);
    return YES;
}
- (CVPixelBufferRef)copySourceImage {
    if (!_latest) return CVPixelBufferRetain(_black);
    // UHD sink input is aspect-fitted by the host; only scale to the consumer's selected size.
    if (CVPixelBufferGetWidth(_latest) == CVPixelBufferGetWidth(_black) &&
        CVPixelBufferGetHeight(_latest) == CVPixelBufferGetHeight(_black)) return CVPixelBufferRetain(_latest);
    CVPixelBufferRef output = NULL;
    NSDictionary *limit = @{(id)kCVPixelBufferPoolAllocationThresholdKey: @4};
    if (CVPixelBufferPoolCreatePixelBufferWithAuxAttributes(kCFAllocatorDefault, _outputPool,
        (__bridge CFDictionaryRef)limit, &output) != kCVReturnSuccess) return NULL;
    if (CVPixelBufferLockBaseAddress(_latest, kCVPixelBufferLock_ReadOnly) != kCVReturnSuccess) {
        CVPixelBufferRelease(output); return NULL;
    }
    if (CVPixelBufferLockBaseAddress(output, 0) != kCVReturnSuccess) {
        CVPixelBufferUnlockBaseAddress(_latest, kCVPixelBufferLock_ReadOnly);
        CVPixelBufferRelease(output); return NULL;
    }
    size_t width = CVPixelBufferGetWidth(output), height = CVPixelBufferGetHeight(output);
    size_t sourceWidth = CVPixelBufferGetWidth(_latest), sourceHeight = CVPixelBufferGetHeight(_latest);
    size_t sourcePitch = CVPixelBufferGetBytesPerRow(_latest), pitch = CVPixelBufferGetBytesPerRow(output);
    const uint8_t *source = CVPixelBufferGetBaseAddress(_latest);
    uint8_t *destination = CVPixelBufferGetBaseAddress(output);
    for (size_t y = 0; y < height; ++y) {
        const uint32_t *row = (const uint32_t *)(source + (y * sourceHeight / height) * sourcePitch);
        uint32_t *out = (uint32_t *)(destination + y * pitch);
        for (size_t x = 0; x < width; ++x) out[x] = row[x * sourceWidth / width];
    }
    CVPixelBufferUnlockBaseAddress(output, 0);
    CVPixelBufferUnlockBaseAddress(_latest, kCVPixelBufferLock_ReadOnly);
    return output;
}
- (NSSet<CMIOExtensionProperty> *)availableProperties {
    return [NSSet setWithObject:CMIOExtensionPropertyDeviceModel];
}
- (CMIOExtensionDeviceProperties *)devicePropertiesForProperties:(NSSet<CMIOExtensionProperty> *)requested
                                                         error:(NSError **)error {
    (void)error;
    CMIOExtensionDeviceProperties *p = [[CMIOExtensionDeviceProperties alloc] initWithDictionary:@{}];
    if ([requested containsObject:CMIOExtensionPropertyDeviceModel]) p.model = @"NectarLink Phone Camera";
    return p;
}
- (BOOL)setDeviceProperties:(CMIOExtensionDeviceProperties *)properties error:(NSError **)error {
    (void)properties;
    if (error) *error = nlError(@"Camera device properties are read-only.");
    return NO;
}
- (BOOL)authorizeProducer:(CMIOExtensionClient *)client {
    if (_producer) return [_producer.clientID isEqual:client.clientID] && !_sinkRunning;
    // Never accept any app just because it knows the public device UUID.
    // Require the exact host identifier and this extension's own signing team.
    SecCodeRef ownCode = NULL;
    CFDictionaryRef ownInfo = NULL;
    if (SecCodeCopySelf(kSecCSDefaultFlags, &ownCode) != errSecSuccess) return NO;
    SecStaticCodeRef ownStatic = NULL;
    OSStatus ownStatus = SecCodeCopyStaticCode(ownCode, kSecCSDefaultFlags, &ownStatic);
    if (ownStatus == errSecSuccess)
        ownStatus = SecCodeCopySigningInformation(ownStatic, kSecCSSigningInformation, &ownInfo);
    if (ownStatic) CFRelease(ownStatic);
    CFRelease(ownCode);
    NSString *team = ownInfo ? ((__bridge NSDictionary *)ownInfo)[(__bridge id)kSecCodeInfoTeamIdentifier] : nil;
    if (ownStatus != errSecSuccess || ![team isKindOfClass:NSString.class] ||
        team.length != 10 || [team rangeOfCharacterFromSet:
        [[NSCharacterSet characterSetWithCharactersInString:@"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"] invertedSet]].location != NSNotFound) {
        if (ownInfo) CFRelease(ownInfo);
        return NO;
    }
    NSString *rule = [NSString stringWithFormat:
        @"anchor apple generic and identifier \"%s\" and certificate leaf[subject.OU] = \"%@\"", NL_CAMERA_HOST_ID, team];
    CFRelease(ownInfo);
    SecRequirementRef requirement = NULL;
    SecCodeRef code = NULL;
    OSStatus status = SecRequirementCreateWithString((__bridge CFStringRef)rule,
        kSecCSDefaultFlags, &requirement);
    NSDictionary *guest = @{(id)kSecGuestAttributePid: @(client.pid)};
    if (status == errSecSuccess) status = SecCodeCopyGuestWithAttributes(NULL,
        (__bridge CFDictionaryRef)guest, kSecCSDefaultFlags, &code);
    if (status == errSecSuccess) status = SecCodeCheckValidity(code, kSecCSStrictValidate, requirement);
    if (code) CFRelease(code);
    if (requirement) CFRelease(requirement);
    if (status != errSecSuccess) return NO;
    _producer = client;
    return YES;
}
- (void)stopProducer {
    _sinkRunning = NO;
    _generation += 1;
    _consuming = NO;
    _producer = nil;
    _latestTime = 0;
    if (_latest) { CVPixelBufferRelease(_latest); _latest = NULL; }
    // Existing source clients receive black on the next 30 Hz tick.
}
- (void)disconnect:(CMIOExtensionClient *)client {
    if ([_producer.clientID isEqual:client.clientID]) [self stopProducer];
}
- (void)ensureTimer {
    if (_timer) return;
    _timer = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, _queue);
    dispatch_source_set_timer(_timer, DISPATCH_TIME_NOW, NSEC_PER_SEC / NL_CAMERA_FPS, NSEC_PER_MSEC);
    __weak NLDevice *weakSelf = self;
    dispatch_source_set_event_handler(_timer, ^{ [weakSelf tick]; });
    dispatch_resume(_timer);
}
- (void)tick {
    if (!_sinkRunning && !_sourceStarts) {
        dispatch_source_cancel(_timer); _timer = nil;
        return;
    }
    uint64_t now = hostNS();
    if (_latest && (now < _latestTime || now - _latestTime > NL_CAMERA_MAX_AGE_NS)) {
        CVPixelBufferRelease(_latest); _latest = NULL; _latestTime = 0;
    }
    if (_sinkRunning && !_consuming && _producer) {
        _consuming = YES;
        uint64_t generation = _generation;
        NSUUID *clientID = _producer.clientID;
        __weak NLDevice *weakSelf = self;
        [_sink.stream consumeSampleBufferFromClient:_producer completionHandler:
            ^(CMSampleBufferRef sample, uint64_t sequence, CMIOExtensionStreamDiscontinuityFlags discontinuity,
              BOOL hasMore, NSError *error) {
            (void)discontinuity; (void)hasMore;
            // The API owns this borrowed buffer during its completion; retain
            // across the hop to our one state/camera queue only.
            if (sample) CFRetain(sample);
            NLDevice *owner = weakSelf;
            if (!owner) { if (sample) CFRelease(sample); return; }
            dispatch_async(owner.queue, ^{
                if (owner.generation != generation || !owner.sinkRunning ||
                    ![owner.producer.clientID isEqual:clientID]) {
                    if (sample) CFRelease(sample);
                    return;
                }
                owner.consuming = NO;
                if (error) { [owner stopProducer]; if (sample) CFRelease(sample); return; }
                if (sample) {
                    CVPixelBufferRef image = CMSampleBufferGetImageBuffer(sample);
                    CMTime pts = CMSampleBufferGetPresentationTimeStamp(sample);
                    uint64_t now = hostNS();
                    int64_t ptsNS = CMTIME_IS_NUMERIC(pts) ? CMTimeConvertScale(pts,
                        1000000000, kCMTimeRoundingMethod_Default).value : -1;
                    BOOL valid = image && CVPixelBufferGetPixelFormatType(image) == kCVPixelFormatType_32BGRA &&
                        CVPixelBufferGetWidth(image) == NL_CAMERA_WIDTH && CVPixelBufferGetHeight(image) == NL_CAMERA_HEIGHT &&
                        ptsNS >= 0 && (uint64_t)ptsNS <= now && now - (uint64_t)ptsNS <= NL_CAMERA_MAX_AGE_NS;
                    if (valid && owner.sourceStarts) {
                        CVPixelBufferRetain(image);
                        if (owner.latest) CVPixelBufferRelease(owner.latest);
                        owner.latest = image; owner.latestTime = (uint64_t)ptsNS;
                    }
                    [owner.sink.stream notifyScheduledOutputChanged:[[CMIOExtensionScheduledOutput alloc]
                        initWithSequenceNumber:sequence hostTimeInNanoseconds:now]];
                    CFRelease(sample);
                }
            });
        }];
    }
    if (!_sourceStarts) return;
    CMSampleTimingInfo timing = {CMTimeMake(1, NL_CAMERA_FPS), CMTimeMake((int64_t)now, 1000000000), kCMTimeInvalid};
    CMSampleBufferRef sample = NULL;
    CVPixelBufferRef image = [self copySourceImage];
    if (!image) return;
    if (CMSampleBufferCreateReadyWithImageBuffer(kCFAllocatorDefault, image,
        _format, &timing, &sample) == noErr && sample) {
        [_source.stream sendSampleBuffer:sample discontinuity:0 hostTimeInNanoseconds:now];
        CFRelease(sample);
    }
    CVPixelBufferRelease(image);
}
@end

@interface NLProvider : NSObject <CMIOExtensionProviderSource>
@property(nonatomic, strong) CMIOExtensionProvider *provider;
@property(nonatomic, strong) NLDevice *camera;
@end
@implementation NLProvider
- (instancetype)init {
    self = [super init];
    if (!self) return nil;
    dispatch_queue_t queue = dispatch_queue_create("com.nectarlink.camera.frames", DISPATCH_QUEUE_SERIAL);
    _provider = [[CMIOExtensionProvider alloc] initWithSource:self clientQueue:queue];
    NSError *error = nil;
    _camera = [[NLDevice alloc] initWithQueue:queue error:&error];
    if (!_camera || ![_provider addDevice:_camera.device error:&error]) return nil;
    return self;
}
- (NSSet<CMIOExtensionProperty> *)availableProperties {
    return [NSSet setWithArray:@[CMIOExtensionPropertyProviderName, CMIOExtensionPropertyProviderManufacturer]];
}
- (CMIOExtensionProviderProperties *)providerPropertiesForProperties:(NSSet<CMIOExtensionProperty> *)requested
                                                             error:(NSError **)error {
    (void)error;
    CMIOExtensionProviderProperties *p = [[CMIOExtensionProviderProperties alloc] initWithDictionary:@{}];
    if ([requested containsObject:CMIOExtensionPropertyProviderName]) p.name = @"NectarLink Camera";
    if ([requested containsObject:CMIOExtensionPropertyProviderManufacturer]) p.manufacturer = @"NectarLink";
    return p;
}
- (BOOL)setProviderProperties:(CMIOExtensionProviderProperties *)properties error:(NSError **)error {
    (void)properties;
    if (error) *error = nlError(@"Provider properties are read-only.");
    return NO;
}
- (BOOL)connectClient:(CMIOExtensionClient *)client error:(NSError **)error {
    (void)client; (void)error; return YES;
}
- (void)disconnectClient:(CMIOExtensionClient *)client { [_camera disconnect:client]; }
@end

int main(void) {
    @autoreleasepool {
        __attribute__((objc_precise_lifetime)) NLProvider *source = [NLProvider new];
        if (!source) return 1;
        [CMIOExtensionProvider startServiceWithProvider:source.provider];
        CFRunLoopRun();
    }
    return 0;
}
