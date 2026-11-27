# Releasing

How a version of NectarLink gets from `main` to people's devices, and how the
apps update themselves afterwards.

## What a release is

A GitHub release with five files:

| File | Built by |
|---|---|
| `Nectarlink-<version>-x64-setup.exe` | The `windows` job: `scripts/package.ps1`, then `scripts/smoke-installer.ps1` installs and uninstalls it |
| `Nectarlink-<version>-arm64-setup.exe` | The `windows-arm64` job: cross-compiled and packaged via `scripts/package.ps1 -Arch arm64` |
| `Nectarlink-<version>-macos-universal.dmg` | The `macos` job: `build-dmg.sh`, signed and notarized when the Apple secrets are set (see Signing) |
| `Nectarlink-<version>-android.apk` | The `android` job: a release build for 64-bit and 32-bit ARM, signed with the release key and checked with `apksigner verify` |
| `SHA256SUMS.txt` | The `publish` job, over all installers and APKs |

Each app's updater needs its own file and `SHA256SUMS.txt`: it only
accepts an installer that `SHA256SUMS.txt` lists with a matching hash.
Each Windows updater picks the installer for its own architecture.

## Steps

1. **Version.** `Cargo.toml` (`[workspace.package] version`) is the only
   place it's set: the Windows app, the installer and the Android app
   (`versionName`, and `versionCode` derived from it) all read it. Use
   `MAJOR.MINOR.PATCH`; a pre-release such as `0.2.0-beta.1` is allowed, and
   `[workspace.metadata.release] name` gives the release a name. The
   README's download table links straight to the files, so change the
   version in its two links too.
2. **Checks.** `./scripts/check.ps1 -Android` locally, and a green CI run on
   the commit to release.
3. **Tag and push.** `git tag v<version>` on that commit and push the tag.
   The `Release` workflow refuses a tag that doesn't match `Cargo.toml`. It
   builds with `build.yml`, the same workflow nightlies use.
