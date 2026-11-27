# macOS Virtual Camera (CoreMediaIO Camera Extension)

`NectarLinkCamera` is a macOS 12.3+ CoreMediaIO Camera Extension
(`com.apple.cmio.camera-extension`) that exposes `"NectarLink Webcam"` to
video conferencing apps (Zoom, Microsoft Teams, Google Meet, FaceTime, Photo Booth).
It is what a phone's **Start webcam** feeds on a Mac.

## Architecture

- **`NectarLinkCamera/`**: Swift `CMIOExtensionProviderSource` / `CMIOExtensionDeviceSource` /
  `CMIOExtensionStreamSource` reading seqlock-synchronized `BGRX` (`kCVPixelFormatType_32BGRA`)
  frames from `SharedFrameMapping` (`NLVC` v1, defined in `desktop/vcam/src/shm.rs`).
  `Info.plist.in` is a template (version, App Group) that the build fills in.
- **`desktop/app/cpp/mac_sysext.mm`**: asks `OSSystemExtensionManager` to activate (or remove) the
  extension, from NectarLink's own executable. It has to be the app's main executable: macOS only
  honors the restricted `com.apple.developer.system-extension.install` entitlement, backed by the
  app's provisioning profile, for that binary. A separate helper executable beside it is killed
  at launch (tried and confirmed). Errors map to messages in `desktop/app/src/mac/vcam.rs`.
- **`build-vcam-extension.sh`**: compiles the extension with `swiftc` (one slice per
  architecture, joined with `lipo`) and renders the entitlements for the team.
- The desktop side (`desktop/app/src/mac/vcam.rs`) writes decoded frames into the App Group
  shared-memory file and requests the activation when the user clicks Settings → Virtual
  camera add-on → Set up.

Identifiers (they have to agree everywhere):

