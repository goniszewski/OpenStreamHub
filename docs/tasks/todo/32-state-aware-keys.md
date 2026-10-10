# 32 · State-aware keys and dial display

**Priority:** P2 · **Effort:** L · **Hardware needed:** yes
Depends on [18](18-live-test-mode-model-and-header-badge.md) and on the measured redraw speed from [27](27-demo-display-speed-test.md).

## Why
The reviews describe static icons only. Keys that show live state are a differentiator and fix a known gap: the timer countdown is not drawn on the key. Tom's also found the dial screen too limited (clock, CPU/GPU, image).

## Scope
- Keys: OBS recording/streaming/mute/scene-active indicator, system mic mute, timer countdown. Each is an optional "show state" face on the matching action, with on/off artwork pair.
- Dial screen: active dial function name, OBS status, now playing.
- Redraw only on change and at the display path's sustainable rate; fall back to static artwork when OBS is offline.

## Acceptance
- Toggle recording in OBS and the key changes within a second; no redraw storms (diffing tested); timer face counts down on a key.
