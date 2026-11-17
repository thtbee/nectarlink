// SPDX-License-Identifier: GPL-3.0-or-later
//
// CameraProvider.swift — CoreMediaIO Camera Extension (`CMIOExtensionProviderSource`)
// exposing "NectarLink Webcam" to macOS video apps (Zoom, Teams, Meet, FaceTime, Photo Booth).

import CoreMedia
import CoreMediaIO
import CoreVideo
import Foundation

private let kDeviceUUID = UUID(uuidString: "8E6C3B74-5D4A-4B9E-9A12-7C8F1E2D3A40")!
private let kStreamUUID = UUID(uuidString: "8E6C3B74-5D4A-4B9E-9A12-7C8F1E2D3A41")!

final class NectarLinkStreamSource: NSObject, CMIOExtensionStreamSource {
    private(set) var stream: CMIOExtensionStream!
    private let device: CMIOExtensionDevice
    private let streamFormats: [CMIOExtensionStreamFormat]
    private var activeFormatIndex = 0
    private var streamingClients = Set<CMIOExtensionClient>()
    private var timer: DispatchSourceTimer?
    private let queue = DispatchQueue(label: "com.nectarlink.desktop.camera-extension.stream", qos: .userInteractive)
    private let reader = SharedFrameReader()
    private var pixelBufferPool: CVPixelBufferPool?
    private var poolWidth = 0
    private var poolHeight = 0

    init(localizedName: String, streamID: UUID, device: CMIOExtensionDevice) {
        self.device = device
        let resolutions: [(Int32, Int32)] = [(1280, 720), (1920, 1080), (3840, 2160)]
        var formats: [CMIOExtensionStreamFormat] = []
        for (w, h) in resolutions {
            var desc: CMFormatDescription?
            let status = CMVideoFormatDescriptionCreate(
                allocator: kCFAllocatorDefault,
                codecType: kCVPixelFormatType_32BGRA,
                width: w,
                height: h,
                extensions: nil,
                formatDescriptionOut: &desc
            )
            if status == noErr, let desc {
                let frameDuration = CMTime(value: 1, timescale: 30)
                formats.append(
                    CMIOExtensionStreamFormat(
                        formatDescription: desc,
                        maxFrameDuration: frameDuration,
                        minFrameDuration: frameDuration,
                        validFrameDurations: nil
                    )
                )
            }
        }
        self.streamFormats = formats
        super.init()
        self.stream = CMIOExtensionStream(
            localizedName: localizedName,
            streamID: streamID,
            direction: .source,
            clockType: .hostTime,
            source: self
        )
    }

    var formats: [CMIOExtensionStreamFormat] {
        streamFormats
    }

    var availableProperties: Set<CMIOExtensionProperty> {
        [.streamActiveFormatIndex, .streamFrameDuration]
    }

    func streamProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionStreamProperties {
        let props = CMIOExtensionStreamProperties(dictionary: [:])
        if properties.contains(.streamActiveFormatIndex) {
            props.activeFormatIndex = activeFormatIndex
        }
        if properties.contains(.streamFrameDuration) {
            props.frameDuration = CMTime(value: 1, timescale: 30)
        }
        return props
    }

    func setStreamProperties(_ streamProperties: CMIOExtensionStreamProperties) throws {
        if let idx = streamProperties.activeFormatIndex, idx >= 0, idx < streamFormats.count {
            activeFormatIndex = idx
        }
    }

    func authorizedToStartStream(for client: CMIOExtensionClient) -> Bool {
        true
    }

    func startStream() throws {
        queue.async { [weak self] in
            self?.startTimerIfNeeded()
        }
    }

    func stopStream() throws {
        queue.async { [weak self] in
            self?.stopTimer()
        }
    }

