// SPDX-License-Identifier: GPL-3.0-or-later
//
// ActivateExtension.swift — Activates or deactivates the NectarLink Camera
// Extension via `OSSystemExtensionManager`. Must be invoked from a signed
// `.app` bundle with `com.apple.developer.system-extension.install`.

import Foundation
import SystemExtensions

final class ExtensionDelegate: NSObject, OSSystemExtensionRequestDelegate {
    private let semaphore = DispatchSemaphore(value: 0)
    private(set) var exitCode: Int32 = 1

    func request(
        _ request: OSSystemExtensionRequest,
        actionForReplacingExtension existing: OSSystemExtensionProperties,
        withExtension ext: OSSystemExtensionProperties
    ) -> OSSystemExtensionRequest.ReplacementAction {
        .replace
    }

    func requestNeedsUserApproval(_ request: OSSystemExtensionRequest) {
        fputs(
            "NectarLink Camera Extension requires approval in System Settings -> General -> Login Items & Extensions -> Camera Extensions.\n",
            stderr
        )
    }

    func request(_ request: OSSystemExtensionRequest, didFinishWithResult result: OSSystemExtensionRequest.Result) {
        switch result {
        case .completed:
            exitCode = 0
        case .willCompleteAfterReboot:
            fputs("System extension change will complete after reboot.\n", stderr)
            exitCode = 0
        @unknown default:
            exitCode = 0
        }
        semaphore.signal()
    }

    func request(_ request: OSSystemExtensionRequest, didFailWithError error: Error) {
        fputs("System extension request failed: \(error.localizedDescription)\n", stderr)
        exitCode = 1
        semaphore.signal()
    }

    func wait() -> Int32 {
        semaphore.wait()
        return exitCode
    }
}

let bundleID = "com.nectarlink.desktop.camera-extension"
let args = CommandLine.arguments
let deactivate = args.contains("--deactivate") || args.contains("--unregister-vcam")

let delegate = ExtensionDelegate()
let request: OSSystemExtensionRequest = deactivate
    ? .deactivationRequest(forExtensionWithIdentifier: bundleID, queue: .main)
    : .activationRequest(forExtensionWithIdentifier: bundleID, queue: .main)
request.delegate = delegate
OSSystemExtensionManager.shared.submitRequest(request)

DispatchQueue.global().async {
    let code = delegate.wait()
    exit(code)
}
RunLoop.main.run()
