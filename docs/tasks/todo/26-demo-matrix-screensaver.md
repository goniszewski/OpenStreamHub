# 26 · Demo: Matrix-style screensaver (digital rain)

**Priority:** P3 (low) · **Effort:** M · **Hardware needed:** yes
Part of the demos set; needs [21](21-demos-view-and-tic-tac-toe.md). Quality depends on the measured frame rate (task 27).

## Scope
- Falling green glyph columns drawn across the whole 5×3 grid as one 360×216 picture: render one frame, cut it into 15 key images, push only the keys whose pixels changed. Glyphs are from an open-licensed monospaced font or simple drawn shapes (the film's katakana style, without using its artwork).
- Frame rate adapts to what the display path sustains (target ≤ 10 FPS until measured otherwise); trails fade smoothly, columns advance in steps so a low frame rate still reads as motion.
- Runs until any key or Esc ends it; optional "start when the Mac is idle" is out of scope.
- Does not run profile actions (same exclusive session as the other demos).

## Acceptance
- A continuous run of at least 10 minutes on hardware with no failed writes and no visible tearing between keys; the achieved FPS is shown in the Demos view and recorded in `docs/masterhub-hardware-results.md`.
- The frame cutter and the changed-keys diff are unit-tested; QA capture of a frame.
