# 12 · Key macro recorder

**Priority:** P2 · **Effort:** L · **Hardware needed:** no (macOS Accessibility/Input Monitoring permissions)

## Why
Official "Key Actions" has a Hot Key / Macro switch and a Record Macro modal (Start/Stop, Clear, timeline). We have shortcut chords and multi-action but no recorder — a listed parity gap.

## Scope
- Key picker alongside capture (from the reviews: Tom's Hardware could not enter numpad keys or F13–F24 without a second device): every key choosable without pressing it, including F13–F24, numpad keys and media keys. Cheap, so it can ship before the recorder.
- Hotkey capture field ("Press keys…", max 5 keys) replacing manual key/modifier selects in `ActionFields`.
- Macro recorder modal: record key down/up with timing, edit timeline, trim delays, replace as a bounded sequence action (new action type or multi-action step, decide in design).
- Permission flow: detect missing Input Monitoring/Accessibility, explain and deep-link to System Settings.
- Dry-run preview prints the planned keystrokes; high-risk disclosure applies on Execute.

## Acceptance
- Recorded macro replays identically in dry-run and on a real Mac; limits (length, duration) validated in Rust and TS.
