<!-- SPDX-License-Identifier: CC-BY-4.0 -->
# macOS native camera backend draft

This is an opt-in source contribution, not a qualified macOS camera release.
Default builds do not compile the producer/activation bridge or activate an
extension. **All builds, including signed opt-in builds, refuse native Start,
legacy incoming streams, producer claim, and `camera.virtual` advertisement.**
The explicit `vcam::stream_admission_ready()` gate remains false until the owned
request/session protocol is ported and reviewed. No helper or driver is downloaded.
The desktop feature flag is `macos-native-camera`.

## Integration

The staged data path connects decoded frames and preview to native output,
but cannot currently admit a macOS phone stream. After an owned-protocol port,
a correctly admitted sink will claim one native producer with a unique `(device, generation)` lease.
Frames pass through the existing BGRX conversion and optional mirror operation
once, then the CMIO sink. Windows shared textures/mappings and Linux webcam
behavior remain on their existing paths. macOS advertises H.264 only.

`WebcamSink::closed()` is an optional, default-compatible exact-sink signal.
Native Stop, end/drop, output failure, and app shutdown revoke the producer and
signal that admitted QUIC stream. Native Stop does not enqueue a deferred
peer-wide stop for an active output. Late packets, configs, previews, and
cleanup cannot publish into or release another lease, including one belonging
to the same phone. No macOS Start can currently request phone capture. Opening a source in a
conferencing app never starts a phone or bypasses consent.

The extension publishes one source (720p, 1080p, 3840×2160 at 30 fps) and one
fixed UHD BGRA sink. The producer aspect-fits portrait/landscape frames. The
extension returns black after Stop, producer disconnect, or a 250 ms stale-frame
deadline. Queues and pixel pools are bounded. Only one signed producer is
accepted; a client PID is checked against the exact host identifier and the
extension's own signing team before it receives sink access.

The proposed upstream bundle identifiers are `app.nectarlink.desktop` and
`app.nectarlink.desktop.CameraExtension`. The new NectarLink device/source/sink
UUIDs intentionally differ from the donor. Maintainers must approve these
identifiers before signing; changing them requires changing the shared header,
activation identifier, and assembly recipe together.

## Build and assembly path

The VideoToolbox H.264 contribution (`c4cdd858d4b5f7059c9f2892d81c06f07641e670`)
is a dependency. The desktop Cargo feature compiles the Objective-C producer and
activation bridge through `build.rs` and links the Apple frameworks. It does not
build or install the separate extension executable. Both native components
require macOS 12.3+ SDK/runtime qualification and the desktop app's own supported
minimum. No OS or architecture support is claimed by source inclusion.

After the desktop build, a disposable candidate `.app` with the reviewed host
identifier may be assembled using:

```sh
python3 scripts/macos/build-native-camera-extension.py \
  --source-root /absolute/source \
  --candidate-app /absolute/scratch/NectarLink.app \
  --team-id YOURTEAMID \
  --version 0.1.4 --build 1 --arch arm64
```

This compiles an **unsigned** payload and writes only its bundle metadata.
It leaves host metadata, profiles, and entitlements alone, and never signs,
activates, launches, or contacts a service. The architecture must match the
qualified desktop candidate. A failed assembly must be discarded with that
disposable candidate rather than reused as release evidence.

## Gates before enabling a release

1. Approve the scope/identifiers and inspect the GPL native-source provenance.
   Port the donor's additive `camera.webcam.owned.v1` chain across protocol,
   core, Android/UniFFI and desktop before opening `stream_admission_ready()`:
   nonzero durable request allocation, retained original Shared/Session/view,
   request-bound Start/Stop/keyframe/Config and video.owned admission, retired
   request fencing across Stop/OFF/ON/reconnect, and exact Android consent Entry
   plus terminal encoder/CameraX cleanup. Preserve upstream HEVC and legacy
   Windows/Linux paths. Matching only a peer or local Asking state cannot
   distinguish a delayed old request from a same-phone successor.
2. Compile/check the default and opt-in desktop, native extension, and existing
   Windows/Linux targets with the supported SDK and toolchains. Run the existing
   core webcam and desktop tests, then exercise Stop/restart, same-phone stream
   replacement, decode/output failure, revocation, and disconnect races.
3. Qualify the host's `com.apple.developer.system-extension.install` grant and
   the extension's `com.apple.developer.cmio.extension` and App Sandbox grants
   using the actual signing team, Apple profiles, certificate and entitlements.
   This repository supplies no accounts, keys, profiles, or entitlement claims.
4. Sign the extension and then the host, qualify their architecture/load closure,
   and apply the project's distribution/notarization requirements. Activation
   is a separate explicit Setup action through public SystemExtensions APIs.
   User approval, activation completion, and device readiness are distinct.
5. Independently verify the installed extension's provenance. Host runtime checks
   validate the host and embedded extension signatures with an Apple-anchored,
   exact-identifier, same-team requirement, and check live CMIO UID/stream formats.
   Public CMIO metadata alone does not attest the installed extension binary.
6. Complete the macOS setup/approval/error presentation before exposing the
   feature in a release. The shared upstream settings row still describes the
   Windows add-on/admin setup. This draft deliberately does not redesign that
   UI. Native pending requests must also use the future owned protocol;
   automatic consumer-triggered phone requests are outside this draft.
7. Qualify phone consent, Stop, mirror handling, portrait/landscape, 720p/1080p/4K,
   and camera permission behavior with a physical phone and receiving video apps.
   Confirm QUIC closure stops the Android foreground encoder, including idle
   capture, reconnect, and app shutdown. Synthetic checks cannot establish this.

Payload presence, signatures, activation and live CMIO publication cannot open
stream admission. The owned-protocol gate currently refuses capture regardless
of all those conditions. After that protocol is implemented, the feature build,
strict signed-payload checks and live exact-format publication remain required. A successful
activation callback does not replace these checks. The command-line Windows
registration entry points return an error on macOS and invoke no privileged
helper.

## Provenance and offline checks

The native-only implementation is curated from GPL-3.0-or-later Waypair Link
commits `f73dc0f76101ffa6e4ac1299e94ca7cef5e3f9df` and
`604ca29c59310305d8c5b73f1761aa27665fa995`, available in donor
`78e31626e4cfa05bee0429855a967c74e18168c7`. Third-party camera providers, brand
assets, private endpoints, installer/download flows, and release-specific
packaging inputs were excluded. The source remains GPL-3.0-or-later under
NectarLink's desktop license; the small core cancellation seam remains MPL-2.0.

The two assembly-recipe tests mock compilation and use disposable local files:
`python3 scripts/macos/test_native_camera_recipe.py`. Rust formatting/parsing and
source review are additional offline checks. They do not establish native
compilation, signature validity, activation, device readiness, or video delivery.