4. **Review the draft.** The workflow creates a draft release with generated
   notes. Edit the notes (what's new, anything users must do), check the
   files are there, then **Publish**. Publishing runs `channels.yml`, which
   points the stable feed at the release; installed copies on Stable see it
   within half a day, sooner when a paired PC tells its phone.

## Channels and nightlies

Installs follow a channel, picked in Settings: **Stable** (releases) or
**Nightly**. Each channel has a feed, a small JSON file on the `channels`
branch (`stable.json`, `nightly.json`), with the newest build's version,
name, notes page, and each file's download address and SHA-256
(`.github/scripts/write-feed.py`; the format is in
`desktop/app/src/feed.rs`). The apps read it with a conditional request, so
an unchanged feed costs a few hundred bytes; until a feed exists they fall
back to GitHub's latest release. A Nightly install reads both feeds, so it
takes a release that's newer than its nightly.

`nightly.yml` runs only when started by hand (Actions, or
`gh workflow run Nightly`). When `main` changed (more than docs) since
the last nightly and CI passed on it, it builds `<next>-nightly.N` with
`build.yml`, publishes it as a pre-release named for a flower
(`nightly-name.py`, "Golden Lavender"), points the nightly feed at it and keeps
the last six nightlies. `<next>` is the version in `Cargo.toml`, or the next
patch when that's already released; `set-version.sh` sets it for the build.
Run it by hand from the Actions tab (with **force** to build even if nothing
changed).

Version order: `1.2.3-nightly.N` comes after `1.2.2` and before `1.2.3`, and
`nightly.10` after `nightly.9`. Android's `versionCode` is
`major·10⁷ + minor·10⁵ + patch·10³ + build`, where `build` is N for a nightly
and 999 for a release; the Windows file and package versions end in the same
number, and Linux packages use `1.2.3~nightly.N`, which apt and dnf sort
before `1.2.3`. So a nightly install moves to the release when it's out, and
a phone can't go from a nightly back to an older release without
reinstalling.

### Canary

`canary.yml` builds `main` as it is, for people who want to try what landed
before a nightly. It runs after each push to `main` that CI passed, when the
apps changed (not for docs, `.github/` or `tools/report-worker/` alone), and
only from a commit CI built and tested. GitHub keeps at most one canary
waiting behind the running one, and a newer push replaces it, so builds
never pile up. Start one by hand with `gh workflow run Canary` (`-f force=true`
builds even when nothing changed).

A canary is the pre-release under the tag `canary`, replaced each time, with
fixed file names (`Nectarlink-canary-x64-setup.exe`, the APK, the DMG and the
Linux packages), so a download link keeps working. Its release notes start
with a warning that it's untested. No feed points at it and the apps never
offer it: its version is the last nightly's with build metadata,
`1.2.3-nightly.N+canary.<commit>`, which orders like that nightly. So it
installs over that nightly (same key, same Android `versionCode`), and the
next nightly or release updates it as usual.

## Signing

**Android.** Release builds are signed with the key in the
`ANDROID_KEYSTORE` (base64 of the `.jks`) and `ANDROID_KEYSTORE_PASSWORD`
repository secrets; the key's alias is `nectarlink`. Android only installs an
update signed with the same key as the installed app, so:

- Keep an offline backup of the keystore and its password. If the key is
  lost, no existing install can ever be updated again; everyone would have to
  uninstall and reinstall.
- Builds signed with the debug key (local `assembleRelease` without
  `NECTARLINK_KEYSTORE`) can't update to or from official releases.
- To sign locally: set `NECTARLINK_KEYSTORE` to the `.jks` path and
  `NECTARLINK_KEYSTORE_PASSWORD`, then `./gradlew assembleRelease`.

**macOS.** `desktop/packaging/macos/build-dmg.sh` builds a universal app
(`aarch64-apple-darwin` and `x86_64-apple-darwin`, joined with `lipo`), signs
every library, framework and executable inside out with a Developer ID
certificate, the hardened runtime and `NectarLink.entitlements` (Qt Quick's
JIT needs two of them), notarizes the app and the DMG with `notarytool` and
staples the tickets. `build.yml` does it when these repository secrets are
set, and falls back to an ad-hoc signature (the user must click "Open Anyway")
when they aren't, as on forks:

| Secret | What |
|---|---|
| `APPLE_CERT_P12` | Base64 of the **Developer ID Application** certificate with its private key, exported from Keychain Access as a `.p12` |
| `APPLE_CERT_PASSWORD` | The password the `.p12` was exported with |
| `APPLE_API_KEY` | Base64 of an App Store Connect API key (`.p8`) with the Developer role, which `notarytool` signs in with |
| `APPLE_API_KEY_ID` | That key's ID |
| `APPLE_API_ISSUER` | The issuer ID shown on App Store Connect's Keys page |

All five must be set, or none: with the certificate and key present the
script runs with `REQUIRE_SIGNING=1`, so a rejected notarization fails the
build instead of shipping an unsigned app. To sign on your own Mac, set
`SIGN_IDENTITY` (a `Developer ID Application: ...` name from `security
find-identity -v -p codesigning`) and either `NOTARY_PROFILE` (made once with
`xcrun notarytool store-credentials`) or the three `APPLE_API_*` variables,
then run `bash desktop/packaging/macos/build-dmg.sh`. `ARCH=arm64` builds for
the Mac's own chip only, which is what Homebrew's Qt supports; a universal
build needs the universal Qt from the Qt installer. The Mac updater doesn't
exist yet, so the DMG isn't in the update feed.

**Windows.** The installer and the app aren't Authenticode-signed yet, so
SmartScreen warns on first download until the files build reputation. Options
for open-source projects: [SignPath Foundation](https://signpath.org) (free
certificates for qualifying projects, signing in CI) or Azure Trusted
Signing. Either plugs in after `package.ps1` builds the installer, before
`SHA256SUMS.txt` is written.

## How the apps update

- **Windows** (`desktop/app/src/updater.rs`): checks its channel's feed 30
  seconds after start, then every 12 hours on Stable and every 2 hours on
  Nightly (twice as long on battery), and sooner when a phone tells it of a
  build (`update_relay.rs`, `event.update`); it tells its phones when it sees
  one. The **Nectarlink Update** task reads the channel from
  `%ProgramData%\Nectarlink\update-channel`, which the app writes; any user
  may change it, and it only picks between this repository's own builds. With **Install updates automatically** on,
  it waits until nobody has used the PC for 10 minutes and nothing is in
  progress, then starts the **Nectarlink Update** task that the installer
  registers (`desktop/installer/nectarlink.nsi`). The task runs as SYSTEM,
  downloads and checks the installer itself, and runs it silently; a hidden
  relauncher reopens the app afterwards. The task logs to `update.log` in the
  install folder. Details and the security reasoning:
  [security model](security-model.md#automatic-updates-and-release-verification).
- **Android** (`update/AppUpdater.kt`): hears of new builds from its PC
  (`onHint`), and checks on its own daily on Stable or every six hours on
  Nightly, and when the app opens; installs through `PackageInstaller`.
  Android 12 and later skip the confirmation once NectarLink installed the
  running version itself; the first update after a manual install asks once.

## Testing an update before publishing

Updaters only look at the latest published release, so test with two
versions on a test PC and phone: install version N from its draft's files,
publish N+1, and check that both apps update by themselves (`update.log` on
the PC; `adb logcat -s AppUpdater` on the phone).
