# 24 · Demo: whack-a-mole / reaction test

**Priority:** P3 (low) · **Effort:** S · **Hardware needed:** yes
Part of the demos set; needs [21](21-demos-view-and-tic-tac-toe.md).

## Scope
- A mole image appears on a random key for a short time; pressing it scores; missing it or pressing an empty key costs a point. 30-second rounds, rising speed, best score kept locally.
- A second mode, "Reaction test": one key lights up after a random delay; the screen reports the press time in milliseconds over five rounds. This also shows the end-to-end latency users get from the hardware.
- Mole and hit/miss images are bundled 72×72 art made for the demo.

## Acceptance
- Both modes work; scoring and the random placement (never the same key twice in a row) are unit-tested; QA capture of a round in progress.
- Hardware check recorded, with the observed reaction-test spread.
