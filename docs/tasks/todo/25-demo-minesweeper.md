# 25 · Demo: minesweeper on the 5×3 grid

**Priority:** P3 (low) · **Effort:** M · **Hardware needed:** yes
Part of the demos set; needs [21](21-demos-view-and-tic-tac-toe.md).

## Scope
- 5×3 board (15 keys), 3 mines on "easy" and 4 on "normal"; the first press is always safe and never a mine (mines placed after it); a number key shows the adjacent-mine count; zero-count cells open their neighbours.
- A long press toggles a flag (use the existing press / long-press gesture thresholds); there is no spare key, so New game and Exit are on the module's other controls or the app, and the Demos view shows them. If key space is too tight, the view offers a "Flag mode" toggle on the app side instead.
- Win when every safe cell is open; lose on a mine, which reveals all mines.
- Because the board fills the whole module, Exit is by Esc or a button in the Demos view.

## Acceptance
- Win, lose, flag and first-press-safe behaviour are unit-tested on seeded boards, including flood-open; renderer QA capture shows numbers legible at 72×72.
- Hardware check recorded, including that long-press flagging is comfortable.
