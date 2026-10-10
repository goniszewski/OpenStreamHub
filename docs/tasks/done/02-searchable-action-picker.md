# 02 · Searchable, categorised action picker

**Priority:** P0 · **Effort:** M · **Hardware needed:** no
**Status:** Done · 9 October 2026 · commit 3580036; native drag check not yet performed

## Why
Binding an action means choosing from one flat `<select>` of ~20 mixed entries (`stepActionTypes` + Multi-action/Profile/Timer in `ActionFields.tsx`). The official app uses drag-and-drop from a module-filtered accordion, with no search. We can beat it with search plus click-to-assign.

## Scope
- New popover/side picker with groups: Audio, Keyboard, Apps & web, Media, System, OBS, Timers & sequences, Navigation (folders/profiles).
- Search box filters across groups; recent/frequent section.
- Filter by selected module (faders only offer position-driven actions, etc.).
- Click to assign; also drag from the picker onto a control on the module image/keypad tile.
- Keep keyboard accessibility (arrow keys, type-to-filter, Esc).
- Keep `ActionFields` parameter editors; only the type selection changes.

## Acceptance
- Any existing action can still be assigned and round-trips through validation.
- Fader module lists only valid actions.
- Component tests cover search, filter, click-assign, drag-assign.

## What was done
- **Picker model** (`action-picker-model.ts`): the catalogue with each entry's category, label (wording unchanged from the old options) and search keywords; `entriesForScope` (`binding`, `binding-fader`, `step`), `isValidForControl`, word-anywhere search across label/keywords/type, `pickerGroups` (display order, empty groups dropped), `defaultBindingAction` (the defaults the old select's onChange built, moved verbatim), the recent list (localStorage `open-stream-hub.recent-actions`, newest first, capped at 4, junk-tolerant) and the drag payload (`application/x-open-stream-hub-action`, with `offersActionDrag` for dragover, where data is unreadable).
- **Picker component** (`ActionPicker.tsx`): a trigger styled like the other inputs — showing the bound action's catalogue name or "Choose an action", keeping the historical ids (`binding-action`, `step-N-action`) so labels keep pointing at it — opening a popover with the search box (autofocused), the RECENT section (only while not searching, only entries valid in scope), the grouped catalogue, and, for a bound top-level picker, "Remove action" pinned at the bottom. The bound action is marked with a check and accent colour. Keyboard: type-to-filter, ArrowUp/ArrowDown cycle the entries (wrapping, first/last from the search box), Enter activates, Escape closes; an outside press closes. Fully controlled (`PickerState`), so it renders identically everywhere; `ActionPickerField` owns the open state. No assignments lost on close: search resets on reopen.
- **Wiring** (`ActionFields.tsx`): the top-level action select and every multi-action step select are now pickers (steps: `allowDrag={false}`, no Remove-action entry). Parameter editors, ids and hints untouched; `profile` cannot be chosen as a step (unchanged). The dial-functions editor (Profiles page) gets the same picker for its bindings.
- **Drag-assign**: picker entries drag onto the module image's controls (`HeroImage`, via shared `controlDropHandlers`) and the keypad tiles (`KeypadScreen`; never the reserved Back key). The drop selects the target and applies `dropProfileEdit` — the action's default parameters on the control's primary gesture, or on a key's press with the action-derived label (same as an inspector assignment, recent included). Refusals: module-invalid payloads (a fader accepts only the position action), folder keys and the Back key (existing messages surface as the edit error), the encoder while dial functions exist (message points to Profiles · Dial functions). Disabled editing accepts nothing.
- **Faders**: the picker offers exactly one entry (Output volume follows position); unbound trigger reads "Choose an action", bound shows the action name; the Remove-action entry unbinds. Copy follows the UI standards (no banned words; typographic quotes in the "No action matches “…”" message).

