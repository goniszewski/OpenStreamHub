# 28 · Per-input lighting (knob rings, fader LEDs, base strip)

**Priority:** P2 · **Effort:** M · **Hardware needed:** yes (knob and fader modules)
Extends [09](../done/09-lighting-panel.md), which only covers the encoder dial backlight.

## Why
Tom's Hardware and Noisy Pixel both complain that colours can only be set per module, so you cannot tell three knobs or five faders apart by colour. Noisy Pixel also found the base edge strip dimmer and less saturated than the knob rings.

## Scope
- Colour for each knob ring and each fader's LED strip individually, plus a "same for all" shortcut.
- Base edge strip colour and brightness, with a "match modules" option.
- Live preview in the rig image; reuse the colour favourites and debounced apply queue from the Lighting page.
- Reset per input and per module.

## Acceptance
- On a real knob and fader module, each input shows its own colour; settings survive reconnect. Unit tests for the model; hardware results recorded in `docs/masterhub-hardware-results.md`.
