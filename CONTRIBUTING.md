# Contributing to NectarLink

Thank you for helping with NectarLink. The project is young, so the codebase
and these guidelines still change often.

## Before you start

- Read [docs/PLAN.md](docs/PLAN.md) to understand the goals, the architecture
  and the roadmap.
- For anything bigger than a small fix, open an issue first so we can agree on
  the approach.
- Be kind. Everyone is expected to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Sign your commits (DCO)

NectarLink uses the [Developer Certificate of Origin](DCO.txt) instead of a
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

## Building and checking

The [README](README.md#building) lists the requirements and build commands.
Before opening a pull request, run the same checks as CI:

```powershell
./scripts/check.ps1
```

The desktop app's shared QML components have tests in
`desktop/app/tests/qml` (`tst_*.qml`, Qt Quick Test), which check.ps1 runs
through `./scripts/qml-tests.ps1`. The Rust singletons the components read
are stubbed in `desktop/app/tests/qml/stubs`; a test that needs one more
adds it there.

## Ground rules for code

These come from the plan and will grow into a full style guide:

- **Rust is the source of truth.** Business logic lives in Rust. QML only
  describes screens, layout and animation.
- **One bridge.** Only `desktop/app/src/bridge` (cxx-qt) connects Rust to Qt.
- **Privacy first.** Never log notification text, message content, file names
  or keys. No telemetry.
- **All user-facing strings are translatable**: `qsTr` in QML,
  `tr!("Context", "Sent to %1", name)` for text the desktop app builds in
  Rust, string resources on Android. After changing text on Windows, run
  `cargo xtask strings`, then `lupdate` over every catalogue and the
  template, as [docs/translating.md](docs/translating.md) shows. That page
  also covers translating NectarLink into a new language.
- **Accessibility is not optional.** Labels for screen readers and full
  keyboard navigation from day one.
- Format with `cargo fmt` and keep `cargo clippy` clean. Android code follows
  the official Kotlin style.

## Questions and ideas

Questions, ideas that need talking through, and "how do I…" belong in
[Discussions](https://github.com/alejandrokundrah/nectarlink/discussions), not issues.

## Reporting bugs

Open an issue with your Windows build, phone model, Android version, and the
NectarLink versions on both sides. Security problems go through
[SECURITY.md](SECURITY.md) instead, never a public issue.