    private func startTimerIfNeeded() {
        guard timer == nil else { return }
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now(), repeating: .nanoseconds(33_333_333), leeway: .milliseconds(2))
        t.setEventHandler { [weak self] in
            self?.emitFrame()
        }
        t.resume()
        timer = t
    }

    private func stopTimer() {
        timer?.cancel()
        timer = nil
    }

    private func ensurePool(width: Int, height: Int) -> CVPixelBufferPool? {
        if let pool = pixelBufferPool, poolWidth == width, poolHeight == height {
            return pool
        }
        pixelBufferPool = nil
        poolWidth = width
        poolHeight = height
        let attrs: [CFString: Any] = [
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_32BGRA,
            kCVPixelBufferWidthKey: width,
            kCVPixelBufferHeightKey: height,
            kCVPixelBufferIOSurfacePropertiesKey: [:],
        ]
        var pool: CVPixelBufferPool?
        CVPixelBufferPoolCreate(kCFAllocatorDefault, nil, attrs as CFDictionary, &pool)
        pixelBufferPool = pool
        return pool
    }

    private func currentDimensions() -> (Int, Int) {
        let format = streamFormats[activeFormatIndex]
        let dims = CMVideoFormatDescriptionGetDimensions(format.formatDescription)
        return (Int(dims.width), Int(dims.height))
    }

    private func emitFrame() {
        let (targetW, targetH) = currentDimensions()
        guard let pool = ensurePool(width: targetW, height: targetH) else { return }

        var pixelBuffer: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, pool, &pixelBuffer) == kCVReturnSuccess,
              let pb = pixelBuffer
        else {
            return
        }

        CVPixelBufferLockBaseAddress(pb, [])
        defer { CVPixelBufferUnlockBaseAddress(pb, []) }

        if let baseAddress = CVPixelBufferGetBaseAddress(pb) {
            let dstBytesPerRow = CVPixelBufferGetBytesPerRow(pb)
            if let frame = reader.readFrame() {
                copyAspectFitBGRX(
                    srcData: frame.data,
                    srcW: frame.width,
                    srcH: frame.height,
                    dst: baseAddress,
                    dstW: targetW,
                    dstH: targetH,
                    dstBytesPerRow: dstBytesPerRow
                )
            } else {
                fillIdleSlate(dst: baseAddress, width: targetW, height: targetH, bytesPerRow: dstBytesPerRow)
            }
        }

        var formatDesc: CMVideoFormatDescription?
        guard CMVideoFormatDescriptionCreateForImageBuffer(
            allocator: kCFAllocatorDefault,
            imageBuffer: pb,
            formatDescriptionOut: &formatDesc
        ) == noErr, let desc = formatDesc else {
            return
        }

        let now = CMClockGetTime(CMClockGetHostTimeClock())
        var timing = CMSampleTimingInfo(
            duration: CMTime(value: 1, timescale: 30),
            presentationTimeStamp: now,
            decodeTimeStamp: .invalid
        )
        var sampleBuffer: CMSampleBuffer?
        guard CMSampleBufferCreateForImageBuffer(
            allocator: kCFAllocatorDefault,
            imageBuffer: pb,
            dataReady: true,
            makeDataReadyCallback: nil,
            refcon: nil,
            formatDescription: desc,
            sampleTiming: &timing,
            sampleBufferOut: &sampleBuffer
        ) == noErr, let sbuf = sampleBuffer else {
            return
        }

        let hostNs = UInt64(CMTimeGetSeconds(now) * 1_000_000_000.0)
        stream.send(sbuf, discontinuity: [], hostTimeInNanoseconds: hostNs)
    }

    private func fillIdleSlate(dst: UnsafeMutableRawPointer, width: Int, height: Int, bytesPerRow: Int) {
        // Dark slate (#121318) in BGRA
        for y in 0..<height {
            let row = dst.advanced(by: y * bytesPerRow).assumingMemoryBound(to: UInt32.self)
            for x in 0..<width {
                row[x] = 0xFF12_1318
            }
        }
    }

    private func copyAspectFitBGRX(
        srcData: Data,
        srcW: Int,
        srcH: Int,
        dst: UnsafeMutableRawPointer,
        dstW: Int,
        dstH: Int,
        dstBytesPerRow: Int
    ) {
        srcData.withUnsafeBytes { raw in
            guard let srcBase = raw.baseAddress else { return }
            if srcW == dstW && srcH == dstH {
                let srcStride = srcW * 4
                for y in 0..<dstH {
                    memcpy(dst.advanced(by: y * dstBytesPerRow), srcBase.advanced(by: y * srcStride), srcStride)
                }
                return
            }

            fillIdleSlate(dst: dst, width: dstW, height: dstH, bytesPerRow: dstBytesPerRow)
            let scale = min(Double(dstW) / Double(srcW), Double(dstH) / Double(srcH))
            let fitW = max(1, Int(Double(srcW) * scale))
            let fitH = max(1, Int(Double(srcH) * scale))
            let offX = (dstW - fitW) / 2
            let offY = (dstH - fitH) / 2
            let srcStride = srcW * 4

            for dy in 0..<fitH {
                let sy = min(srcH - 1, (dy * srcH) / fitH)
                let srcRow = srcBase.advanced(by: sy * srcStride).assumingMemoryBound(to: UInt32.self)
                let dstRow = dst.advanced(by: (offY + dy) * dstBytesPerRow).assumingMemoryBound(to: UInt32.self)
                for dx in 0..<fitW {
                    let sx = min(srcW - 1, (dx * srcW) / fitW)
                    dstRow[offX + dx] = srcRow[sx] | 0xFF00_0000
                }
            }
        }
    }
}

