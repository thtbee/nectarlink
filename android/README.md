# Nectarlink for Android

Kotlin + Jetpack Compose (Material 3) on top of the shared Rust core,
called through UniFFI (`core/nectarlink-ffi`).

## Build

Needs the Android SDK (platform 37, NDK 30.0.16248370), Rust with the
Android targets, and `cargo-ndk`:

```
rustup target add aarch64-linux-android x86_64-linux-android
cargo install cargo-ndk
```

Then, from `android/`:

```
./gradlew assembleDebug                          # phones + x86_64 emulator
./gradlew assembleDebug -Pnectarlink.abis=x86_64 # emulator only (faster)
./gradlew testDebugUnitTest
```

Gradle builds the core with cargo-ndk (cargo profile `android`: release
optimizations, symbols kept for UniFFI and stripped when packaging) and
generates the Kotlin bindings from the built library. Both are generated
sources; nothing generated is checked in except the theme tokens
(`ui/theme/Tokens.kt`, from `cargo xtask tokens`).

## Layout

- `core/`: the bridge to the Rust core. `Core` owns the node and folds its
  events into `CoreState` (a pure reducer, unit-tested); `KeystoreKeyProtector`
  encrypts the identity key with a non-exportable Android Keystore key;
  `Ringer` rings on the alarm stream; battery and network monitors feed the
  core (Android doesn't let native code watch the network).
- `notifications/`: the notification listener (with the user's notification
  access) that hands notifications to the core for mirroring, and reads them
  (messaging-style content, actions, app icons); replies and actions from a
  PC run through `core/PhonePlatform`.
- `clipboard/`: sending text to the PC (share sheet, Quick Settings tile,
  the connection notification's button) through `SendActivity`, which reads
  the clipboard once it has focus, as Android requires; and writing text a
  PC sent.
- `service/ConnectionService`: a "connected device" foreground service that
  keeps the phone reachable and holds the Wi-Fi multicast lock local
  discovery needs.
- `ui/`: pairing (QR scanner with CameraX + ZXing, no Google Play services;
  nearby pairing with a 6-digit code), home and settings. Bloom uses
  Material You colors from the wallpaper on Android 12+.

The identity lives in `noBackupFilesDir`, so it's never restored onto
another phone. `nectarlink://pair?...` links (a PC's QR code scanned with any
camera app) open the app and pair.
