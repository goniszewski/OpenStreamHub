# 14 · Autosave with silent versioning

**Done · 9 October 2026 · committed on `master` as `54e2591` (task 06 of the suggested order still open; this one jumped ahead as the next P1)**

*Review: tsc clean, 743 vitest tests, 369 Tauri tests, fmt and clippy clean. Still worth a manual pass in the desktop app: edit, wait 1 s, relaunch, restore an earlier version.*

**Priority:** P1 · **Effort:** M · **Hardware needed:** no

## Why
Reviewers report the official app loses settings (APH: config wiped; Gaming Trend: changes vanish unless saved manually). We have a safe draft model, but "Unsaved changes", restore, discard and duplicate-version prompts are heavy.

## Scope
- Optional (default on) autosave of valid drafts to the profile file with a short debounce; invalid drafts stay as local drafts with the validation message.
- Keep a rolling history (e.g. last N versions) for restore; keep disk-conflict protection (`changedOnDisk`).
- Remove most unsaved prompts; keep confirmations for destructive actions.
- Running a session uses the last saved version, shown in the Run control (task 05).

## Acceptance
- Quit/crash after an edit restores the edit with no prompt in the normal case.
- Existing draft-store/profile-session tests migrated; new tests for debounce, invalid drafts, and conflicts.

## What was done
**Automatic saving (frontend, `ProfileSession`).** On by default, persisted in local storage (`open-stream-hub.autosave`), toggled in Settings > Profile storage ("Save changes automatically", desktop only). An edit to a *saved* profile schedules `save_profile` one second after the last change (`autosaveDelayMs`); success moves only the saved baseline — undo history, results and the open keypad folder survive, and the local crash-recovery copy is cleared as usual. An edit during an in-flight save is never lost: the stale response is skipped and the newer edit reschedules (a guard then still asks, since the draft is unsaved again).

**What is never autosaved.** A new profile has no file yet, so it keeps today's behaviour (recovery copy + restore offer). A draft that cannot be saved yet — a name being retyped, an empty key label — is gated by `profileIssue` (`draft-model.ts`, same rules the fields show) and simply stays local; the next edit re-checks. A save refused because the file changed on disk (`changedOnDisk`) shows the error once and stops (`autosaveBlocked`) instead of looping; reload or a successful save clears it. Manual Save and version restore hitting the same conflict also set the stop.

**Prompts.** `guard` (switch/reload/import/new profile) and `requestClose` now settle the pending edit first and proceed; only an edit that cannot be saved still gets the discard confirmation, and close-while-busy still waits. The duplicate-version prompt and the startup restore offer are unchanged (rare under autosave). The `changedOnDisk` fine print on the restore dialog now says "refuse until you reload" (true for autosave errors too). `closeDecision` was folded into `requestClose` and removed.

**Run control (task 05).** Starting a run settles the pending autosave first: `useRunSession` gained `beforeStart`, which App backs with `session.flushAutosave()` and returns the settled identity, so `start_service` carries `profile_id` and the run uses the saved version even when pressed within the debounce. The Run panel note now reads `Profile "X" (the saved version) was used…` / `(unsaved changes)` when it could not be settled.

**Version history (backend, `store.rs`).** Every content-changing overwrite snapshots the previous file into a hidden `<profiles>/.history/<id>/<modified_at_ms>.json` (original mtime preserved, atomic write, collision bumped, newest 20 kept, junk ignored; an identical rewrite writes no version; deleting the profile removes its history). New commands `profile_versions` and `load_profile_version` (registered in `generate_handler!`, build.rs, capabilities). Restoring is an ordinary load + `save_profile`, so validation, the imported-content flags and conflict protection apply, and the replaced version is kept first — restore is itself reversible.

**UI.** Profiles page: an "Earlier versions" panel under Validate/Reload (hook-free, fetches on every open, "Saved at <time>" rows with Restore; no extra confirm — the current version is kept in the list). Settings: the toggle plus rewritten storage copy. Docs: app README (saving flow, `.history` layout) and `masterhub-implementation-status.md`.

## Verification
- `npx tsc --noEmit` clean; `npx vitest run`: 743 tests pass (was 719). Migrated: guard/close tests (autosave off restores the old prompts), the two import-guard tests, the Settings SSR snapshot; new: autosave debounce/cancel/superseded/conflict-stop/busy-wait/new-profile+invalid skips, undo across an autosave, close-saves-then-destroys, `profileIssue`, VersionHistory component (list, empty, failure, toggle fetch, restore through the session), run-settling (`beforeStart` identity, no-result and failure fall back), `(the saved version)` note.
- Tauri workspace: `cargo fmt --check`, `cargo clippy --workspace --all-targets` (0 warnings), 369 tests pass — including new store tests: rolling history + reversible restore, cap at 20 + hidden from listings/seeding + removed with the profile, identical rewrite and refused save write no version. Root workspace fmt/clippy clean. Directory-count assertions now skip hidden entries (the `.history` folder is never profile data).
- Browser-preview capture: `qa/profiles-versions-dark.png` (panel open; devtools `details.open = true`, noted in `design-qa.md`). The version rows and the Settings toggle are desktop-only states, covered by tests, not captured (the running desktop app predates this build).