final class NectarLinkDeviceSource: NSObject, CMIOExtensionDeviceSource {
    private(set) var device: CMIOExtensionDevice!
    private var streamSource: NectarLinkStreamSource!

    init(localizedName: String) {
        super.init()
        self.device = CMIOExtensionDevice(
            localizedName: localizedName,
            deviceID: kDeviceUUID,
            legacyDeviceID: nil,
            source: self
        )
        self.streamSource = NectarLinkStreamSource(
            localizedName: "NectarLink Video",
            streamID: kStreamUUID,
            device: self.device
        )
        do {
            try self.device.addStream(self.streamSource.stream)
        } catch {
            NSLog("NectarLinkCamera: failed to add stream: %@", String(describing: error))
        }
    }

    var availableProperties: Set<CMIOExtensionProperty> {
        [.deviceTransportType, .deviceModel]
    }

    func deviceProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionDeviceProperties {
        let props = CMIOExtensionDeviceProperties(dictionary: [:])
        if properties.contains(.deviceTransportType) {
            // kIOAudioDeviceTransportTypeVirtual ('virt')
            props.transportType = 0x7669_7274
        }
        if properties.contains(.deviceModel) {
            props.model = "NectarLink Virtual Camera"
        }
        return props
    }

    func setDeviceProperties(_ deviceProperties: CMIOExtensionDeviceProperties) throws {}
}

final class NectarLinkProviderSource: NSObject, CMIOExtensionProviderSource {
    private(set) var provider: CMIOExtensionProvider!
    private var deviceSource: NectarLinkDeviceSource!

    init(clientQueue: DispatchQueue?) {
        super.init()
        self.provider = CMIOExtensionProvider(source: self, clientQueue: clientQueue)
        self.deviceSource = NectarLinkDeviceSource(localizedName: "NectarLink Webcam")
        do {
            try self.provider.addDevice(self.deviceSource.device)
        } catch {
            NSLog("NectarLinkCamera: failed to add device: %@", String(describing: error))
        }
    }

    func connect(to client: CMIOExtensionClient) throws {}
    func disconnect(from client: CMIOExtensionClient) {}

    var availableProperties: Set<CMIOExtensionProperty> {
        [.providerManufacturer]
    }

    func providerProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionProviderProperties {
        let props = CMIOExtensionProviderProperties(dictionary: [:])
        if properties.contains(.providerManufacturer) {
            props.manufacturer = "NectarLink"
        }
        return props
    }

    func setProviderProperties(_ providerProperties: CMIOExtensionProviderProperties) throws {}
}
