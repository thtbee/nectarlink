// SPDX-License-Identifier: GPL-3.0-or-later
//
// main.swift — Entry point for NectarLinkCamera.systemextension.

import CoreMediaIO
import Foundation

@main
enum NectarLinkCameraMain {
    static func main() {
        autoreleasepool {
            let providerSource = NectarLinkProviderSource(clientQueue: nil)
            CMIOExtensionProvider.startService(provider: providerSource.provider)
            CFRunLoopRun()
        }
    }
}
