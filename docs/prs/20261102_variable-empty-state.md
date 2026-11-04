## Summary
- The If / Set variable editors no longer show a made-up "Is equal to: Off" when no variable exists, and link to the Variables section.
- All selects use a styled listbox dropdown instead of the unstyleable native popup.
- Each module shows "x of y assigned" with a popup listing every hardware trigger and its action.
- The Variables panel lists which bindings use a variable, and each one opens that binding in the workspace.
- "Switch automatically when an app is in front" and the focused-app rules panel add any installed app through the Finder dialog on Applications instead of a typed bundle ID.
- The Manage profiles button uses a layers icon instead of one that looked like "add folder".
- A new app key is labelled with the app's name, and that label is drawn under the app icon on the key.

## Changes
- Value fields stay hidden until a variable is chosen; "Add one under Variables" opens and focuses the Variables section.
- `SelectField` is now a select-only combobox (keyboard, type-ahead, ARIA) with the same `<option>` children API; all selects and tests migrated.
- `moduleAssignments` counts controls in use, folders included and the reserved Back key excluded on folder pages.
- `ModuleAssignmentsChip` popup closes on outside click, Escape, and module or keypad page change.
- "Used 1 time, by:" in the Variables panel now lists each trigger and its action; each trigger is a button that opens that binding in the workspace.
- The auto-switch row's last option is now "Another app from Applications…", which runs `choose_app` (via a new `FocusRulesStore.chooseApp`) and selects the result; the bundle ID field and Use current app button are gone from that row.
- The focused-app rules panel's bundle ID field is read-only and filled by a new Choose app… button (same dialog); Use current app stays.
- `SelectField` closes when it becomes disabled, ignores a stale highlight when its options change, and always gives the trigger its id.
- A new "Open app" key takes the app's name as its label (`app-names.ts` remembers names the picker has seen). The auto-drawn app artwork carries the key label as its text and is redrawn when the label changes; artwork you designed or uploaded, or text you typed in the studio, is never replaced.
- The Clear key button gets space above and below it in the inspector.
- Changing a key's app moves its label with it while the label is still the automatic one; the app drawing follows every label edit, not just the last.
- `SelectField` also closes on a press outside it (WebKit does not focus a clicked button) and keeps its highlight on the same choice when options change; the shared `useOutsidePress` hook serves it and the assignments popup.
- Labels that name an app redraw as soon as the app's name is learned (`app-names.ts` notifies the app root).
- Apps chosen in the Finder dialog keep their name in the auto-switch list even when they are not running.
- Artwork the app draws itself carries `auto: true` in its saved design (new optional field in the Rust `KeyArtworkDesign`); the studio's save writes `auto: false`, so text cleared in the studio is never redrawn. Artwork from earlier builds has no field and is still recognised by its content. Profiles with the field do not load in older builds (the design rejects unknown fields).
- The action editor (`ActionFields.tsx`) takes its text from the message catalogue too, and the converted-files guard covers it.
- The exported label maps (operations, gestures, connection states, session reasons, permissions, picker entries and others) take their text from the catalogue too, keeping their names and keys; `i18n-labels.test.ts` fails on an empty value or a raw message key. They are filled once at start-up, which suits the English-only app.
- A "Terminal and AI agents" starter: Accept, Interrupt, Cycle mode, Continue, /clear, /compact, Ctrl+C/D/L and tab keys, with the knobs on prompt history and scrolling. It is the one starter that sends keys and text, so `typingActionIssue` checks it and the dry-run-safe test leaves it out (a separate test limits it to shortcuts and text). After creating it, a notice offers "Switch to it when a terminal is in front": one focused-app rule for each installed terminal (Terminal, iTerm2, Ghostty, Warp, kitty, WezTerm, Alacritty, VS Code) via `FocusRulesStore.addForApps`, which keeps an app's existing rule and turns the rules on.
- The assignments popup, Variables panel and variable pickers take their text from the message catalogue. "Used by N triggers:" counts the triggers listed under it.
- Tests: new DOM tests (`jsdom` as a dev dependency, `dom-test.ts` harness) drive the select, the assignments popup and the Variables links; Rust tests cover the app picker's cancel and failure paths and fake app bundles.
- The header's Manage profiles button swaps lucide `FolderCog` for `Layers`; the folder-with-cog glyph read as "add folder" at 17px.

## Test plan
- [x] `tsc` and the full vitest suite pass (1414 tests) and `cargo test` for the Tauri app
- [ ] Open a dropdown near the bottom of the inspector and check it is not clipped
- [ ] Click and keyboard use of the dropdown and the assignments popup in the built app
- [ ] Click a trigger under "Used … by:" in the Variables panel and check it opens that binding
- [ ] In Manage profiles, "Another app from Applications…" opens Finder; the chosen app can be added, and cancelling changes nothing
- [ ] In the focused-app rules panel, Choose app… fills the bundle ID from Finder; add and edit a rule with it
- [ ] Pick an app for a new key: its name shows under the icon; edit Key label and the key redraws; Clear key has room around it
- [ ] Create the Terminal and AI agents starter, use the notice's button, and check the rules for your installed terminals appear and the profile switches when one is in front
- [ ] Add an If with no variables: no value field, link focuses Add variable

