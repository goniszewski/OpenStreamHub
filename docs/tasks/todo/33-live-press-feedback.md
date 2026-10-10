# 33 · Visible press feedback in Live mode

**Priority:** P3 · **Effort:** S · **Hardware needed:** yes
Related: [18](18-live-test-mode-model-and-header-badge.md) (Test-mode flash), [27](27-demo-display-speed-test.md).

## Why
Noisy Pixel liked that the official icons shrink on press: it shows the device is responding. We only plan a flash in Test mode.

## Scope
- Brief pressed-state key image (dim or shrink) in Live too, as one extra redraw on press and one on release, skipped when measured latency makes it feel late.
- Setting to turn it off; works with state-aware faces ([32](32-state-aware-keys.md)).

## Acceptance
- Press feedback visible on hardware without delaying the action; off switch persisted.
