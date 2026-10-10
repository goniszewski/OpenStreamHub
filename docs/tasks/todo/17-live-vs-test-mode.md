# 17 · Live / Test mode toggle replaces the per-run "Dry run / Execute actions" choice

**Priority:** P1 · **Effort:** M–L · **Hardware needed:** yes for the final check (press a real key in both modes)

## Why
Today the mode is a start-time choice on the Run panel (`ServicePanel.tsx`: "Dry run (default)" / "Execute actions"). It is stored per run and per module, is disabled while a run is active, does not follow the device dropdown, and is overridden by Settings → "Run actions automatically" for app-started runs. The toggle shows a pending choice, not the current state, so users cannot tell whether a press will act. Regular users who edit an action must also stop and restart the run to apply it ("Profile changed since start. Restart to apply.").

## Desired behaviour (from the product owner)
- One prominent **Live / Test** toggle. **Live** = every press executes its action. **Test** (dry run) = presses are logged only ("would run: …"), nothing happens on the Mac.
- **Live is the default** when not editing. Test is recommended while editing actions and for scripts / shell snippets / sequences.
- A setting such as **"Switch to Test mode while editing"**: while the user is changing a binding, presses are dry-run; when they stop editing the mode returns to Live.

## Scope
*Built as tasks 18 (mode model and badge), 19 (apply saved edits live) and 20 (Test while editing).*

- A single global mode with one status badge ("Live" / "Test") always visible in the header, replacing the Run panel toggle and the "Dry run / Executing" wording split across banner, button and panel.
- Applies to every running module, including runs the app starts itself. "Run actions automatically" in Settings is removed and folded into this (see Decisions, item 6).
- Editing rule: define "editing" precisely (proposed: the inspector has a selected binding with unsaved changes, or the action picker / bindings drawer is open) and make the auto-switch visible — the badge reads "Test (editing)" and says why.
- Switching mode applies to running sessions without a restart; saving an edited profile also applies it to the running session, keeping the current mode (removes "Restart to apply").
- Guardrails that stay: macOS-only execution, imported `.mhz` profiles stay Test-only until allowed (`dry_run_only`), the corrupt-import-list fallback, and the confirmation dialog that lists what a profile will do.

## Decisions (recommended, agreed 10 October 2026)
1. **Live is the default, behind a one-time confirmation.** First launch and every imported or unreviewed profile start in Test. The first time the user switches to Live (or a profile is first run live) the existing "what this profile will do" dialog appears once; after that Live is simply the default. Never silently live on first launch.
2. **One global mode**, not per profile or per module: one badge, one truth. Imported `.mhz` profiles are the exception and stay Test until allowed; the badge says "Test (imported profile)" there.
3. **"Editing" means:** a binding has unsaved changes in the inspector, or the action picker / bindings drawer is open. It ends on save, revert or closing the editor. While editing, presses on every module are held in Test (the mode is global), and the badge says so.
4. **Feedback in Test:** a pressed key's tile flashes briefly and shows "Test" in the Run log; the real key on the device behaves as normal. The log line reads "Would run: …".
5. **"Switch to Test mode while editing" is on by default** (it is the safe choice), with a plain-language description under it.
6. **Remove "Run actions automatically"** as a separate setting: it becomes the Live default plus the first-run confirmation. Existing users who had it on are Live, those who had it off are Test, until they change it once.

## Acceptance
- Exactly one place shows the current mode, and it always matches what a press will do.
- In Live a hardware press executes; in Test it only logs; switching needs no restart.
- With "Switch to Test mode while editing" on, opening a binding for edit flips to Test and shows why; closing/saving returns to the previous mode.
- Saving an edited profile applies to the running session without a restart or a mode re-pick.
- An imported profile cannot be put in Live until it is allowed; tests cover it.
- Updated onboarding / Settings copy, `docs/masterhub-actions.md`, and design-QA captures (header badge in Live, Test, Test-while-editing, imported-profile-locked).
- On-device check of both modes recorded in `docs/masterhub-hardware-results.md`.