| What | Value |
| --- | --- |
| Host app bundle ID | `app.nectarlink.desktop` |
| Extension bundle ID | `app.nectarlink.desktop.camera-extension` (must start with the host's ID, or macOS refuses to install it) |
| App Group | `<TEAMID>.group.com.nectarlink.desktop` (`group.com.nectarlink.desktop` in builds without a team) |
| Mach service | `<App Group>.camera-extension` (must start with an App Group the extension is in) |

## How the DMG gets it

`desktop/packaging/macos/build-dmg.sh` runs `build-vcam-extension.sh` for the DMG's
architectures (universal by default) and puts the result in the app:

```
NectarLink.app/Contents/Library/SystemExtensions/app.nectarlink.desktop.camera-extension.systemextension
```

Everything is signed inside out (extension first, then the app) with the hardened runtime and
a secure timestamp, so the same notarization of the app and the DMG covers it. The app advertises
`camera.virtual` to phones as soon as the extension is in its bundle (`mac::vcam::is_available`),
and `addon.vcam` once macOS reports it `[activated enabled]`.

Check a build:

```bash
codesign --verify --deep --strict --verbose=2 NectarLink.app
find NectarLink.app/Contents/Library -maxdepth 3
lipo -archs NectarLink.app/Contents/Library/SystemExtensions/*.systemextension/Contents/MacOS/*
codesign -d --entitlements - NectarLink.app/Contents/Library/SystemExtensions/*.systemextension
```

## Provisioning profile (Apple Developer portal)

Installing a system extension needs the *restricted* entitlement
`com.apple.developer.system-extension.install`, and macOS only honors it when a provisioning
profile from Apple backs it. The build can't create profiles; it takes one as a file. The
extension itself needs none: its entitlements (sandbox and the team-prefixed App Group) aren't
restricted. Without the profile the DMG still contains the extension (and the app advertises the
camera), but **Set up fails** with "isn't allowed to install the camera extension", and the app is
signed with its usual entitlements only.

### Release (Developer ID)

1. At <https://developer.apple.com/account> → Certificates, Identifiers & Profiles →
   **Identifiers**, register the **App ID** `app.nectarlink.desktop` (macOS) with the
   **System Extension** capability (and **App Groups** if the portal offers it for macOS).
2. Under **Profiles**, create a **Developer ID** (distribution outside the App Store) provisioning
   profile for it, using the *Developer ID Application* certificate the DMG is signed with.
   Download the `.provisionprofile`.
3. Give it to the build:
   - Locally: `HOST_PROVISIONING_PROFILE=/path/host.provisionprofile
     SIGN_IDENTITY="Developer ID Application: ..." bash desktop/packaging/macos/build-dmg.sh`.
     Add `REQUIRE_VCAM=1` to fail instead of building without it. `APPLE_TEAM_ID` is read from
     the profile if you don't set it.
   - In CI: store the file base64-encoded (`base64 -i file.provisionprofile | pbcopy`) in the
     repository secret `APPLE_HOST_PROFILE`. `build.yml` (Nightlies and releases) installs it and
     sets `REQUIRE_VCAM=1`. `APPLE_VCAM_PROFILE` is optional and only for an extension profile.

Profiles expire (a year at most) and need regenerating; an expired profile makes the app fail to
launch, which the DMG check in `build.yml` catches.

### Trying it locally (Apple Development, no portal clicks)

Xcode can make a development profile for you. In a throwaway project, signed with your own team:
an application target `app.nectarlink.desktop` (entitlement `com.apple.developer.system-extension.install`
and the App Group `$(TeamIdentifierPrefix)group.com.nectarlink.desktop`) and a system-extension
target `app.nectarlink.desktop.camera-extension`. Run
`xcodebuild -allowProvisioningUpdates ... build`; Xcode registers the App IDs and writes the profile
to `~/Library/Developer/Xcode/UserData/Provisioning Profiles/` ("Mac Team Provisioning Profile:
app.nectarlink.desktop"). Then:

```bash
HOST_PROVISIONING_PROFILE=/path/that.provisionprofile REQUIRE_VCAM=1 \
SIGN_IDENTITY="Apple Development: ..." bash desktop/packaging/macos/build-dmg.sh
```

A development-signed extension installed and ran this way on a Mac with SIP on and developer mode off
(approve it once in System Settings). The build only runs on Macs registered in the profile.

## Production checklist (release and nightly workflows)

The workflows (`build.yml`, called by `nightly.yml` and `release.yml`) already sign and notarize the
DMG with `APPLE_CERT_P12`, `APPLE_CERT_PASSWORD` and the `APPLE_API_*` key. To ship a DMG whose
camera extension installs:

1. **App ID.** In the Apple Developer portal of the team that signs releases, register the App ID
   `app.nectarlink.desktop` (macOS) if it isn't there, and enable its **System Extension**
   capability (Identifiers → `app.nectarlink.desktop` → Capabilities). The extension's ID
   `app.nectarlink.desktop.camera-extension` and the App Group need nothing registered.
2. **Developer ID profile.** Profiles → **+** → *Developer ID* (macOS) → pick that App ID → pick the
   *Developer ID Application* certificate that is in `APPLE_CERT_P12` → name it, download the
   `.provisionprofile`. A profile made for another certificate fails the signature check.
3. **Secret.** `base64 -i NectarLink_DeveloperID.provisionprofile | pbcopy`, then add the repository
   secret `APPLE_HOST_PROFILE` (Settings → Secrets and variables → Actions). Nothing else changes:
   `nightly.yml` and `release.yml` already pass it to `build.yml`.
4. **Run it.** Start a Nightly (or push a tag). The log must show `Building the virtual camera
   extension (universal, team <your team ID>)` and no "no provisioning profile" warning; with the
   secret set the build runs with `REQUIRE_VCAM=1` and fails instead of shipping a camera that
   can't install. The step "Check the DMG launches cleanly" runs the signed app, which fails if
   the profile doesn't match the signature.
5. **Test the DMG** from the release on a Mac that never ran a dev build: drag NectarLink to
   Applications, Settings → Virtual camera add-on → Set up, approve in System Settings, start
   **Start webcam** on the phone.
6. **Renew.** Profiles expire (see the date in the portal). An expired or revoked profile makes
   the signed app refuse to launch, so put the expiry in a calendar and replace the secret before it.

Without the secret the DMG still builds, signs and notarizes as before, with the extension bundled
but not installable (Set up says so) and a warning in the log.

Not verified yet: a *Developer ID* profile, the notarization of an app carrying it and the
universal (x86_64) slice of the extension. The development-profile path above is what was tested.

## What users see

- **Settings → Virtual camera add-on → Set up** asks macOS to install the extension. macOS shows a notification
  that a system extension was blocked; NectarLink shows "Waiting for your approval…" until the
  user turns on **NectarLink Webcam** in **System Settings → General → Login Items & Extensions →
  Camera Extensions**, then reports Ready. It stops waiting after 15 minutes.
- A failure (not in `/Applications`, missing entitlement, rejected signature, a restart
  needed) is shown in the same row.
- **Remove** asks macOS to deactivate it.
- Outside developer mode (`systemextensionsctl developer on`, which needs SIP off), macOS
  only loads Developer ID signed and notarized extensions, and only from `/Applications`.
  Ad-hoc builds (CI's, Nightlies without secrets) bundle the extension but can't install it.

## Verified, and not

Verified on an Apple silicon Mac, SIP on, developer mode off: the DMG builds and signs with an
Apple Development certificate and a development profile; the app launches with the profile-backed
entitlements; Set up installs the extension, shows the approval prompt with a button to System
Settings, and reports Ready after the user allows it; the phone then sees the Mac's virtual camera
and streams to it. A separate helper binary carrying the entitlements is killed at launch (hence
`mac_sysext.mm`).

Not verified: the Developer ID / notarized path (the checklist above), the x86_64 slice, and
the older macOS versions.
