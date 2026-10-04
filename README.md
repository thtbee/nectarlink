# Nectarlink

**Your Android phone and your Windows PC, working as one.**

Nectarlink is one open-source app that replaces the pile of tools people juggle today: Phone Link, KDE Connect, scrcpy, LocalSend and the discontinued Intel Unison.

> Bees carry nectar back to the hive. Nectarlink carries everything between your phone and your PC.

> **Status: pre-alpha, in active development.** Nothing is ready to install yet.

## What it will do

- **Notifications** on your PC with replies and actions, including OTP codes.
- **Messages and calls**: SMS/MMS, one inbox for chat apps, call alerts and controls.
- **Clipboard** synced both ways, with history and smart suggestions.
- **Files and photos**: drag and drop, resumable transfers, your phone's storage inside Explorer, LocalSend compatibility.
- **Mirroring and phone apps on your PC**: each app in its own window, with audio and keyboard/mouse. Nothing extra to install.
- **Phone as webcam, touchpad, air mouse, Stream-Deck-style macro pad, voice recorder and voice keyboard** for your PC.
- **Works everywhere**: Wi-Fi, USB, and optionally away from home. End-to-end encrypted, no account, no cloud.
- **Power Levels**: everything works with normal permissions, and an optional one-time setup unlocks more on any Android phone (no root required).

The full plan, including research, features, architecture and roadmap, is in [docs/PLAN.md](docs/PLAN.md).

## Tech

- **Shared core:** Rust (networking via [iroh](https://iroh.computer), pairing, transfers, storage).
- **Windows app:** Qt Quick (QML) UI on a Rust backend. Windows 11, x64 and ARM64.
- **Android app:** Kotlin + Jetpack Compose (Material 3 Expressive).

## License

- Desktop and Android apps: [GPL-3.0-or-later](LICENSES/GPL-3.0-or-later.txt), with an [app-store exception](LICENSES/APP-STORE-EXCEPTION.md).
- Core libraries: [MPL-2.0](LICENSES/MPL-2.0.txt).
- Protocol specification and documentation: [CC BY 4.0](LICENSES/CC-BY-4.0.txt).

See [LICENSE](LICENSE) for which applies where. The Nectarlink name and logo are covered by the [trademark policy](TRADEMARKS.md).

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md). Contributions are accepted under the [Developer Certificate of Origin](DCO.txt), and everyone is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md). To report a security issue, please follow [SECURITY.md](SECURITY.md).
