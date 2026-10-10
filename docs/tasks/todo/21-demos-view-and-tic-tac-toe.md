# 21 · Demos view, with tic-tac-toe as the first demo

**Priority:** P3 · **Effort:** M–L · **Hardware needed:** yes (15-key module)
Related: [17](17-live-vs-test-mode.md) (modes — a demo is a state where no profile actions run).

## Why
Experiments that show what the modules can do, and that exercise the display and key-input path harder than profiles do. Throughput and latency are still unmeasured (`docs/masterhub-display-sync.md`: "redraw latency (logged, not timed)"; GIFs are clamped to a conservative 10 FPS), so the first demo must work at a very low frame rate.

## Scope
- **Demos view** in the app (sidebar entry beside Workspace / Display artwork / Compatibility): a gallery of demo cards (name, one-line description, which module it needs, "Needs the 15-key module" when none is connected). Browser preview shows the cards read-only.
- **Demo runtime** (the reusable part): an exclusive session on one module. It suspends the open profile's bindings and the profile display while it runs, draws its own key images through the existing key-image path (72×72 JPEG per key, diffed redraw), receives key presses as plain events, and **never executes any profile action**. Exit (Esc in the app, a dedicated exit key on the module, closing the view, unplugging, sleep) restores the profile and its display. Starting one asks nothing risky: no confirmation dialog, but the view says "Your profile's actions are paused while a demo runs."
- **Key position calibration** (needed once): only keys 0–2 have been confirmed to line up between display and input. The runtime asks the user to press the key lit up for each of a few positions (or a "light up key N" check) and stores the key-index → grid-position map, so every demo uses the real layout. Result recorded in `docs/masterhub-hardware-results.md`.
- **Tic-tac-toe** (first demo): 3×3 board on the left nine keys of the 5×3 grid (confirmed positions), X/O drawn as key images; the remaining keys: New game, Exit, who-goes-first, and a score. Play vs the computer (a perfect minimax opponent with an "easy" setting that sometimes errs) and two-player on the same module. Winning line highlighted; last-move highlight; works with only a few redraws per move (no animation needed).
- Pure game logic in a testable module (`demos/tictactoe.ts`), separate from rendering and device I/O; the board renderer shares the key-icon drawing code.

## Acceptance
- With a 15-key module connected, Demos → Tic-tac-toe plays a full game on the keys; win, draw and restart all work; exit restores the profile and its key artwork.
- While a demo runs, no key press runs a profile action or changes anything on the Mac (test: simulated presses with a muting profile loaded).
- Unplug / sleep / app quit during a demo ends it cleanly and restores the profile display.
- Unit tests for game logic (all win lines, draw, perfect-opponent never loses), runtime suspend/restore, and calibration mapping; QA captures of the Demos view (empty, module connected, demo running).
- Hardware check recorded: one full game, exit, profile restored.
