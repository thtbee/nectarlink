# Contributing to Nectarlink

Thanks for your interest! Nectarlink is in early development, so the codebase
and these guidelines will change quickly.

## Before you start

- Read [docs/PLAN.md](docs/PLAN.md) to understand the goals, the architecture
  and the roadmap.
- For anything bigger than a small fix, open an issue first so we can agree on
  the approach.
- Be kind. Everyone is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Sign your commits (DCO)

Nectarlink uses the [Developer Certificate of Origin](DCO.txt) instead of a
CLA. By signing off, you certify that you wrote the change or otherwise have
the right to submit it under the project's licenses.

Add a sign-off line to every commit:

```
git commit -s -m "Your message"
```

This appends a line like:

```
Signed-off-by: Your Name <you@example.com>
```

Commits without a sign-off can't be merged.

## Licensing of contributions

Your contribution is licensed under the license that covers the files you
change, as described in the top-level [LICENSE](LICENSE) file. For GPL-licensed
parts, this includes the [app-store exception](LICENSES/APP-STORE-EXCEPTION.md).

## Ground rules for code

These come from the plan and will grow into a full style guide:

- **Rust is the source of truth.** Business logic lives in Rust. QML only
  describes screens, layout and animation.
- **One bridge.** Only the `nectarlink-qt` crate talks to Qt.
- **Privacy first.** Never log notification text, message content, file names
  or keys. No telemetry.
- **All user-facing strings are translatable** (`qsTr` in QML, string
  resources on Android).
- **Accessibility is not optional.** Labels for screen readers and full
  keyboard navigation from day one.
- Format with `cargo fmt` and keep `cargo clippy` clean. Android code follows
  the official Kotlin style.

## Reporting bugs

Open an issue with your Windows build, phone model, Android version, and the
Nectarlink versions on both sides. Security problems go through
[SECURITY.md](SECURITY.md) instead, never a public issue.
