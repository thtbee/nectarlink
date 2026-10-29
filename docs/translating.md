# Translating NectarLink

NectarLink's Android and Windows apps can be translated into any language.
Simplified Chinese, German and Turkish are complete today.

## How to help

**On Weblate** (once the project is set up there; see below): pick a language,
translate in the browser, and Weblate opens a pull request with your work. No
tools or Git needed.

**In a pull request**: edit the files directly.

- Android: `android/app/src/main/res/values-<language>/strings.xml`, for example
  `values-de/strings.xml` or `values-pt-rBR/strings.xml`. Copy the entries
  from `values/strings.xml` and translate their text. Keep placeholders such as
  `%1$s` and `%2$d` exactly as they are; their order in the sentence can change.
- Windows: `desktop/app/i18n/nectarlink_<language>.ts`, for example
  `nectarlink_de.ts`. Start a new language by copying
  `desktop/app/i18n/template/nectarlink.ts` (every string, none translated),
  then fill in the `<translation>` elements with Qt Linguist or a text editor.
  Placeholders are `%1`, `%2` and so on.

A few things that keep translations consistent:

- Name each language in its own language in pickers ("Deutsch", "简体中文").
- Keep the product name "NectarLink" as it is.
- Short UI labels stay short; the English text is already written to fit.
- If a string's meaning isn't clear from its context, ask in the pull request
  or on [Discussions](https://github.com/thtbee/nectarlink/discussions).

## Partly translated languages

A language can be merged while it's still in progress. Until it's complete it
isn't offered anywhere, and it isn't shipped: a phone or PC set to that
language keeps showing English rather than a mix.

- Android: the APK only contains the languages listed in
  `android/app/src/main/res/xml/locales_config.xml` (`localeFilters` in
  `android/app/build.gradle.kts` reads that file). Android lint doesn't fail
  on missing translations (`android/app/lint.xml`).
- Windows: the installer only compiles the catalogues for languages in
  `i18n::LANGUAGES` (`desktop/app/src/i18n.rs`; `scripts/package.ps1` reads it).

## Shipping a finished language

When a language is complete:

1. Android: add it to `res/xml/locales_config.xml` and to
   `AppLanguage.languages` (`ui/AppLanguage.kt`), with its name in its own
   language. `LocalesTest` checks that the two match and that every offered
   language has every string.
2. Windows: add it to `i18n::LANGUAGES` with its name in its own language. A
   test checks that every offered catalogue has no unfinished strings.
3. Add a line to `CHANGELOG.md` crediting the translators.

## For developers: keeping catalogues current

Android strings live in `values/strings.xml`; nothing else to run.

On Windows, after adding or changing text in QML or in Rust (`tr!`), update
the generated header and every catalogue, the template included:

```bash
cargo xtask strings
lupdate desktop/app/qml desktop/app/i18n/rust_strings.h -ts desktop/app/i18n/nectarlink_*.ts desktop/app/i18n/template/nectarlink.ts
```

New strings appear as unfinished in each catalogue, so translators see them.
Offered languages must have their new strings translated in the same change;
the test above fails otherwise.

## Setting up Weblate (maintainers)

[Hosted Weblate](https://hosted.weblate.org/) is free for open-source projects
(the Libre plan). Create a project named NectarLink with the repository
`https://github.com/thtbee/nectarlink.git`, branch `main`, and have Weblate
push to a fork and open pull requests rather than push to `main`. Then add
two components:

| Setting | Android app | Windows app |
|---|---|---|
| File format | Android String Resource | Qt Linguist translation file |
| File mask | `android/app/src/main/res/values-*/strings.xml` | `desktop/app/i18n/nectarlink_*.ts` |
| Monolingual base language file | `android/app/src/main/res/values/strings.xml` | (none: TS files are bilingual) |
| Template for new translations | `android/app/src/main/res/values/strings.xml` | `desktop/app/i18n/template/nectarlink.ts` |
| Language code style | Android style | POSIX style using underscore (`zh_CN`) |
| Edit base file | Off | Off |

Turn on the placeholder checks (Weblate enables the Android and Qt format
checks for these formats), and the "Add missing languages" add-on if you want
Weblate to offer every language. Pull requests from Weblate run through CI like
any other.
