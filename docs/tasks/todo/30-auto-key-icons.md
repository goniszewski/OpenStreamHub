# 30 · Automatic key icons for Launch app and Open URL

**Priority:** P1 · **Effort:** S–M · **Hardware needed:** no

## Why
APH Networks had to import Steam and Genshin Impact icons by hand because the official app does not use the app's own icon.

## Scope
- When a key gets a Launch app action, draw its artwork from the app's icon (read from the app bundle) on first assignment, unless the user already designed one.
- Open URL: site icon only if fetched with explicit user consent (the app has a no-network posture); otherwise offer a text/glyph default from the library.
- Result opens in the artwork editor ([16](../done/16-key-artwork-design-persistence.md)) so it stays editable; never overwrite a saved design.

## Acceptance
- Picking an app produces a 72×72 key image with its icon; undo restores the previous artwork; tests with a fake bundle.
