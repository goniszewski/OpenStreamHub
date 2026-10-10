# 10 · Dial functions and display modes in the dial inspector

**Priority:** P1 · **Effort:** M · **Hardware needed:** no for editor

## Why
Official dial panel has Edit / Sort / LCD Display tabs: ordered function list (press cycles), drag-sort, display mode (Default, CPU/GPU, Clock Date/Analog, Image). Ours is `DialFunctionsPanel` buried on the Profiles page, and the dial face is behind an experimental Settings toggle.

## Scope
- Move dial functions into the dial inspector with tabs: Functions (add, remove, drag-sort) · Display (mode picker, clock style, image upload).
- Show the generated dial face as the preview inside the module image (extends `EncoderDisplayContent`).
- Promote dial display from experimental toggle to a per-profile setting once hardware acceptance in `masterhub-dial-functions.md` is done.
- Show only CPU usage (GPU/temperature are out of scope per parity tracker).

## Acceptance
- Existing `dial_functions` profiles load unchanged; tests updated.
- Preview face matches `dial_artwork.rs` output.