## Verification
- `npx tsc --noEmit` clean; `npm test` 690/690 pass (was 648). New suites: `action-picker-model.test.ts` (catalogue coverage/wording, scope filtering, grouping, word-anywhere search, defaults for every type, recents incl. damaged storage, drag payload round-trip and foreign-payload refusal), `ActionPicker.test.tsx` (closed trigger markup, grouped catalogue, scope filtering, current-action mark, search narrowing and miss message, recents ordering/scope, click-assign → assign+recent+close, unassign, no unbind in steps, drag payload present/absent, Escape, arrow cycling incl. wrapping, per-entry labels), `action-drop.test.tsx` (`dropProfileEdit` for flat/keypad/fader + folder/Back refusals, `controlDropHandlers` disabled/foreign/invalid cases, keypad tile drops incl. Back-key exclusion). `ActionFields.test.tsx` updated for trigger markup; the old option-list assertions moved into the model/component suites.
- Workspace Clippy clean (no Rust changes).
- Real-browser smoke check (dev preview): clicking an entry assigned it, closed the popover, updated the trigger label and recorded the recent entry; the Recent section then listed Timer, Open URL.
- Screenshots (browser preview at 1485×1087; "Desktop app required" banner expected; the read-only fieldset was re-enabled via devtools purely to capture the open popover): `qa/action-picker-open-dark.png`, `qa/action-picker-search-dark.png` ("obs"), `qa/action-picker-recent-dark.png`, `qa/action-picker-fader-dark.png` (single entry), `qa/action-picker-steps-dark.png` (multi-action steps with per-step triggers), `qa/action-picker-open-light.png`. All six passed an independent visual review; the steps capture was retaken after the known window-scroll gotcha cut the header. Dated section appended to `apps/open-stream-hub/design-qa.md`; README "Select any of the five module types" bullet now describes the picker and drag-assign.

## Design decisions worth reviewing
- **Recents are per-install, not per-profile**, stored in localStorage like the rig arrangement and theme — device-agnostic app preference, never part of a profile.
- **Steps got the picker too** ("only the type selection changes" read as everywhere a type is chosen); they are not draggable, since drag-assign targets a control, not a step slot.
- **The recent list shows recency only** (the task's "recent/frequent"); frequency ranking felt like it needed more usage data than four entries to matter.
- **The trigger shows the catalogue name, not `actionLabel`** (which would print "Open https://…"), keeping the type identity stable while parameters are edited below; the full detail stays in the editors and the bindings list.
- **Drops on a folder key surface the existing "Open the folder to configure its keys." error** rather than silently failing, matching how the same edit is refused in the inspector.

## Review follow-ups (9 October 2026)

All three review points addressed (the two scope notes — module filtering is fader-vs-rest, recents are recency-only — stand as documented above):

- **Focus returns to the trigger.** Every popover-initiated close — choosing an entry, Remove action, Escape, toggling the trigger — now focuses the trigger before closing (`focusTrigger`, guarded so node-env tests skip it). An outside press closes without refocusing: the user's click decides where focus belongs, and stealing it back would fight the user. Covered by tests with a stubbed `document` (assign, unassign, Escape, toggle-close).
- **Drops are announced, with Undo.** `ProfileSession` notices now carry an optional action (`noticeAction`; cleared automatically whenever a notice write brings no fresh one, so it can never linger stale), and the app's notice strip renders the action as a compact button between the text and the dismiss cross. A successful drop shows one of three one-liners in catalogue names, each with an **Undo** button that runs the session's one-step undo and dismisses the notice: "Replaced Timer with Open URL on Knob 2." / "Added Open URL to Knob 1." / "Added Open URL to Knob 3; the binding for serial S1 is unchanged." — the last one covers the exact-serial edge the review named: the drop still writes an any-serial binding (the control keeps working for every other device), and the notice now says so instead of silently adding a second binding. Any later edit clears the notice, as with other session notices. Tests: notice-action carry/clear semantics in `profile-session.test.ts`; wording for all three cases (flat and keypad) in `action-drop.test.tsx`.
- **Drag-assign on WKWebView.** `tauri dev` exits immediately on this machine because the running OpenStreamHub.app (built 13:02, before the picker) holds the `tauri-plugin-single-instance` lock — so the in-app drop could not be driven here without quitting that app. The platform question was verified instead with a minimal WKWebView harness (a real `.app` running this WebView engine, replicating the picker's exact pattern: `<button draggable>` writing a custom MIME payload, a drop target admitting via `dragover` + reading it on `drop`). A computer-use-driven OS drag fired `dragstart` → repeated `dragover` → `drop` and read back the payload ("GOT ok"), and a manual drag by Robert confirmed the same. So WKWebView ① fires the HTML5 DnD sequence from real mouse events and ② round-trips `dataTransfer` payloads — the mechanism drag-assign relies on. **Remaining manual check (needs the picker build):** quit the running OpenStreamHub.app, run `npm run desktop`, open a picker and drag one entry onto a knob and one onto a key; each should select the control, assign the default action and show the notice with Undo.

## Follow-up verification
- `npx tsc --noEmit` clean; `npm test` 697/697 pass (was 690: focus-return, notice-action and drop-notice suites added); workspace Clippy clean (no Rust changes).

