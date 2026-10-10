# 38 · Universal module position detection (slot table from the official app)

**Priority:** P2 · **Effort:** M · **Hardware needed:** yes, to confirm the table on 2–3 arrangements
Replaces the per-module `ends_with` rules in `base_position` (`apps/open-stream-hub/src-tauri/src/feed.rs`).

## Why
Today a module is placed only if its exact (module, USB port chain) pair is hard-coded, so every rearrangement shows "Position not detected" (seen 10 October 2026 after the 15-key and rollers moved). Quadrants (4 values) are also too coarse for a 4×6 base.

## What the official app does (1.0.0.113, `app-analysis/sources/src`)
- Not a formula: a lookup table. `App.vue` `arrayContainsSerialNumber` finds the base's own hub among the USB devices, drops that hub's port-path prefix from each module's `portNumbers` and joins the rest into a string (`portNumber`, e.g. `23`, `43`, `134`). That string identifies the base connector slot, independent of module type.
- `Baseboard.vue` (`rollerPos`, `m4315Pos`, `sliderPos`, `knobx1Pos`, `knobx3Pos`) turns it into a CSS class `pos<portNumber>`; the stylesheet (about lines 2460–2900) holds `top`/`left` and a rotation per module class and slot. Pitch is about 129 px per cell; columns at 25 and 283 px, rows at 25, 154, 283, 412, 541, 670.
- Orientation is a separate per-base value (see [37](37-orientation-check.md) for deriving each module's orientation from it, with a per-module override) (`M00x64Direction`, vertical or horizontal) that rotates the whole layout.
- Observed chains here match its strings once the base prefix `1.1` is removed: dial `43`, knobs `34`, keys now `23`; `pos23`, `pos43` and `pos34` all exist in its CSS.

## Scope
- Compute the base prefix from the connected base hub (as above) instead of matching fixed suffixes.
- Extract the slot table from `Baseboard.vue` with a script into a data file in the repo (slot string → cell, plus the per-module connector offset and rotation), keeping our own format; do not copy CSS or art.
- Return a cell position (row, column) and let the rig preview place any module by its footprint (4×3 keys, 4×1 rollers, 2×2 dial and knobs, faders) and orientation, instead of four quadrants.
- Unknown slot strings stay "not detected" with the chain shown in diagnostics.

## Acceptance
- Unit tests for the table against the chains already recorded in `docs/masterhub-hardware-results.md` (both arrangements).
- Rearranging modules on real hardware places each correctly without code changes; record at least one new arrangement in the results doc.
