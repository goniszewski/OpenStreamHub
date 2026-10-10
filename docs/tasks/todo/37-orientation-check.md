# 37 · Module orientation: derived from the base, with a per-module override

**Priority:** P2 · **Effort:** M · **Hardware needed:** yes
Builds on [38](38-universal-module-position-detection.md) (slot positions).

## Why
The base can be used vertically or horizontally, and every module's controls, key grid, artwork and dial face must be turned to match. The official app keeps one orientation per base (`M00x64Direction`, vertical or horizontal) and rotates everything with it. Tom's notes the key module's printed logo makes vertical use awkward and that artwork must be rotated by hand. Power users also mount modules turned the other way (faders running left to right, a roller module sideways), which one base-wide value cannot describe.

## Scope
- **Base orientation:** one value for the base (vertical or horizontal), taken from the device's own sensor if the protocol exposes it, otherwise chosen in the app (the official app lets the user set it). Which USB-C port the cable uses is a hint at most.
- **Derived module orientation:** each module's effective rotation (0, 90, 180 or 270°) is computed from the base orientation plus the module's slot and class (the slot table in 38 carries the per-class rotation). Controls, key index order, fader direction, roller direction, key artwork and the dial face all follow it.
- **Optional override per module:** default "Follow base"; a power user can set a fixed rotation for one module (for example faders running left to right, or the key grid turned a quarter). Stored in the profile or rig, shown in the inspector with a clear "Overridden" mark and a one-click reset.
- Rig preview, key and dial artwork previews, fader/roller direction labels and the generated images all use the same effective-orientation value, so editor and hardware never disagree.
- Check on hardware what the module reports for physical input in each rotation (key index order, roller direction, fader range); record anything the protocol does not tell us.

## Acceptance
- Vertical and horizontal base layouts show and behave correctly on real hardware, with modules in at least two slots.
- Overriding one module changes only that module's preview, artwork and input mapping; reset returns it to following the base.
- The effective-orientation function is unit-tested for all base, slot, class and override combinations; results in `docs/masterhub-hardware-results.md`.
