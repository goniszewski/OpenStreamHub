# 03 · Group OBS actions into one editor

**Done · 9 October 2026 · commits `483d545` and `e2dd77d` (unpushed)**

## Why
Eight separate "OBS …" entries clutter the action list. The official app groups them under one OBS category and fills pickers from the live app.

## Scope
- One "OBS" action category in the picker (task 02) with an operation dropdown (scene, virtual camera, replay buffer, studio mode, transition, filter, source visibility, scene collection, profile).
- Keep the underlying `obs_*` action types and schema unchanged; this is UI grouping only.
- Show a single inline "OBS not connected — Set up" status instead of the long per-action notes in `ActionFields.tsx`.

## Acceptance
- All existing `obs_*` bindings load and edit unchanged.
- One connection hint per OBS editor, linking to Settings → OBS.

## What was done
- New `ObsFields` editor in `ActionFields.tsx` handles all nine `obs_*` types: an Operation select (Scene, Virtual camera, Replay buffer, Studio mode, Studio transition, Filter, Source visibility, Scene collection, Profile) above the chosen operation's fields. Which of the nine picker entries the user chose, the editor is the same one — that is the grouping; the picker itself keeps its per-operation entries under the OBS category from task 02 (searching "filter" still lands on OBS filter).
- `switchObsOperation` in `action-model.ts` builds the new operation from its defaults and carries over the names both operations share (scene, source, filter, collection/profile name); operation settings (toggle/start/save) reset to the new default, so no invalid op can cross types (replay buffer's "save" never reaches the virtual camera). Field ids keep their historical names (`obs-scene`, `obs-filter-source`, `step-N-obs-reload-name`, …), so existing bindings load and edit unchanged.
- The nine repeated connection notes (`obsNeedsHint`/`obsNeedsMore` in every editor) are replaced by one inline status per OBS editor: "Connects to OBS 28 or later on this Mac while OBS runs with its WebSocket server on. Set up in Settings" + a short Learn more ("logged as unavailable"). The Set up button navigates to Settings, where the OBS Studio panel lives (`setPage('settings')`, plumbed through `Inspector` ← App and `DialFunctionsPanel` ← ProfilesPage; optional props, so existing call sites and tests are unaffected). Note the status is intentionally requirement wording, not a live "not connected" claim — the editor cannot see whether OBS is actually reachable; a live-driven status would belong with task 04's connection work.
- Per-operation behaviour notes stay but carry no connection story any more: toggle semantics, exact-name matching bounds, replay buffer needs to run before saving, and the scene-collection/profile reload warning (hint + Learn more, unchanged copy).
- `.obs-setup` styled as an inline accent link in `styles.css`.

## Verification
- `npx tsc --noEmit` clean; `npm test` 698 passed (was 697): new `obs operations` suite in `action-model.test.ts` (label coverage over the nine types, switch carry-over/reset rules), `ActionFields.test.tsx` OBS describes rewritten for the unified editor (Operation select with nine options, one status per editor, historical ids per operation, prefixed step ids, per-operation notes without the connection story, and an element-tree interaction check of the Operation select and the Set up button).
- Browser preview QA (1485×1087, dark, fieldset lifted via devtools for interaction): `qa/obs-editor-dark.png`, `qa/obs-editor-operation-dark.png`; clicking "Set up in Settings" lands on Settings with the OBS Studio panel. Entry appended to `design-qa.md`.
- Rust untouched (no service/schema change; real-OBS check still pending with task 04's hardware pass).

## Review follow-ups (9 October 2026)
1. **Single picker entry.** The nine OBS picker entries are collapsed into one "OBS" entry that assigns the Scene operation; the Operation dropdown chooses the rest, so the choice is made once. The trigger label and drop notices read "OBS" for every bound obs_* action (new `pickerEntryLabel` fallback and `pickerEntryIsCurrent` in `action-picker-model.ts`), the entry's keywords keep all nine operations searchable, recents store `obs` (legacy obs_* recents are dropped on read), and drags/drops carry `obs` and assign the scene default. Step assignment in multi-actions now goes through `defaultBindingAction` (the obs pseudo-type would have crashed `defaultStepAction`).
2. **Duplicate labels.** The per-operation on/off select is labelled "Action" in every operation (was Virtual camera / Replay buffer / Studio mode / Filter / Visibility).
3. **Dead Set up button.** `ObsConnection` renders the button only when an `onSetUpObs` handler is passed; the status text remains without one.
4. **Per-step repetition.** A multi-action with OBS steps shows the connection status once, after the step list (`MultiActionFields`), instead of once per OBS step; step editors show none of their own.

Follow-up verification: `npx tsc --noEmit` clean; 698 tests pass; re-captured `qa/action-picker-search-dark.png`, `qa/obs-editor-dark.png`, `qa/obs-editor-operation-dark.png` (the open/recent/steps/fader captures show no OBS content and needed no re-take); entry appended to `design-qa.md`.
