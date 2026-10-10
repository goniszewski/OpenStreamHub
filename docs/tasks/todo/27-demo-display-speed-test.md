# 27 · Demo: display speed test

**Priority:** P3 (low), but the other demos' tuning depends on it · **Effort:** S · **Hardware needed:** yes
Part of the demos set; needs [21](21-demos-view-and-tic-tac-toe.md).

## Scope
- Pushes full-grid frames (15 changing key images) as fast as the module accepts them for 30 seconds, and a second pass changing one key at a time. Reports sustained FPS, the 95th-percentile write time per key image and the dropped/failed writes, and the press-to-redraw latency (press a lit key, time the redraw).
- Results shown in the Demos view and copyable as text for `docs/masterhub-hardware-results.md`; Simon, the Matrix demo and any future fast demo read the result to pick their frame rate.

## Acceptance
- Numbers measured on a real 15-key module and recorded in the hardware results doc; the metrics code is unit-tested against a fake sink with known delays.
