# 19 · Saving a profile applies to the running session

**Priority:** P1 · **Effort:** M · **Hardware needed:** yes for the final check
**Part of:** [17](17-live-vs-test-mode.md). Needs 18 (mode survives the swap).

## Why
Editing a profile while a run is active shows "Profile changed since start. Restart to apply." The user must stop, restart and re-pick the mode before a change takes effect.

## Scope
- Saving (including autosave, task 14) swaps the running session's profile in place, keeping the current mode and the device connection; no stop/start, no re-confirmation unless the swap adds actions the user has not yet confirmed in Live (then ask once, listing only what is new).
- Remove the stale banner for the saved-and-applied case; keep a clear note while there are unsaved changes ("Unsaved changes — not applied yet").
- Key artwork and dial content redraw from the new profile.

## Acceptance
- Change an action, save, press the key: the new action runs with no restart, in the same mode.
- A swap that fails (invalid profile) keeps the old session running and shows the reason.
- Tests for swap, failed swap and new-actions confirmation; on-device check recorded.
