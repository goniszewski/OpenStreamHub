# 29 · Wider fader action set

**Priority:** P2 · **Effort:** M · **Hardware needed:** yes (fader module)

## Why
Tom's Hardware found only four things mappable to faders (device brightness, system volume, OBS and Streamlabs mixer), so not all five were usable. Faders are the hardware's most unusual input; a rich action set is a clear advantage. We already have `Action::VolumeFromPosition`.

## Scope
- Per-app volume (macOS), mic input gain, output device volume.
- OBS input volume and a scene-item/filter value (pairs with [04](../in-review/04-obs-live-pickers.md) pickers).
- Module/lighting brightness, and generic MIDI CC out if a virtual port is feasible (decide in design).
- Fader picker in the inspector shows only fader-capable actions, with live position readout.
- Deadband/jitter handling, informed by the fader range checks still open in the parity doc.

## Acceptance
- Each new action moves the target as the fader moves; Test mode logs "Would set …". Fader range and jitter recorded in the hardware results doc.
