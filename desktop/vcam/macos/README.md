# macOS Virtual Camera (CoreMediaIO Camera Extension)

`NectarLinkCamera` is a macOS 12.3+ CoreMediaIO Camera Extension
(`com.apple.cmio.camera-extension`) that exposes `"NectarLink Webcam"` to
video conferencing apps (Zoom, Microsoft Teams, Google Meet, FaceTime, Photo Booth).

## Architecture

- **`NectarLinkCamera/`**: Swift `CMIOExtensionProviderSource` / `CMIOExtensionDeviceSource` /
  `CMIOExtensionStreamSource` reading seqlock-synchronized `BGRX` (`kCVPixelFormatType_32BGRA`)
  frames from `SharedFrameMapping` (`NLVC` v1, defined in `desktop/vcam/src/shm.rs`).
- **`ActivateExtension.swift`**: `OSSystemExtensionManager` activation/deactivation helper.
- **`build-vcam-extension.sh`**: Compiles `com.nectarlink.desktop.camera-extension.systemextension`
  using `swiftc`.

## Apple Developer Signing & Notarization Caveats

1. **Bundle structure**: macOS only loads a `.systemextension` when embedded inside a signed `.app`
   bundle at `NectarLink.app/Contents/Library/SystemExtensions/com.nectarlink.desktop.camera-extension.systemextension`.
2. **Restricted entitlements**: Activating a Camera Extension via `OSSystemExtensionManager` requires:
   - An Apple Developer Team ID and provisioning profiles for both the host `.app` and the extension.
   - `com.apple.developer.system-extension.install` on the host `.app` (`HostApp.entitlements`).
   - `com.apple.security.app-sandbox` and `com.apple.security.application-groups` (`$(TeamIdentifierPrefix)group.com.nectarlink.desktop`)
     so the sandboxed extension can read the shared-memory frame file written by `nectarlink-desktop`.
3. **Notarization & user approval**: Outside Developer Mode (`systemextensionsctl developer on`
   with SIP disabled), macOS requires Developer ID signing + notarization and one-time user
   approval in **System Settings → General → Login Items & Extensions → Camera Extensions**.
4. **Honest capability advertising**: `nectarlink-desktop` checks `systemextensionsctl list` and
   the `.app` bundle's `Contents/Library/SystemExtensions/` directory before advertising
   `camera.virtual` (`WEBCAM_VIRTUAL`) or `addon.vcam` (`WEBCAM_ADDON_VCAM`).
