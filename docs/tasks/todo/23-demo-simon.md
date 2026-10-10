# 23 · Demo: Simon (repeat the sequence)

**Priority:** P3 (low) · **Effort:** S · **Hardware needed:** yes
Part of the demos set; needs [21](21-demos-view-and-tic-tac-toe.md).

## Scope
- Four to six coloured keys (a 2×3 block of the grid) flash in a growing random sequence; the player repeats it by pressing the keys. A wrong press ends the round and shows the score; best score kept locally.
- Three speeds (slow, normal, fast), limited by the measured redraw latency: the flash length never goes below what the module can show reliably (read from the display speed test once task 27 exists; until then a conservative default).
- No sound (the module has none); the Mac may play a soft tone through the app as an optional toggle, off by default.
- Reports the median press-to-redraw latency at the end as a diagnostic line.

## Acceptance
- Playable rounds to at least length 10; wrong press, restart and exit work; the sequence generator and checker are unit-tested.
- Hardware check recorded, including whether flashes look even at "fast".
