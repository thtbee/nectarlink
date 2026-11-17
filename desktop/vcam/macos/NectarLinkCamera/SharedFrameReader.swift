// SPDX-License-Identifier: GPL-3.0-or-later
//
// SharedFrameReader.swift — Reads seqlock-synchronized BGRX frames from the
// `SharedFrameMapping` (`NLVC` v1) file written by `nectarlink-desktop`.

import Darwin
import Foundation

final class SharedFrameReader {
    static let magic: UInt32 = 0x4E4C_5643 // "NLVC"
    static let version: UInt32 = 1
    static let maxWidth = 3840
    static let maxHeight = 2160
    static let maxFrameBytes = maxWidth * maxHeight * 4
    static let headerBytes = 128
    static let totalMappingBytes = headerBytes + maxFrameBytes

    static let appGroupID = "group.com.nectarlink.desktop"
    static let appGroupFileName = "nectarlink_vcam_frame_v2"
    static let defaultMappingName = "Global_NectarlinkVcamFrame_v2"

    // Exact byte offsets of `SharedHeader` in `desktop/vcam/src/shm.rs`
    private enum Offset {
        static let magic = 0
        static let version = 4
        static let seq = 8
        static let active = 16
        static let width = 20
        static let height = 24
        static let stride = 28
        static let timestampUs = 40
    }

    private var fd: Int32 = -1
    private var view: UnsafeMutableRawPointer?
    private var lastOpenAttempt = Date.distantPast
    private var cachedWidth = 0
    private var cachedHeight = 0
    private var cachedTimestamp: UInt64 = 0
    private var cachedFrame = Data()

    deinit {
        closeMapping()
    }

    private func closeMapping() {
        if let ptr = view {
            munmap(ptr, Self.totalMappingBytes)
            view = nil
        }
        if fd >= 0 {
            close(fd)
            fd = -1
        }
    }

    private static func candidatePaths() -> [URL] {
        var urls: [URL] = []

        if let groupURL = FileManager.default.containerURL(
            forSecurityApplicationGroupIdentifier: appGroupID
        ) {
            urls.append(groupURL.appendingPathComponent(appGroupFileName))
        }

        // Team-prefixed App Group containers (if present when unsandboxed)
        if let home = FileManager.default.homeDirectoryForCurrentUser as URL? {
            let groupContainers = home
                .appendingPathComponent("Library/Group Containers", isDirectory: true)
            if let entries = try? FileManager.default.contentsOfDirectory(
                at: groupContainers,
                includingPropertiesForKeys: nil
            ) {
                for entry in entries where entry.lastPathComponent.hasSuffix(appGroupID) {
                    urls.append(entry.appendingPathComponent(appGroupFileName))
                }
            }
        }

        let uid = geteuid()
        let shmName = "nectarlink_shm_\(uid)_\(defaultMappingName)"
        urls.append(URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent(shmName))
        urls.append(URL(fileURLWithPath: "/tmp").appendingPathComponent(shmName))
        return urls
    }

    private func ensureMapped() -> UnsafeRawPointer? {
        if let ptr = view {
            return UnsafeRawPointer(ptr)
        }
        let now = Date()
        if now.timeIntervalSince(lastOpenAttempt) < 0.5 {
            return nil
        }
        lastOpenAttempt = now

        for url in Self.candidatePaths() {
            let opened = open(url.path, O_RDONLY | O_NOFOLLOW)
            if opened < 0 {
                continue
            }
            var st = stat()
            if fstat(opened, &st) != 0 || st.st_size < Int64(Self.totalMappingBytes) {
                close(opened)
                continue
            }
            let mapped = mmap(
                nil,
                Self.totalMappingBytes,
                PROT_READ,
                MAP_SHARED,
                opened,
                0
            )
            if mapped == MAP_FAILED || mapped == nil {
                close(opened)
                continue
            }
            fd = opened
            view = mapped
            return UnsafeRawPointer(mapped)
        }
        return nil
    }

    private static func loadUInt32(_ base: UnsafeRawPointer, _ offset: Int) -> UInt32 {
        base.loadUnaligned(fromByteOffset: offset, as: UInt32.self)
    }

    private static func loadUInt64(_ base: UnsafeRawPointer, _ offset: Int) -> UInt64 {
        base.loadUnaligned(fromByteOffset: offset, as: UInt64.self)
    }

    /// Reads the latest live BGRX frame if active, returning `(width, height, timestampUs, data)`.
    /// Returns `nil` when idle or when no mapping is available.
    func readFrame() -> (width: Int, height: Int, timestampUs: UInt64, data: Data)? {
        guard let base = ensureMapped() else { return nil }
        OSMemoryBarrier()

        let magic = Self.loadUInt32(base, Offset.magic)
        let version = Self.loadUInt32(base, Offset.version)
        guard magic == Self.magic, version == Self.version else {
            return nil
        }

        for _ in 0..<3 {
            let seq1 = Self.loadUInt64(base, Offset.seq)
            OSMemoryBarrier()
            if seq1 == 0 || (seq1 & 1) != 0 {
                continue
            }
            let active = Self.loadUInt32(base, Offset.active)
            if active != 1 {
                return nil
            }
            let width = Int(Self.loadUInt32(base, Offset.width))
            let height = Int(Self.loadUInt32(base, Offset.height))
            let stride = Int(Self.loadUInt32(base, Offset.stride))
            let ts = Self.loadUInt64(base, Offset.timestampUs)
            let byteCount = stride * height
            guard width > 0, height > 0, stride >= width * 4, byteCount <= Self.maxFrameBytes else {
                return nil
            }

            let payload = base.advanced(by: Self.headerBytes)
            let copy = Data(bytes: payload, count: byteCount)
            OSMemoryBarrier()
            let seq2 = Self.loadUInt64(base, Offset.seq)
            if seq1 == seq2 {
                cachedWidth = width
                cachedHeight = height
                cachedTimestamp = ts
                cachedFrame = copy
                return (width, height, ts, copy)
            }
        }

        if cachedWidth > 0, cachedHeight > 0, !cachedFrame.isEmpty {
            return (cachedWidth, cachedHeight, cachedTimestamp, cachedFrame)
        }
        return nil
    }
}
