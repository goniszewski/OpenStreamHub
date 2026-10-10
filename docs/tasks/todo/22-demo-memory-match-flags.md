# 22 · Demo: memory match with country flags

**Priority:** P3 (low) · **Effort:** S–M · **Hardware needed:** yes
Part of the demos set; needs the Demos view and runtime from [21](21-demos-view-and-tic-tac-toe.md).

## Scope
- 14 of the 15 keys are cards (7 pairs of country flags), the 15th is New game / Exit. A card shows a face-down back; a press flips it; two flipped cards stay up briefly, then match (stay face-up) or flip back.
- Flags are bundled assets, not fetched: a vetted subset of ~40 flags from an openly licensed set (e.g. `flag-icons`, MIT), prepared to 72×72 with the same pipeline as key artwork. Record the licence in the repo's third-party notices. Each game picks 7 random countries; flags with near-identical colours (e.g. Chad / Romania, Indonesia / Monaco) are never dealt together.
- Shows moves and best score (stored locally); the country name appears on the Demos view while a pair is matched (the module has no room for text).
- One redraw per press; no animation required, so it works at any frame rate.

## Acceptance
- Full game playable on the module; pairs, mismatch flip-back, win state and restart work; shuffle is unbiased.
- Pure logic (deal, flip, match, win) unit-tested; flag assets are 72×72, licence noted; QA capture of the board mid-game.
- Hardware check recorded.
