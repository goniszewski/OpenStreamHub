# OpenStreamHub focused-app profile rules

The desktop app (`apps/open-stream-hub`) can switch the profile of a running session when another app comes to the front. A rule maps an app bundle ID, such as `com.apple.Safari`, to a saved profile. The feature is **opt-in**: it stays off until the user enables it in the profile manager (the app header’s Manage profiles button, next to the profile switcher). The code is in `src-tauri/src/focus.rs` (storage, matching and the macOS watcher), `src-tauri/src/session.rs` (`Driver::switch_profile`), `src/focus-rules.ts` (frontend store) and `src/FocusRulesPanel.tsx` (UI). Nothing here has been tested with a real focus change or on hardware; see [What is not verified](#what-is-not-verified).

## Storage

- The rules are kept in `<app data>/focus-rules.json`, next to the `profiles/` folder. The profile schema is not changed and profiles never refer to rules.
- File shape: `{"enabled": false, "rules": [{"id": "<uuid>", "app_bundle_id": "com.apple.Safari", "profile_id": "<uuid>"}]}`. Unknown fields are rejected.
- A missing file means no rules and the feature off. Reading and writing reuse the profile store's helpers (`store::read_private_file`, `store::write_private_file`):
  - The read is bounded (64 KiB), never follows a symbolic link, and never opens a FIFO or device node.
  - The write goes to a private 0600 temporary file, which is flushed and renamed over the target, and then the folder is flushed.
  - A symbolic app data folder is refused for writing.
- A damaged, oversized or invalid file is reported as an error and is never overwritten, as with an unreadable profile. Fix or remove the file to recover.
- A crash during a save can leave a hidden `.<uuid>.tmp` file in the app data folder. When the rules are opened at startup, regular files with exactly that name shape (a dot, a UUID, `.tmp`) older than one hour are deleted. Other hidden `.tmp` files are left alone, because the app data folder also holds other files; recent ones, folders and symbolic links are left alone too. (The `profiles/` folder belongs to the profile store, which removes any stale hidden `.tmp` file there.)

### Validation

- At most 64 rules (`MAX_RULES`).
- The rule `id` is a canonical lowercase UUID, unique within the file. The backend creates it when a rule is added.
- `app_bundle_id` is trimmed, 1 to 255 characters long, uses only ASCII letters, digits, `.`, `-` and `_`, and starts with a letter or digit.
- `profile_id` is a canonical lowercase UUID. `save_focus_rule` also requires a readable saved profile with that id.

## Matching

`focus::match_rule(rules, bundle_id, profile_exists)` is a pure function:

- Nothing matches while `enabled` is false, or when the frontmost app has no bundle ID.
- Rules are evaluated in list order, and the **first** rule whose bundle ID equals the frontmost app's wins. The comparison ignores ASCII case. Bundle IDs must match exactly: there are no wildcards or prefixes.
- A rule whose profile no longer exists (deleted or unreadable) is skipped, so the next matching rule can apply. The UI marks such a rule as "Missing profile".
- When no rule matches, nothing changes. The session keeps its current profile.

## Watcher (macOS)

- At startup the app registers one observer for `NSWorkspaceDidActivateApplicationNotification` on the shared workspace's notification center. This is event driven, not polling. The notification is posted on the main thread, and the block reads `NSWorkspace.frontmostApplication.bundleIdentifier` and sends it to a `focus rules` worker thread. The current frontmost app is reported once at startup. Reading the app from the notification's `userInfo` (`NSWorkspaceApplicationKey`) instead would need the `NSDictionary` feature of `objc2-foundation`, which is not enabled, so it is not done.
- The worker skips OpenStreamHub's own bundle ID (the Tauri `identifier`), so bringing the app to the front never switches profiles. It records the last other app for **Use current app**, matches the rules, and emits `focus://changed`.
- Other platforms have no watcher. The rules can still be edited, but they never match, and `focus_current_app` returns `supported: false`.
- Dependencies: `objc2-app-kit` was already a direct macOS dependency (for the sleep/wake observers). Only its `NSRunningApplication` feature is newly enabled, and nothing new is downloaded.

## IPC

| Command | Arguments | Result |
| --- | --- | --- |
| `list_focus_rules` | none | `{enabled, rules}` |
| `save_focus_rule` | `rule: {id: null \| uuid, app_bundle_id, profile_id}` (a null id adds a rule, an existing id edits it) | `{enabled, rules}` |
| `delete_focus_rule` | `id` | `{enabled, rules}` |
| `set_focus_rules_enabled` | `enabled` | `{enabled, rules}` |
| `focus_current_app` | none | `{supported, bundle_id}`: the last app other than OpenStreamHub that was frontmost |
| `list_running_apps` | none | `[{bundle_id, name}]`: the ordinary apps (with a Dock icon) running now, sorted by name, without OpenStreamHub itself, at most 200; empty off macOS. It feeds the app picker and reads no app's contents |
| `switch_session_profile` | `sessionId`, `profileId` | the `StoredProfile` that the session now uses, plus `profile_generation`: the session's new generation |
| `session_profile` | `sessionId`, `profileGeneration` | `{profile_id, profile_generation, profile}`: the profile `resume_session` would restart, or the error `session::PROFILE_CHANGED` |
| `resume_session` | `sessionId`, `profileGeneration` | `null`, or the error `session::PROFILE_CHANGED` when the generation is no longer the session's |

Event `focus://changed`: `{bundle_id, rule_id, profile_id}`. `rule_id` and `profile_id` are null unless the rules are enabled and a rule matched an existing profile. Session events (`session://state`) carry `profile_generation` and the profile of that generation: `profile_id` (the saved profile a switch put in place, `null` while the session still runs the profile it was started with) and `profile_name`. Shared contract fixtures are in `apps/open-stream-hub/fixtures/focus-*.json`, `session-switch-events.json` and `session-command-*.json`. Regenerate them with `UPDATE_FIXTURES=1 cargo test`.

## Where the rules are edited and shown

- **The open profile's row.** The profile manager's current-profile section has "Switch automatically when an app is in front": the apps that switch to this profile as removable chips, and a picker of the running apps that have no rule yet (or a typed bundle ID, with "Use current app"). It edits the same rules as the panel below it, shares its on/off switch (a note with a Turn it on link appears while it is off), and asks to save a new profile first.
- **The header.** While automatic switching is on, an icon sits beside the connection icon; its tooltip says how many rules there are and which profile the app in front switches to (or that it has no rule). With a match the profile's name shows beside the icon. Clicking it opens the profile manager.
- There is no file browser for an app that is not running: type its bundle ID.

## Switching a session, and the safety rule

Focus changes are debounced in the frontend: a switch happens 400 ms (`focusDebounceMs`) after the last `focus://changed` event, and only for that event's match. Switching through several apps quickly (Cmd-Tab) therefore switches at most once, and ending on an app without a rule switches nothing. Saving, deleting or disabling a rule cancels a pending switch, which was matched against the old rules. Until that command's reply arrives, a debounced switch that fires in the meantime is skipped (the change may be slower than the debounce), and the reply cancels any switch scheduled while it was in flight. When the timer fires, a switch whose rule is no longer enabled or no longer maps to that profile is dropped as well (checked once the rules list has been loaded in the frontend).

When the latest event carries a `profile_id`, the frontend calls `switch_session_profile` for every live managed session (any phase except `stopped`) that is not already on, or switching to, that profile. A run started from an **unsaved draft** does not count as being on its saved profile, so a rule for that profile still switches it to the saved version. Unmanaged runs, which have no session, are not switched. The backend loads and validates the **saved** profile from storage, never the editor draft, and hands it to the session driver:

- `Driver::switch_profile` replaces the profile in the session's stored start request and applies `Event::ProfileSwitched`. If a process is running, the session stops it through the usual SIGINT path, so the service restores hardware mode, and records `reason: "profile_switch"`.
- **Dry run** (`execute: false`): once the old process has exited, the session restarts it immediately with the new profile, the same `software_mode` and the same device.
- **Executing session** (`execute: true`): the session follows the [session manager safety rule](masterhub-session-manager.md#safety-rule). After the old process exits, the session moves to `needs_confirmation` with reason `profile_switch`, and actions stop. Nothing executes with the new profile until the user confirms the existing Resume dialog (`resume_session`). The paused-run notice reads "Focused-app rule switched the profile — confirm to resume actions" (it names no profile, because a later switch can change it) and offers **Review**, which focuses the Resume button.
- A session without a live process (suspended, waiting for its device, already waiting for confirmation) only has its stored profile replaced. Its next start uses the new profile, under the same rules.
- A second switch while the first stop is still in progress only replaces the profile again. The restart uses the latest one.
- Switches of one session are serialized in the frontend (`ServiceRunner.switchProfile`): one request is in flight, and only the newest waits; an older waiting request is dropped. Replies therefore arrive in order, and the run always names the profile the backend holds.
- Switch requests queued for the driver at the same time (one focus change, several sessions) are handled together, and every process they interrupt is stopped in one parallel batch (`Runner::stop_many`, as for sleep), so N sessions take one stop grace period rather than N.
- Queued sleep/wake events are handled before any other queued input. A sleep never waits behind queued switches, and a dry run that a switch just stopped is not restarted only to be stopped again by the sleep; it restarts after wake with the new profile.
- Once a focused-app rule switched a run, the Resume text says that the switched saved profile resumes, even when a later pause (sleep, crash) is the reason for the confirmation.
- A [profile key](masterhub-actions.md#profile-nextprevious-profile) (`profile` action, next/previous) switches through the same `Driver` path with reason `profile_action`. The backend picks the saved profile itself (`Driver::profile_step`), so no `switch_session_profile` call is involved; the frontend resyncs the run from `session://state`. User-started executing sessions wait for confirmation; app-started sessions follow the automatic-actions setting and execution guard. Shared fixture: `apps/open-stream-hub/fixtures/session-profile-action-events.json`.

### Confirmation is tied to the profile shown

Every switch bumps the session's `profile_generation` (shown in `session://state` and returned by `switch_session_profile`). The run in the frontend remembers which generation its displayed profile name belongs to. `resume_session` takes that generation:

- The frontend sends the generation of the profile the Resume dialog showed. If a switch reply is still on its way, it waits for it first. If the session reports a newer generation than the profile name the run shows, it does not ask at all and reports that the profile changed.
- If a switch lands while the dialog is open, the frontend asks again, now naming the new profile, instead of sending the old confirmation.
- The run resyncs from every `session://state` event of its session: it adopts `profile_name`, `profile_generation` and (after a switch) `profile_id`, and never moves back to an older generation. A switch reply that timed out (the command waits at most 30 s for the driver) or a webview reload therefore cannot leave the run on a stale generation; the Resume dialog names the backend's profile for the backend's generation. The Resume dialog is bound to the session being resumed, not to whichever run the device selection shows while it is open. If that run disappears while the dialog is open, the dialog answers no and closes instead of showing start-a-run text. A state event carries no profile body, so when it adopts a newer switched profile the run's snapshot becomes unknown, and the "profile changed since start" notice is not shown for that run until a switch reply supplies the profile again.
- The backend compares the generation with the session's current one. On a mismatch it refuses with `PROFILE_CHANGED`, starts nothing, and emits the session's current state again; the frontend then asks again. A confirmation therefore never starts a profile the user did not see.

## What is not verified

All of the following was checked only with unit tests (fake runner, fake clock, simulated focus events):

- Whether `NSWorkspaceDidActivateApplicationNotification` fires for every app switch: Cmd-Tab, Dock clicks, Mission Control, full-screen spaces, and apps that activate without a window.
- Whether `frontmostApplication` already reports the newly activated app when the notification is delivered. Reading the app from the notification's `userInfo` would avoid that question, but needs a new `objc2-foundation` feature (`NSDictionary`), so it was not done.
- Whether 400 ms is the right debounce for real app switching.
- Behavior with several displays, Stage Manager, and helper processes that have their own bundle IDs, for example browser helpers.
- How quickly the stop-and-restart completes on real hardware, and how a module behaves when it is switched out of software mode and back in quick succession.
- That OpenStreamHub's own window reports the Tauri identifier as its bundle ID in a packaged build. In `tauri dev` the binary has no bundle, so its own activations report no bundle ID and are ignored anyway.
