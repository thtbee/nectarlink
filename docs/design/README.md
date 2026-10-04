# Design

Preview: open `themes/index.html` in a browser (switch themes and Material You seeds at the top).

## Decision (2026-10-05)

- **Bloom** is the default theme: the "Bloom Expressive" direction (C), refined to be minimal,
  soothing and professional. Material You tonal color roles, seeded from the phone's wallpaper
  (Honey is the brand default seed). Light and dark.
- **Graphite** is a second, built-in theme, translated from the maintainer's Graphite project
  (github.com/thtbee/graphite): archival paper (`#F5F2EB`) and obsidian slate (`#0D0D0E`),
  ink-black controls, hairline rules, faint drafting grid and paper grain, monospace uppercase
  annotations, crop marks, Instrument Serif display type and a rare handwritten margin note.
  Monochrome by design (Material You off). Variants: Paper and Slate.
- Directions **A (Honey Glass)** and **B (Graphite Pro)** were rejected.

## Principles

- Calm first: neutral surfaces, color only where it carries meaning (state, primary action, the
  connected device).
- No sloppy decoration: every texture, shadow and annotation must survive a 9-hour workday.
- Graphite's art-site effects (smudge physics, gritty lettering, scroll sound) are *not* used in
  the app; only its visual language is.
- Both themes share the same components and layout; themes only change tokens.
- Fonts are bundled (SIL OFL): Figtree (UI), Instrument Serif (Graphite display),
  Space Mono (Graphite labels), Caveat (Graphite margin notes). No fonts from the web at runtime.
