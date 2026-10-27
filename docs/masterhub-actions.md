# MasterHUB action catalogue (macOS backend)

This page covers the actions added to close the cheapest gaps with the official MasterHUB 1.0.0.113 action catalogue ([app features](research/masterhub-1.0.0.113-app-features.md), "Actions" row of the [parity tracker](windows-parity-gaps.md)). It explains what each action does, which ones the macOS backend actually runs, and what limits the input. The base actions (volume, set volume, mute, ASCII shortcut, launch app) are described in [the service doc](masterhub-service.md#actions-and-permissions).

Several of these were checked on a real Mac on 20 to 22 October 2026 (media with Apple Music, the macro, the timer, the profile key, microphone volume, and on 22 October: open URL, launch app, text snippets, plain and modified shortcuts, undo, redo, screenshot, volume and mute; see the [results](masterhub-hardware-results.md#macos-actions-on-a-real-mac-m4315-22-october-2026)). The rest of this page was written before that and still says unverified in places; where it does, the results page is newer. The macOS behaviour described here comes from tests that run against a mock process host and assert the exact argv; no real process is started in tests. Treat every "executes" entry as **hardware- and permission-unverified**.

## Summary

| Action JSON | macOS backend | Needs |
| --- | --- | --- |
| `{"type":"open_url","url":"https://example.com"}` | Executes: `/usr/bin/open -u <url>` | Nothing extra |
| `{"type":"shortcut","key":"arrow_left","modifiers":["command"]}` | Executes: System Events `key code 123 using {command down}` | Accessibility, Automation |
| `{"type":"system","command":"undo"}` / `"redo"` | Executes: System Events `keystroke "z"` with Command / Command+Shift | Accessibility, Automation |
| `{"type":"system","command":"screenshot"}` | Executes: `/usr/sbin/screencapture -c` (whole screen to the clipboard) | Screen Recording |
| `{"type":"text","text":"Hello","press_enter":false}` | Executes: pasteboard plus Command-V (see below) | Accessibility, Automation |
| `{"type":"media","command":"play_pause"}` (also `next_track`, `previous_track`) | Executes: `/usr/bin/osascript -l JavaScript` (fixed script; after `--`, the key type and a `request`/`check` mode) posts a hardware media key, down then up, to whichever app owns media keys (see below) | Accessibility (posting events) |
| `{"type":"media","command":"stop"}` | **Best effort, two players only.** macOS has no stop media key: a fixed script stops Apple Music and pauses Spotify, each only if already running; with neither running it succeeds and does nothing | Automation (control of Music or Spotify) |
| `{"type":"conditional","when":{"obs_recording":true},"then":[…],"else":[…]}` (`else` optional; `when` is one of `obs_streaming`, `obs_recording`, `obs_virtual_camera`, `obs_replay_buffer`, or without OBS `output_muted` (bool), `output_volume_at_least` (percent) and `timer_running` (bool, timers of this module only) and `focused_app` (a bundle ID string, compared without regard to case, read from System Events so it needs the same Automation permission as the keystroke actions; to act in every other app swap the branches); or `{"all":[…]}` / `{"any":[…]}` of 2 to 8 of them, not nested or mixed) | Asks OBS the state of that output (or macOS for the output), then runs the `then` steps (state equals the value) or the `else` steps. Steps follow the `multi_action` rules (1 to 8 in `then`, 0 to 8 in `else`, no nesting) and share one 15 s deadline with the read. If OBS cannot be read, the press fails and neither branch runs. Needs an OBS endpoint only for an OBS condition or an OBS step. Unit-tested with a fake OBS and a mock process host only; hardware check skipped ([design](masterhub-flows.md)) | Whatever the steps need, plus OBS |
| `{"type":"set_variable","name":"takes","value":0}`, `{"type":"toggle_variable","name":"live"}`, `{"type":"add_to_variable","name":"takes","amount":1}` | Change a variable the profile declares in `variables` (`[{"name":"takes","value":0}]`; up to 16; a bool, a whole number within ±1 000 000, or text up to 64 bytes). Set replaces the value, toggle flips a bool, add moves a number and stops at the limits. The kind never changes, and an undeclared name or wrong kind is refused when the profile is validated. Values are kept across presses while the service runs and start again from the declared value when it starts; each module process has its own. They may be steps of a `multi_action` or of a conditional. Tested with fakes only. |
| `{"type":"multi_action","actions":[…]}` | Executes each step in order, 280 ms apart, within a 15 s overall deadline | Whatever the steps need |
| `{"type":"macro","steps":[…]}` | Executes: `/usr/bin/osascript -l JavaScript` (fixed script) replays the recorded keystrokes with their waits and holds as paired `CGEvent` down/up events; steps travel on stdin (see [Recorded macros](#recorded-macros-macro)) | Accessibility (posting events); recording also needs Input Monitoring |
| `{"type":"profile","direction":"next"}` (or `"previous"`) | **Never executed on the host.** The service reports a request; the OpenStreamHub session manager switches the run to another saved profile. Standalone runs log it as unavailable and do nothing | Nothing (no macOS permission) |
| `{"type":"timer","seconds":300,"label":"Tea","sound":"glass"}` | The service counts down; a second press of the same control cancels. At the end: `/usr/bin/osascript` (fixed script, label as an argv item) posts a notification, then `/usr/bin/afplay /System/Library/Sounds/<Name>.aiff` plays the sound | Notification permission for the process macOS attributes `osascript` to (unverified; a refusal that prints to stderr is logged as `timer_alert_warning`) |
| `{"type":"obs_scene","scene":"Live"}` | Executes when the run has an OBS endpoint: one loopback obs-websocket v5 connection, `SetCurrentProgramScene`, close. Without an endpoint it is logged as unavailable and does nothing | OBS 28+ with its WebSocket server on; OBS turned on in OpenStreamHub Settings (or `--obs-endpoint`) |
| `{"type":"obs_virtual_camera","op":"toggle"}` (also `start`, `stop`) | Same transport as `obs_scene`: `ToggleVirtualCam`, `StartVirtualCam` or `StopVirtualCam`, no request data. See [OBS outputs](#obs-outputs-obs_virtual_camera-and-obs_replay_buffer) | Same as `obs_scene` |
| `{"type":"obs_replay_buffer","op":"save"}` (also `toggle`, `start`, `stop`) | `ToggleReplayBuffer`, `StartReplayBuffer`, `StopReplayBuffer` or `SaveReplayBuffer`. `save` needs a running buffer | Same as `obs_scene` |
| `{"type":"obs_stream","op":"toggle"}` (also `start`, `stop`), optionally `"confirm":"press_twice"` | `ToggleStream`, `StartStream` or `StopStream`. Goes live or ends a broadcast; the Execute confirmation lists it and the profile counts as elevated risk. With `confirm` the first press only arms the key (logged as `action_armed`, with `confirm_within_ms` 3000) and a second press of the same binding within 3 seconds runs it; a third press arms again, and another key's press never confirms it. The state is per binding in the service process and is forgotten when the run stops or the Mac sleeps. Only a key's own action can carry `confirm`: a `multi_action` step with it is refused, as is `confirm` on any other action | Same as `obs_scene` |
| `{"type":"obs_record","op":"toggle"}` (also `start`, `stop`, `pause`, `resume`, `toggle_pause`) | `ToggleRecord`, `StartRecord`, `StopRecord`, `PauseRecord`, `ResumeRecord` or `ToggleRecordPause`. Pause and resume need a running recording | Same as `obs_scene` |
| `{"type":"obs_input_mute","input":"Mic","op":"toggle"}` (also `mute`, `unmute`) | `ToggleInputMute`, or `SetInputMute` with `inputMuted`. `input` follows the OBS name rules | Same as `obs_scene` |
| `{"type":"obs_adjust_input_volume","input":"Mic","step_db":-3}` | `GetInputVolume`, then `SetInputVolume` with `inputVolumeDb` = current + `step_db`, held between -100 and 0 dB. `step_db` is a nonzero integer from -20 to 20. (The official app's clamp bug is not copied.) | Same as `obs_scene` |
| `{"type":"obs_media_source","input":"Intro","op":"play_pause"}` (also `play_stop`, `play_restart`, `stop`) | `GetMediaInputStatus`, then one `TriggerMediaInputAction` chosen by the official state table ([OBS actions](research/masterhub-obs-actions.md#media-source-sub-modes-l1963-2105)) | Same as `obs_scene` |
| `{"type":"obs_screenshot"}` | `GetCurrentProgramScene`, then `SaveSourceScreenshot` (PNG) to `~/Pictures/OBS Screenshot YYYY-MM-DD HH.MM.SS.png` in local time. The place is fixed (the official app hard-codes a Windows Videos path); OBS writes the file, so it must be allowed to write to Pictures. The only OBS action that writes a file, so a profile with it counts as elevated risk. The path is never logged | Same as `obs_scene` |
| `{"type":"obs_studio_mode","op":"toggle"}` (also `on`, `off`) | `GetStudioModeEnabled` then `SetStudioModeEnabled` for `toggle`; `SetStudioModeEnabled` alone for `on` and `off`. See [OBS source actions](#obs-source-actions-obs_studio_mode-obs_transition-obs_filter-obs_source_visibility) | Same as `obs_scene` |
| `{"type":"obs_transition"}` | `TriggerStudioModeTransition`. OBS refuses it while studio mode is off | Same as `obs_scene` |
| `{"type":"obs_filter","source":"Webcam","filter":"Blur","op":"toggle"}` (`on`, `off`) | `GetSourceFilter` then `SetSourceFilterEnabled` (`toggle`), or the set alone | Same as `obs_scene` |
| `{"type":"obs_source_visibility","scene":"Live","source":"Camera","op":"off"}` (`toggle`, `on`) | `GetSceneItemId`, then `GetSceneItemEnabled` (`toggle` only), then `SetSceneItemEnabled` | Same as `obs_scene` |
| `{"type":"obs_scene_collection","name":"Streaming"}` | `SetCurrentSceneCollection` with `sceneCollectionName`. See [OBS scene collection and profile](#obs-scene-collection-and-profile-obs_scene_collection-obs_profile) | Same as `obs_scene` |
| `{"type":"obs_profile","name":"Work"}` | `SetCurrentProfile` with `profileName` | Same as `obs_scene` |
| `{"type":"set_mic_input_volume","percent":40}` | `osascript -e "set volume input volume 40"`: the default input device's volume (the microphone gain). See [Microphone volume](#microphone-volume-set_mic_input_volume-mic_input_volume_from_position) | macOS only |
| `{"type":"mic_input_volume_from_position"}` | Fader-only template; each position resolves into `set_mic_input_volume` | macOS only; fader bindings only |
| `{"type":"obs_set_input_volume","input":"Mic/Aux","percent":40}` | `SetInputVolume` with `inputName` and `inputVolumeDb` taken from OBS's slider scale at 40% (about -27 dB). See [OBS input volume](#obs-input-volume-obs_set_input_volume-obs_input_volume_from_position) | Same as `obs_scene` |
| `{"type":"obs_input_volume_from_position","input":"Mic/Aux"}` | Fader-only template; each position resolves into `obs_set_input_volume` | Same as `obs_scene`; fader bindings only |

Dry runs, replay and preview call the same `Action::validate_executable` as the backend, so a profile that previews cleanly passes the same checks that run before execution.

## Live and Test mode

The desktop app has one global mode, shown by one badge in the header, that decides what every press does on every running module:

- **Live**: a press runs its action on the Mac. This is the default.
- **Test**: a press is only logged ("Would run: …") and nothing happens on the Mac. The key on the module behaves as normal.

Switching needs no stop or restart by you. The service takes no messages once started, so the app restarts each affected run in place (same session and device, a brief redraw of the display, nothing in flight survives). Saving an edited profile applies it to the running session the same way and keeps the current mode.

Rules that keep Live safe:

- **First launch.** A fresh install reads Live, but nothing executes until you answer the "what this profile will do" dialog once, after the starter chooser. Confirming saves Live, declining saves Test. The answer is stored and not asked again. Until it is answered, runs stay in Test and say why.
- **While editing.** Presses are held in Test while a binding has unsaved changes or the action picker or bindings drawer is open. The badge reads "Test (editing)". Saving, undoing or closing the editor returns to the mode you chose. The hold is global, so it covers every module.
- **Imported profiles.** A `.mhz` import stays Test-only until you allow it. The badge reads "Test (imported profile)" and Live cannot be chosen for it.
- **macOS only.** Actions run only on macOS; on other systems Live is unavailable and Test logs what would happen.

The earlier "Dry run / Execute actions" choice on the Run panel and the "Run actions automatically" setting are gone. A settings file from before reads as Live if automatic runs were on, and Test if they were off.

## open_url

- Only lowercase `http://` or `https://`. Other schemes (`file:`, `javascript:`, `data:`, `ftp:`, custom schemes) are rejected.
- At most 2048 bytes, printable ASCII only. Allowed characters are the RFC 3986 unreserved and reserved sets plus `%`. Every `%` must start a two-hex-digit escape. Spaces, quotes (`"`), backslashes, `<>`, backticks, `{}|^`, control characters and non-ASCII are rejected, so percent-encode them. Internationalized hosts must be given in punycode.
- The host is required. It is either dot-separated labels of letters, digits and hyphens (no empty labels, no leading or trailing hyphen, at most 63 bytes per label) or a bracketed IPv6 literal. An optional port must be 1–65535. User information (`user:pass@`) is rejected, and so is any `@` in the authority.
- It runs `/usr/bin/open -u <url>`, with the URL passed as a single argv element and no shell. `-u` makes `open` treat the value as a URL even if it matches a local path. Since the URL must start with `http`, it can never be read as an option.

## Hotkey chords (`shortcut`)

`shortcut` stays backward compatible. `key` is either:

- one ASCII letter, digit or space, sent as `keystroke "<key>"` as before, or
- exactly one of these lowercase names, sent as `key code <n>`: `return`, `tab`, `space`, `delete` (backspace), `escape`, `forward_delete`, `home`, `end`, `page_up`, `page_down`, `arrow_left`, `arrow_right`, `arrow_down`, `arrow_up`, `f1`…`f20`, the keypad keys `keypad_0`…`keypad_9`, `keypad_decimal`, `keypad_multiply`, `keypad_plus`, `keypad_clear`, `keypad_divide`, `keypad_enter`, `keypad_minus`, `keypad_equals`, or
- one of the media keys `play_pause`, `next_track`, `previous_track`, posted as hardware media keys exactly like the `media` action (with `request` on the first key of a run, then `check`). They have no virtual key code and refuse every modifier, so a chord like Command+Play does not exist; `stop` is not a key.

Any subset of `command`, `control`, `option`, `shift` can be added, with no duplicates. Named keys are mapped to integers from a closed table (`NAMED_KEYS` in `profile.rs`, Carbon `kVK_*` values), so no user text is placed into the AppleScript source. Uppercase names, unknown names, punctuation and sequences are rejected. F13–F20 are the last function keys macOS gives virtual key codes (there are no `kVK_F21`–`kVK_F24` in `HIToolbox/Events.h`, so no key can send them). Function keys and media keys may be captured by macOS or by the keyboard's media-key setting before the focused app sees them. That has not been checked.

In the app the `shortcut` editor is a capture field ("Press keys…") plus a key chooser that lists every key above without pressing it: letters and digits, the named keys, F1–F20, the keypad and the media keys. A chord holds at most five keys (the four modifiers plus the key), which is the same limit the validator enforces.

## Recorded macros (`macro`)

`macro` replays a recorded sequence of keystrokes, each with its own wait and hold time. It is a binding-level action like `multi_action` and is never a step of one: a macro is already a timed sequence, so its replay keeps one deadline instead of nesting two.

```json
{"type":"macro","steps":[
  {"key":"c","modifiers":["command"],"delay_before_ms":0,"hold_ms":80},
  {"key":"arrow_left","modifiers":[],"delay_before_ms":120,"hold_ms":40}
]}
```

- `key` follows the same rules as a `shortcut` key without media keys: one ASCII letter, digit or space (an upper-case letter is the same physical key), or one allow-listed named key. Media keys cannot be macro keys — macOS consumes them before a recording can see them, and the `media` action sends them on its own.
- `modifiers` is any subset of the four, no duplicates.
- Bounds (`Action::MAX_*` in `profile.rs`, mirrored in the app editor): 1–64 steps; `delay_before_ms` 0–10,000; `hold_ms` 0–5,000; the waits and holds together at most 30,000 ms. Both the Rust validator and the editor enforce them.
- The activity log redacts a macro like a text snippet: only `{"type":"macro","steps":N,"ms":M}` is written, never the keys, because the recorded keys read like typed text. The Execute confirmation discloses every step in full, and a macro that types bare characters or presses a submit key (Return, Keypad Enter, Control+M/J/O) rates the profile high risk, like a snippet that pastes line breaks.

### Replay

One fixed `osascript -l JavaScript` run per macro (`MACRO_SCRIPT` in `crates/cooltek-sdk-service/src/macos.rs`):

1. It reads the step list from stdin as a JSON array of validated integers — the virtual key code, the CoreGraphics modifier flag word, the wait and the hold — never from `argv`. The list is bounded far below the 8 KiB stdin limit.
2. It re-checks every bound (including the field set of each step and the 30 s total) and preflights `CGPreflightPostEventAccess()`; on any refusal it throws a fixed `macro: …` error and posts nothing.
3. For each step it waits the recorded delay, posts the key down with `CGEventCreateKeyboardEvent` + `CGEventSetFlags`, waits the hold, and posts the key up. Every event is read back through `NSEvent.eventWithCGEvent` and must show the expected type and key code before it is posted.
4. The subprocess timeout is the macro's own duration plus 5 s, so a long macro neither blocks forever nor rushes its pauses; stopping a run kills the script between two events at the latest.

Failures name fixed causes (posting not allowed → the Accessibility guidance, event check failed → "this macOS did not accept the keyboard event layout"), never stderr text. A macro that fails part way may already have sent its earlier keys; nothing is retried. Letters, digits and named keys go out as fixed ANSI/`kVK_*` codes, so an alternative layout (Dvorak) prints different characters — the same caveat as every key-code path.

### Recording

The app records through a listen-only `CGEventTap` (`apps/open-stream-hub/src-tauri/src/recorder.rs`): key downs and ups with their modifier flags and millisecond times, capped at 60 s and 4096 events. The editor folds the raw events into steps (auto-repeat and modifier taps fold away, unmapped keys are dropped and counted, waits and holds beyond the limits are trimmed), shows the folded timeline for review, and only replaces the macro when the user accepts it. Recording needs the Input Monitoring permission and replay needs Accessibility; the recorder dialog preflights both through the same public `CGPreflight*EventAccess` functions the service scripts use, explains them, and deep-links to the matching System Settings pane — it never requests a prompt implicitly. The "Press keys…" capture field of the shortcut editor needs no permission at all: it reads the web page's own keyboard events while focused.

The service (a standalone `masterhub-service` run) can replay macros but has no recorder; recording is an OpenStreamHub editor feature.

## System actions

- `undo` and `redo` send Command-Z and Shift-Command-Z to the focused application. Each app decides what those keys do.
- `screenshot` runs `/usr/sbin/screencapture -c`. It captures the whole screen to the clipboard, which replaces the clipboard's contents, and plays the system shutter sound. Without Screen Recording permission, macOS may capture only the desktop background. The official app's other system actions (switch desktop, zoom, custom scaling, scroll) are not implemented.

## Media control (`media`)

`media` accepts `play_pause`, `next_track`, `previous_track` and `stop`, matching the official `media-track-control` options. It is allowed as a `multi_action` step: each command is one bounded subprocess, like a shortcut. Unit tests use a mock process host and the scripts are compiled. On 21 October 2026 the four commands were pressed from an M4315 on a real Mac with Apple Music, and the user reported that play/pause, next, previous and stop all worked ([results](masterhub-hardware-results.md#media-keys-and-media-stop-with-apple-music-m4315-21-october-2026)). Browsers and Spotify, and the permission prompts, are still unchecked (see "Unverified" below).

### play_pause, next_track, previous_track

These post the same events a keyboard's media keys produce, so they reach whatever app currently owns media-key handling (Music, Spotify, a browser tab, QuickTime and so on), exactly as pressing the key would. The backend runs `/usr/bin/osascript -l JavaScript` with a fixed script (`MEDIA_KEY_SCRIPT` in `crates/cooltek-sdk-service/src/macos.rs`), then `--` and two argv items: the key type `16` (`NX_KEYTYPE_PLAY`), `17` (`NX_KEYTYPE_NEXT`) or `18` (`NX_KEYTYPE_PREVIOUS`), mapped in Rust from the closed `MediaCommand` enum, and the mode `request` or `check`. The script accepts only those values. No profile text ever reaches the process.

The script:

1. Checks `CGPreflightPostEventAccess()`. If posting events is not allowed, it fails with `media-key: not allowed to post events`. Without this check `CGEventPost` would drop the events silently and the action would look successful. In `request` mode it first calls `CGRequestPostEventAccess()`, which asks macOS to show its permission prompt. The service passes `request` only with the first media key of a run (one `masterhub-service` process); every later media key passes `check`, so turning a knob bound to `next_track` while access is denied cannot stack prompts. After allowing access, later presses work without a restart; if you dismissed the prompt, allow the app in System Settings or restart the run to be asked again.
2. Builds a key-down and a key-up `NX_SYSDEFINED` event (type 14, subtype 8, `data1 = key type << 16 | state << 8`, state `0xA` down and `0xB` up, the same state in the flags). The usual recipe makes these with `NSEvent.otherEventWithType…` and posts its `CGEvent`, but inside `osascript` that `CGEvent` reference comes back unusable (reads as type 0; checked on macOS 27 without posting anything). So the events are built with the public CoreGraphics setters on field numbers that Apple does not document: 83 (subtype), 149 (`data1`) and 150 (`data2`). A Swift comparison found these events byte-for-byte identical to the `NSEvent`-built ones apart from an unused location field.
3. Reads both events back through `NSEvent.eventWithCGEvent` and requires type 14, subtype 8 and the expected `data1`. If a future macOS moves those fields, the script fails with `media-key: event check failed` and posts nothing.
4. Posts the key-down, then the key-up, with `CGEventPost` to the HID event tap.

A missing permission is reported as `macOS did not allow posting the media key. Allow the app that runs the service in System Settings > Privacy & Security > Accessibility, then try again`. Other failures report the exit status and the size of stderr, never its text. Like a shortcut, the first press may only bring up the permission prompt.

### stop (best effort, two players only)

macOS has no stop media key, and `play_pause` would toggle rather than stop. So `stop` does something narrower:

- It looks for running copies of Apple Music (`com.apple.Music`) and Spotify (`com.spotify.client`) with `NSRunningApplication`, which starts nothing.
- Immediately before each command it checks again that the process id still belongs to that player, and skips it otherwise.
- Music gets its own `stop` command (event `hook`/`Stop` from its scripting dictionary) as an Apple event addressed to its process id. Such an event cannot start a process: sent to one that has quit, it just fails. This was checked with TextEdit, which stayed closed. The error code of a failed send cannot be read safely in `osascript`, so a Music failure is reported as "no answer".
- Spotify gets `pause` through JavaScript for Automation (`Application(pid).pause()`), because Spotify's event codes could not be checked here (it is not installed on the development Mac). JXA may resolve that target by name, so if Spotify quits in the instant between the check and the command, macOS may relaunch it.
- In short: `stop` does not start a player that is not running. If a player quits in the instant between the check and the command, macOS may relaunch it (Spotify; Music's pid-addressed event cannot), which does not start playback.
- If both are running, both are stopped. If neither is running, the action succeeds and does nothing; the script prints `nothing to stop` (the service logs it as an ordinary `action_executed`, without that detail).
- Any other player (a browser, VLC, Podcasts, Apple TV and so on) is not affected.

Sending a command to Music or Spotify needs the Automation permission (System Settings > Privacy & Security > Automation) for the app that runs the service. A Spotify refusal (`-1743`) is reported with that guidance; a Music failure says it got no answer and may lack the permission. Each hint names only the player it belongs to, a player that failed does not stop the other one from being stopped, and a failure of a player that has meanwhile quit is not reported. While the permission prompt is open the script waits, so the first `stop` can hit the 5 s timeout. Press it again after allowing.

### Unverified

- That the posted events reach the media-key owner for Spotify and a browser. They reached Apple Music on 21 October 2026. Posting needs a GUI login session.
- Which process macOS asks about and holds responsible for the Accessibility and Automation permissions when `osascript` is started by the bundled sidecar, and whether `CGPreflightPostEventAccess` reflects that process.
- That Spotify's `pause` reaches the player. Music's `stop` worked on 21 October 2026; Spotify is not installed on the development Mac.
- Whether `CGRequestPostEventAccess` shows its prompt again on a later call after a denial, or only once per app (the service calls it at most once per run either way).
- What happens to a macOS Automation (or Accessibility) prompt when `osascript` is killed at the 5 s timeout: the prompt may stay on screen, and its answer then applies only to later presses.
- The undocumented field numbers have been checked on one macOS version only (27). The read-back check is there so a different layout fails instead of posting something else.

`media-fast-back-forward` and the official `play-pause` (play an audio file) are not implemented.

## Text snippet (`text`)

- `text` must be nonempty, at most 2048 UTF-8 bytes, and contain no control characters other than newline and tab. CR, ESC, NUL, BEL and C1 controls are rejected. `press_enter` is optional and defaults to `false`.
- `text` must also contain no invisible format or separator characters, because they would paste something other than what the reviewer saw. The exact rule (Rust: `is_invisible_text_character` in `profile.rs`, built on `is_format_character` in `dial.rs`) rejects a code point if it is a control character other than U+000A (newline) and U+0009 (tab), or if it falls in any of these ranges:

  | Code points | What they are |
  | --- | --- |
  | U+00AD | soft hyphen |
  | U+0600–U+0605, U+061C, U+06DD, U+070F, U+0890–U+0891, U+08E2 | Arabic/Syriac format marks, Arabic letter mark |
  | U+180E | Mongolian vowel separator |
  | U+200B–U+200F | zero-width space, ZWNJ, ZWJ, LRM, RLM |
  | U+2028, U+2029 | line and paragraph separators (Zl, Zp) |
  | U+202A–U+202E | bidi embeddings and overrides |
  | U+2060–U+206F | word joiner, invisible operators, bidi isolates, deprecated format controls (U+2065 is unassigned and rejected too) |
  | U+FEFF | byte-order mark / zero-width no-break space |
  | U+FFF9–U+FFFB | interlinear annotation controls |
  | U+110BD, U+110CD, U+13430–U+1343F, U+1BCA0–U+1BCA3, U+1D173–U+1D17A | Kaithi, Egyptian hieroglyph, shorthand and musical format controls |
  | U+E0001, U+E0020–U+E007F | language tag and tag characters |

  Everything else is allowed, including ordinary non-ASCII letters (`zażółć gęślą jaźń`), CJK, emoji, regional-indicator flags and variation selectors (U+FE0F). Emoji written as ZWJ sequences (such as 👨‍👩‍👧, which uses U+200D) are rejected. The TypeScript mirror (`textIssue` in `apps/open-stream-hub/src/action-model.ts`) must apply the same list. The shared fixture does not pin this rule yet.
- Execution, in order:
  1. `/usr/bin/osascript -e "clipboard info"` (fixed script) lists the clipboard's types.
  2. If every type is plain text (`«class utf8»`, `«class ut16»`, `string`, `Unicode text`), `/usr/bin/pbpaste -Prefer txt` reads it. The result is kept only if it is nonempty, valid UTF-8 and at most 8 KiB.
  3. `/usr/bin/pbcopy` receives the snippet **on stdin**. The snippet never appears in argv or in script text.
  4. `/usr/bin/osascript` runs a fixed script: `keystroke "v" using {command down}`, plus `key code 36` if `press_enter` is set.
  5. If step 2 kept the old text, the backend waits 500 ms, then reads the clipboard again with `/usr/bin/pbpaste -Prefer txt`.
  6. Only if that read succeeds and returns exactly the snippet, byte for byte, does it pipe the old text back through `pbcopy`. If anything else is on the clipboard (for example, you copied something during the 500 ms wait) or the read fails, nothing is restored, so a newer copy is never overwritten.
- Clobber risk: if the clipboard held anything other than small plain text (images, files, rich text, or more than 8 KiB), it is **not** saved. The snippet stays on the clipboard afterwards. A slow application could read the clipboard after the 500 ms restore and paste the old text instead of the snippet. If the paste step fails, the old text is restored straight away, subject to the same re-read check, and the paste error is reported. If restoring fails after a successful paste, the action reports an error that says the text was already pasted.
- Concealed and transient clipboard items: the clipboard is also not saved (so it is never restored) if any type name in the `clipboard info` output contains `nspasteboard`, `concealed` or `transient`, ignoring case. This covers markers such as `org.nspasteboard.ConcealedType`, `org.nspasteboard.TransientType` and `org.nspasteboard.AutoGeneratedType`. **Unverified:** nobody has checked on real macOS whether AppleScript's `clipboard info` lists these marker types at all. If it doesn't, a secret copied from a password manager shows up as plain text, is saved, and is put back on the clipboard after the paste, so it stays there longer than the password manager intended. Restored without its concealed marker, it can also be recorded by clipboard managers and clipboard history, and the password manager's auto-clear (which only clears the item it put there) no longer matches it, so it is not cleared. Don't trigger text snippets while a secret is on the clipboard. The real fix is to inspect the pasteboard types natively with `NSPasteboard` (not `clipboard info`) before saving anything; that is not implemented.
- Clipboard tools run with `LANG`/`LC_ALL=en_US.UTF-8` so non-ASCII text survives in an app started without a locale.
- Requires Accessibility and Automation permission for System Events.

## Multi-action (`multi_action`)

- 2 to 8 steps, matching the official flat `multi-tasking` list. A step can be any executable action except `multi_action` (no nesting), `volume_from_position` (a fader template that needs an input value), `profile` and `timer` (see their sections). `obs_input_volume_from_position` is a fader template too. Fader bindings accept only those two templates, so a multi-action cannot be bound to a fader.
- Every step is validated before the first one runs. Steps run in order with a fixed 280 ms pause between them, not before the first or after the last. The pause is an injected `Sleeper`, so tests never sleep.
- The first failing step stops the sequence. The error names it, for example `multi_action step 2 of 3 (system) failed: …`. Earlier steps have already taken effect and are not rolled back. Nothing is retried.
- Cancellation (`MacOsDispatcher::with_cancellation`) is checked before every subprocess, while monitoring each subprocess, and every 10 ms during the 280 ms pauses, so a cancelled sequence stops between or inside steps.
- Overall deadline: a whole multi-action gets 15 s (`MULTI_ACTION_DEADLINE`). The deadline is checked before each step starts, after the pause. Once it has passed, the sequence stops with `multi_action stopped before step N of M (<type>): the sequence exceeded its 15 s overall deadline; earlier steps already took effect`. A step that is already running is not interrupted. Its own subprocess timeouts bound it, except an `obs_scene` step, which gets only the time left before the deadline (at most its own 3 s) and fails without connecting if none is left. The clock comes from the injected `Sleeper`, so tests use virtual time.
- Worst-case time per step, with every subprocess hitting its 5 s timeout:

  | Step | Subprocesses | Worst case |
  | --- | --- | --- |
  | `text` | up to 6 (clipboard info, pbpaste, pbcopy, paste script, pbpaste re-read, restore pbcopy) | 30 s plus the 0.5 s restore wait = 30.5 s |
  | `obs_scene` | none (one loopback connection) | 3 s |
  | `volume`, `toggle_mute` | 2 (query, set) | 10 s |
  | Every other executable step | 1 | 5 s |

- Worst case for a whole multi-action: the last step can start just before the 15 s deadline and then run its own worst case (an `obs_scene` step never runs past the deadline), so a sequence ending in a `text` step can take about 15 + 30.5 ≈ 45.5 s. Without the deadline, 8 `text` steps would take 8 × 30.5 + 7 × 0.28 ≈ 246 s.
- The service runs actions one at a time, so input that arrives while a multi-action runs waits behind it. The live runtime queues up to 64 reports. After that, later reports are coalesced into one pending work item: relative volume steps are summed, absolute positions keep the latest value, and other events are appended up to a fixed bound and then dropped and counted (see `live_runtime.rs` and [the service doc](masterhub-service.md)). Presses made during a long sequence may run late, be merged with others, or never run.

## Profile next/previous (`profile`)

This matches the official app's `profile` key action, which has the options next and previous ([app features](research/masterhub-1.0.0.113-app-features.md), "Profile switching by a bound key"). Selecting a profile by name is not supported: the official action has no such option, and a name would be user text that has to be matched against a list the service does not have.

- JSON: `{"type":"profile","direction":"next"}` or `{"type":"profile","direction":"previous"}`. `direction` is required and comes from a closed set (`ProfileDirection` in `profile.rs`). Other values, other casing, extra fields such as `name`, and a missing `direction` are rejected.
- It can be bound anywhere a discrete action can: a flat binding, a keypad action tile, or a dial function binding. A fader cannot use it, because fader bindings accept only the fader templates (`volume_from_position`, `obs_input_volume_from_position`). It cannot be a `multi_action` step. The switch stops the running service, so later steps could never run.
- The service does not own the list of saved profiles, so it never switches anything itself. `process_work` intercepts the action before the dispatcher:
  - With `--profile-switch-requests`, the `event` record (status `planned`) is followed by `{"type":"profile_switch_requested","schema_version":1,"event":{…},"direction":"next"}`.
  - Without the flag (standalone `masterhub-service`, or `replay`), it is followed by `{"type":"action_unavailable","schema_version":1,"event":{…},"action":{"type":"profile","direction":"next"},"reason":"profile switching needs the OpenStreamHub session manager; no action was taken"}`. Neither outcome is an `action_failed`, and neither ends the session, even with `--stop-on-action-error`.
  - Neither counts toward `action_count` or `failed_action_count`. `service_end` adds `profile_request_count`, which counts both outcomes; `incomplete_action_count` leaves them out.
  - `MacOsDispatcher` refuses the action with "profile switching is handled by the OpenStreamHub session manager, not the macOS backend; no action was taken" and starts no process. `process_work` never sends it there, so this only guards direct callers.
- The desktop app passes `--profile-switch-requests` only to processes the session driver starts, and only when the profile binds a `profile` action somewhere (flat bindings, keypad tiles on any page, dial functions; `service::uses_profile_action`). Every other run gets exactly the arguments it had before, so an older sidecar without the flag keeps working for profiles that do not use the feature. The flag is decided again for each spawn, so a run switched to a profile with a profile key gets it on its restart.
- The desktop app forwards only a strictly parsed `profile_switch_requested` record (schema 1, a direction from the closed set) to the session driver (`session::profile_switch_request`, `Driver::profile_step`). Each spawn has its own forwarder (`session::SpawnForwarder`) that stops forwarding when that process reports its exit, under the same lock as the exit itself. A late record of a stopped process therefore never reaches the driver after its exit, and cannot switch the process that replaced it under the same service id. The record still appears in the run's activity log. The driver:
  - drops the request unless the Mac is awake and the session that owns that exact process is `running` with it. A process that is already stopping for an earlier switch is ignored, so several presses queued behind one switch switch only once.
  - drops a request within 500 ms (`PROFILE_STEP_DEBOUNCE_MS`) of the last one that passed the checks above, before reading any profile from storage. The window starts even when that request then finds no target, so a flood of presses reads storage at most once per window.
  - reads every readable, valid saved profile, including imported ones that were never opened (the store lock is taken for the listing and then per profile, never across the whole read), and orders them by name (ignoring case), then by exact name, then by id. Modification times are not used, so the order does not change when a profile is saved. With fewer than two saved profiles nothing happens.
  - finds the current position from the saved profile the session last switched to, or else the saved profile the run was started from (`start_service` sends its id as `profile_id` when the editor holds it unchanged), or, only when neither id is known or still saved, the first saved profile equal to the running snapshot. A run on an unsaved draft goes to the first profile for `next` and the last for `previous`. Both ends wrap around, and the target is never the current profile's id.
  - switches through the same path as a [focused-app rule](masterhub-focus-rules.md#switching-a-session-and-the-safety-rule): the profile generation goes up, a dry run restarts at once with the new profile, and an executing session moves to `needs_confirmation` and runs nothing until the user confirms it. The reason is `profile_action` instead of `profile_switch`. A dropped request changes nothing and is not reported beyond the service's own record.
- A driver start on a device the app cannot identify still runs, but without a session. If its profile binds a `profile` action, the process gets the flag and logs the request, and the driver ignores it, because no session owns that process. The activity log then shows "requested" and nothing switches.
- After a profile key switch the editor opens the switched saved profile too, so the module image matches the device (`follow-run-profile.ts`, wired in `App.tsx`). It does that only for key switches, only when the editor is not already on that profile, and never over unsaved changes: with those it keeps them and shows a note. Opening the app on a run that was switched earlier changes nothing. Checked on a real M4315 on 21 October 2026 ([results](masterhub-hardware-results.md#profile-key-and-the-editor-following-it-m4315-21-october-2026)).
- The request carries only the direction and the input event. Nothing in it needs redaction.
- The Execute confirmation lists "Switches profile" and does not raise the risk level, because nothing runs on the host. Executing runs always need a new confirmation after a profile key switch.
- A profile key switch carries no profile body to the frontend. Before the Resume dialog opens, the frontend calls `session_profile(sessionId, profileGeneration)` (allowed by `allow-session-profile` in `capabilities/main.json`; a test checks that every invoke handler is in `build.rs`, the capability and `permissions/autogenerated`). It returns the exact in-memory profile `resume_session` would restart (`{profile_id, profile_generation, profile}`), or refuses a stale generation with `PROFILE_CHANGED`. After a refusal the frontend waits up to 1 s for the session state of the newer generation, then asks for that one. The dialog lists the loaded copy's capabilities, snippets and sequences with the normal risk rules, names the switch as a profile key switch, shows the profile name with a short id (the first 8 characters; two profiles can share a name), and says that later edits to the saved profile do not apply until the next switch. If the profile cannot be loaded for any other reason, no Resume dialog opens: the run shows an error and the user can try again. A confirmation never covers contents the dialog could not show.
- The profile schema is unchanged (`schema_version` is unchanged). The new variant is additive, as the other actions here were. A build without it rejects a profile that uses it with an `unknown variant` error and never ignores the binding silently. Records stay at `schema_version` 1 because every field is additive.

## Countdown timer (`timer`)

This matches the official app's `timer` key action, a countdown with an end sound ([app features](research/masterhub-1.0.0.113-app-features.md), "Action: timer"). The official app also draws the remaining time on the key; this one does too, but only with the experimental display sync on; it was seen working on an M4315 (see "On the key" below). See [the parity tracker](windows-parity-gaps.md).

- JSON: `{"type":"timer","seconds":N}` with optional `"label"` and `"sound"`. `seconds` is a whole number from 1 to 86400 (24 hours). Fractions, strings, negative numbers and anything above the range are rejected. Unknown fields, and an explicit `null` for `label` or `sound`, are rejected too, so every accepted profile re-serializes byte for byte (`deny_unknown_fields`).
- `label` (optional) is the notification text. It follows the dial function name rule plus the `text` invisible-character rule: nonempty, no leading or trailing whitespace, at most 64 UTF-8 bytes, no control characters at all (so no newline or tab), and none of the invisible characters listed under `text` above (`validate_timer_label` in `profile.rs`; TypeScript mirror `timerLabelIssue`). Quotes, backticks, `$()`, `&` and every other printable character are allowed, because the label never becomes script or shell text (see below).
- `sound` (optional) is one of the closed set `basso`, `blow`, `bottle`, `frog`, `funk`, `glass`, `hero`, `morse`, `ping`, `pop`, `purr`, `sosumi`, `submarine`, `tink`, or `none` (`TimerSound` in `profile.rs`; the shared fixture lists them as `timer_sounds`). Each name maps in code to one fixed file, `/System/Library/Sounds/<Name>.aiff`; a profile can never name a path. Capitalised names, file names and paths are rejected. Absent means `glass`; `none` plays nothing.
- Where it can be bound: anywhere a discrete action can — a flat binding, a keypad action tile on any page, or a dial function binding. A fader cannot use it, because fader bindings accept only the fader templates (`volume_from_position`, `obs_input_volume_from_position`). It **cannot be a `multi_action` step** (`timer cannot be a multi_action step`): the toggle belongs to one binding, and a step has none. The official app refuses timers in multi-task lists too (`unsupportMutitaskingList`).
- Semantics. The countdown is owned by the running service process (the action worker in `live_runtime.rs`, using `TimerTable` in `crates/cooltek-sdk-service/src/timer.rs`):
  - The first press starts a countdown keyed by its binding: control, gesture, keypad page and dial function (`TimerKey`). The same control on another page or in another dial function is a different timer.
  - Pressing the same binding again while it runs **cancels** it (toggle). A press after it ended starts a new one.
  - **On the key.** With display sync on, a keypad key whose timer runs shows the time left (`M:SS`, or `H:MM:SS` from an hour) in orange over the bottom of its picture, redrawn each second (`timer_face.rs`: the worker publishes the running timers, the display thread draws them). The rest of the picture stays; the band takes the picture's own background colour, so a label placed at the bottom is replaced while the countdown runs. When the timer ends or is cancelled the key's own picture is drawn back. Digits are drawn with the built-in 5×7 font, so the numbers are blocky next to the app's labels. Seen working on an M4315 on 20 October 2026 ([results](masterhub-hardware-results.md#timer-countdown-on-the-key-checked-20-october-2026)).
  - At most **8** countdowns run at once. Another press fails with `action_failed` (`at most 8 timers can run at once; this timer was not started`); with `--stop-on-action-error` that ends the session like any other failed action. Cancelling still works while all 8 run.
  - Countdowns never survive the process. When the input ends normally (a duration or report limit, or the end of a replayed capture) each running one is logged as `timer_cancelled` with `"reason":"session_end"`. A stop (Ctrl-C, SIGTERM, the app's Stop, the session manager's stop before sleep, or an error) cancels each with `"reason":"stopped"`, and nothing expires or plays after the stop flag is set.
- Expiry never blocks input or actions. The input thread only reads, normalizes and queues reports; it never waits for a timer. The action worker checks deadlines between work items, waking at the next deadline (at most every 250 ms while a countdown runs). When one ends, the worker logs `timer_expired` and hands the alert to the session's **alert thread** (`AlertThread` in `live_runtime.rs`), then carries on with the next action at once. Playing a sound takes real time: `afplay` needs about 2.4 s of wall time for `Glass.aiff` and 2.9 s for `Funk.aiff` (the longest file, 2.2 s), so alerts must not run on the worker.
  - One alert thread per session, started at the first expiry, runs alerts one at a time in expiry order. At most 8 alerts can be pending (queued, running, or finished but not yet logged); another is logged as `timer_alert_failed` ("too many timer alerts are pending"). This can happen: alerts run one after another and each can take up to about 9 s, so if 8 timers end at nearly the same time, a short timer started meanwhile can expire while all 8 are still pending.
  - Its alerter is a second `MacOsDispatcher` with its own cancellation flag (`ActionDispatcher::timer_alerter`). The worker sets that flag as soon as it sees the session's stop flag (it checks at least every 20 ms while alerts are pending, and again while it waits at the end). A stop (Ctrl-C, SIGTERM, Stop, sleep, an error) therefore kills and reaps a running `osascript` or `afplay` (as for every action subprocess: SIGKILL, then up to 1 s to reap) and no queued alert starts afterwards. A cut-off alert is logged as `timer_alert_failed` ("the session stopped while the alert ran; it was cut off"), each skipped one as `timer_alert_failed` ("the session stopped before the alert ran").
  - If the alert thread cannot be started, every alert of that session is logged as `timer_alert_failed` ("could not start timer alert thread: …; nothing was shown or played") and the session goes on.
  - Only the action worker writes records: the alert thread sends each result back over a channel and the worker logs it (polling every 20 ms while alerts are pending), so lines never interleave.
  - At the end of a session the worker skips queued alerts ("the session ended before the alert ran"). After a stop it cuts a running alert off at once. After a normal end (a duration or report limit) it lets a running alert go on for at most 2 s (`ALERT_GRACE`), then cuts it off ("the session ended while the alert ran; it was cut off"). It then waits up to 3 s for the thread, logs every result and joins it. An alert therefore never turns a clean end into a drain error. Only a thread that ignores the cut-off is reported: the session ends with "timer alert thread did not stop … N alert(s) unreported; thread was not joined", and each unreported alert is logged as `timer_alert_failed` ("alert outcome unknown").
  - Dispatchers without a separate alerter (dry runs, replay, tests) run the alert inline, which is instant because nothing is spawned.
- The alert, in order, each in its own subprocess, never retried:
  1. `/usr/bin/osascript -e "on run argv" -e "display notification (item 2 of argv) with title (item 1 of argv)" -e "end run" -- "Timer finished" <message>`. The script lines are constants; `<message>` is the label, or `M:SS countdown ended` (`H:MM:SS` from an hour) without one. It is **one argv item after `--`**, read by AppleScript as data through `on run argv`, and is never interpolated into script text. The fixed title comes first, so even a message that starts with `-` cannot be read as an option. Bounded at 3 s (`TIMER_NOTIFICATION_TIMEOUT`).
  2. Unless the sound is `none`: `/usr/bin/afplay /System/Library/Sounds/<Name>.aiff` with that fixed path as the only argument. Bounded at 6 s (`TIMER_SOUND_TIMEOUT`), about twice the longest sound, so no sound in the closed set is cut short.

  Both are attempted even if the first fails. A failure is logged as `timer_alert_failed` (with the combined error) and counted, but it never ends the session, because it is not tied to a press.
- Notification permission. Whether `display notification` from `osascript` is shown depends on macOS notification settings for the process macOS attributes it to (Script Editor, or the app that started the service); this is **unverified**. `osascript` exits 0 even when the notification is refused (for example with `NSNotificationCenter connection invalid` on stderr). So when it exits successfully but prints anything on stderr, the service logs `timer_alert_warning` (`timer`, `action`, `warning`) instead of claiming success. The warning never contains stderr text, because a tool could echo the label back in some altered form (other case, whitespace, Unicode normalization, invisible characters) that no redaction reliably catches. It is one of two fixed categories: `notification service unavailable (NSNotificationCenter connection invalid)` when stderr contains that marker, otherwise `the notification tool wrote to stderr (N bytes)`. For the same reason, when `osascript` or `afplay` fails, the error gives only its exit status and the size of stderr (`stderr: N bytes, not shown`). A refusal that prints nothing cannot be detected. A warning is not a failure.
- Sleep. The desktop app's session manager stops the service through SIGINT before the Mac sleeps ([session manager](masterhub-session-manager.md#transitions)), so every countdown is cancelled (`stopped`) and nothing is spawned while asleep; a restart after wake starts with no timers. A standalone `masterhub-service run` that is not stopped keeps its process across sleep: its monotonic clock (`Instant`, `CLOCK_UPTIME_RAW` on macOS) does not advance while asleep, so countdowns pause and resume after wake rather than firing in the dark. That standalone case has not been checked on a Mac.
- Logging. The label is user text, so records never contain it: a timer action is logged as `{"type":"timer","seconds":N,"label_chars":K,"sound":"…"}` (`label_chars` and `sound` only when set), in the `event` record and in every timer record. Records (all `schema_version` 1, additive):
  - `timer_started`: `event`, `timer` (`control`, `gesture`, `page_id`, `dial_function`), `action`, `running_count`.
  - `timer_cancelled`: `event` (null when the session ended), `timer`, `action`, `reason` (`pressed_again`, `session_end`, `stopped`).
  - `timer_expired`: `timer`, `action`.
  - `timer_alert_failed`: `timer`, `action`, `error`.
  - `timer_alert_warning`: `timer`, `action`, `warning` (see Notification permission).
  - A press that starts or cancels counts in `action_count` (so it is never "incomplete"); `action_executed` is not written for it. `service_end` adds `timer_started_count`, `timer_cancelled_count`, `timer_expired_count`, `timer_alert_failed_count` and `timer_alert_warning_count`, only when a timer started, so runs without timers log exactly what they did before.
  - The app's run panel counts a timer press (`timer_started`, or `timer_cancelled` by a second press) as Executed in an executing run. Its Failed counter stays equal to `service_end.failed_action_count` (a timer press refused at the 8-timer limit is one); each `timer_alert_failed` is shown in a separate "Timer alerts failed" counter, which appears only when it is not zero.
  - Writing an expiry record can fail part-way through several due timers. Due timers are taken one at a time, so the one that failed and every later one stay in the table and are reported by the session's final cancellation.
- Dry runs and replay. `DryRunDispatcher` and the service's preview dispatcher validate and record the alert and start nothing. A dry run still counts down in real time and logs start, cancel and expiry, so a preview shows when a timer would end. `replay` uses the capture's own timestamps as the clock: a countdown expires before the first report at or after its deadline, and any still running at the end of the capture is cancelled with `session_end`.
- `MacOsDispatcher::dispatch` refuses a `timer` action itself ("timer countdowns are run by the service worker, not the macOS backend; no action was taken") and starts no process; only `timer_alert` runs the alert. The `ActionDispatcher::timer_alert` default refuses too, so a dispatcher that does not opt in never drops an alert silently.
- The Execute confirmation lists "Runs countdown timers" and does not raise the risk level: the effects are a notification and a fixed system sound.
- The profile schema is unchanged (`schema_version` is unchanged). The variant is additive, so a build without it rejects a profile that uses it with an `unknown variant` error. The `.mhz` importer still leaves official timers unbound.

## OBS outputs (`obs_virtual_camera` and `obs_replay_buffer`)

Two more OBS actions, from the [official list of 17](research/masterhub-obs-actions.md), on the transport described under `obs_scene` below (loopback only, endpoint and password from the app or `--obs-endpoint`, never from a profile). Unlike the official app, which only toggles, each has explicit `start` and `stop` too.

- JSON: `{"type":"obs_virtual_camera","op":"toggle"}` with `op` one of `toggle`, `start`, `stop`; `{"type":"obs_replay_buffer","op":"save"}` with `op` one of `toggle`, `start`, `stop`, `save`. `op` is required, lowercase, and a closed set; extra fields (such as `endpoint` or `password`) are rejected. Additive variants: no schema bump, and a build without them rejects such a profile with `unknown variant`.
- Requests: `ToggleVirtualCam`, `StartVirtualCam`, `StopVirtualCam`, `ToggleReplayBuffer`, `StartReplayBuffer`, `StopReplayBuffer`, `SaveReplayBuffer`. None carries request data, so no profile text reaches OBS.
- **Prefer `start` and `stop` in a `multi_action`.** A toggle depends on what OBS is doing now, and OBS refuses `start` on a running output and `stop` on a stopped one. `save` fails when the buffer is not running (the failure names the cause, for example "the output is not running").
- Both are allowed as `multi_action` steps and follow the same rules as `obs_scene` there: a run without an OBS endpoint reports the whole sequence as unavailable, and an OBS step gets only the time left before the 15 s deadline (at most 3 s).
- Logging: records show `{"type":"obs_virtual_camera","op":"toggle"}`. The op is a closed word, so unlike a scene name there is nothing to redact, and diagnostics bundles keep it.
- The Execute confirmation lists "Controls OBS outputs through the OBS WebSocket on this Mac" with each operation. It does not raise the risk level: neither output publishes to an audience by itself. (Recording and streaming will differ; they are not implemented.)
- Tested against the scripted fake obs-websocket server and, as reported by the owner, on a real OBS 5.7.4 (19 October 2026; [results](masterhub-hardware-results.md#obs-studio-with-a-real-obs-19-october-2026)).

## OBS source actions (`obs_studio_mode`, `obs_transition`, `obs_filter`, `obs_source_visibility`)

The next four of the [official 17](research/masterhub-obs-actions.md), on the same transport. These are the first actions that need OBS's answer before they know what to send, so each runs as one session: one connection, one handshake, one 3 s deadline for all of its requests (`ObsClient::run`).

- JSON: `{"type":"obs_studio_mode","op":"toggle"}`; `{"type":"obs_transition"}`; `{"type":"obs_filter","source":"Webcam","filter":"Blur","op":"off"}`; `{"type":"obs_source_visibility","scene":"Live","source":"Camera","op":"on"}`. `op` is `toggle`, `on` or `off` (a closed set); extra fields are rejected. `obs_transition` takes no fields. Additive variants, no schema bump.
- Names (`source`, `filter`, `scene`) follow the `obs_scene` rules (nonempty, at most 128 bytes, no control or invisible characters) and are sent only as JSON string values, never formatted into text. OBS matches them exactly, case and spaces included. A group can be named as the `scene`; a source inside a nested group needs the group's name instead.
- `toggle` reads the current state and writes the opposite, so a second controller changing the same thing between the two requests can make it end up unchanged. `on` and `off` do not read (the visibility action still looks up the item id first). **Prefer `on` and `off` in a `multi_action`.**
- `obs_source_visibility` first asks OBS for the numeric scene item id (`GetSceneItemId`), so it acts on the first item with that source in that scene. The official app stores the id from its picker; we address by name.
- `obs_transition` is the official `transition` action done as intended. The official code always sends the trigger even outside studio mode, because of a comparison bug; ours sends the same request and OBS's refusal is reported.
- Failures: a missing scene, source or filter reports "no such scene, or no such source in it" or "no such source, or no such filter on it" (status 600); a transition outside studio mode reports "studio mode is not active" (506). A failed lookup ends the action before any write.
- **Logging and redaction.** Records and the activity panel show lengths only: `{"type":"obs_filter","source_chars":11,"filter_chars":4,"op":"off"}`. Diagnostics bundles with redaction on replace `source`, `filter` and `scene` of these actions with `<N chars>`, and scrub short names from free text like scene names. **OBS's own comment text is never shown for any request that carried a name** (OBS repeats the name it was given); only the reason above is. This also applies to `obs_scene`, whose error text no longer includes OBS's comment.
- The Execute confirmation lists studio mode and transition as OBS outputs, and each filter and source change with its names (invisible characters made visible). It stays at normal risk: these change what OBS shows, like a scene switch.
- Studio mode and the transition were also reported working on a real OBS 5.7.4 (19 October 2026; [results](masterhub-hardware-results.md#obs-studio-with-a-real-obs-19-october-2026)); filters and source visibility were reported working on the same OBS on 21 October 2026 ([results](masterhub-hardware-results.md#the-remaining-obs-actions-with-a-real-obs-21-october-2026)).

## OBS scene collection and profile (`obs_scene_collection`, `obs_profile`)

The official app's `scene-collection` and `obs-profile` actions ([action list](research/masterhub-obs-actions.md)), on the same transport.

- JSON: `{"type":"obs_scene_collection","name":"Streaming"}` and `{"type":"obs_profile","name":"Work"}`. `name` follows the `obs_scene` rules (nonempty, at most 128 bytes, no control or invisible characters) and is sent only as a JSON string value. OBS matches it exactly. Extra fields are rejected. Additive variants, no schema bump.
- One request each (`SetCurrentSceneCollection`, `SetCurrentProfile`) on one connection. OBS **reloads** its scenes or settings and applies the switch a moment after it answers; the service does not wait for or poll that. Switching can interrupt a live stream or recording, and the editor and the Execute confirmation say so. Both are allowed as `multi_action` steps, but later OBS steps in the same sequence may reach OBS while it is still reloading.
- Failures: a missing name reports "no scene collection with that name" or "no profile with that name" (status 600). OBS's own comment text is never shown (the request carried a name).
- **Logging and redaction.** Records show `{"type":"obs_profile","name_chars":4}`; the activity panel shows `OBS profile (4 characters)`. Diagnostics bundles with redaction replace `name` with `<N chars>`.
- The Execute confirmation lists each switch with its name (invisible characters made visible) and a reload note. It raises the profile to elevated risk, because a reload can interrupt a live output; the editor also says to put a reload last in a multi-action.
- Tested against the scripted fake obs-websocket server and, as reported by the user, on a real OBS 5.7.4 on 21 October 2026 ([results](masterhub-hardware-results.md#the-remaining-obs-actions-with-a-real-obs-21-october-2026)). The detail of whether OBS answers before or after the reload completes was not written down.

## OBS input volume (`obs_set_input_volume`, `obs_input_volume_from_position`)

- JSON: `{"type":"obs_set_input_volume","input":"Mic/Aux","percent":40}` and the fader template `{"type":"obs_input_volume_from_position","input":"Mic/Aux"}`. `input` follows the `obs_scene` name rules (nonempty, at most 128 bytes, no control or invisible characters) and is sent only as a JSON string value. `percent` is 0 to 100. Extra fields are rejected. Additive variants, no schema bump.
- A run sends one `SetInputVolume` request: `inputVolumeMul` 0 for 0 (silent), otherwise `inputVolumeDb` = `-102 * 17^(-percent/100) + 6`, which is the curve of OBS's own mixer slider (`OBS_FADER_LOG`: about -96 dB at the bottom, -19 dB in the middle, 0 dB at 100). It never boosts above 0 dB, and a physical fader moves together with OBS's on-screen slider. (An earlier build sent a linear multiplier, `percent / 100`, which put the middle at -6 dB and felt uneven against OBS's slider.) An input name OBS does not know fails with OBS's "no input with that name" status; OBS's comment text is never shown.
- **Fader template.** `obs_input_volume_from_position` can only be bound to a fader (module `m005s`), and a fader binding can only be one of the two fader templates. A fader position resolves to the nearest whole percentage and then into `obs_set_input_volume`, the same way `volume_from_position` resolves into `set_volume`. The template cannot run unresolved and cannot be a `multi_action` step; `obs_set_input_volume` can be a step and can be bound to any other control.
- **Logging and redaction.** Records show `{"type":"obs_set_input_volume","input_chars":3,"percent":40}`; the activity panel shows the level and the name's length. Diagnostics bundles with redaction replace `input` with `<N chars>`.
- The Execute confirmation lists the input by name with the level, or says the volume follows the fader. It is a local OBS effect and does not raise the risk level.
- Tested against the scripted fake obs-websocket server; on a real OBS 5.7.4 the owner confirmed the fader drives an input's volume as expected (19 October 2026), while its range and jitter on hardware are unrecorded. Each position change sends one request, but a backlog of queued movements on the same input is coalesced to the latest level, like `set_volume`. There is no deadband yet.

## Microphone volume (`set_mic_input_volume`, `mic_input_volume_from_position`)

- JSON: `{"type":"set_mic_input_volume","percent":40}` and the fader template `{"type":"mic_input_volume_from_position"}`. `percent` is 0 to 100; extra fields are rejected. Additive variants, no schema bump.
- **What it does.** Sets the default input device's volume with `set volume input volume N` (the same `osascript` path as `set_volume`, one bounded subprocess, no query). Output volume and mute are untouched, and unlike the output volume the app does not snapshot or restore the input volume.
- **Fader template.** Like `volume_from_position`: bindable only to a fader (module `m005s`), a fader binding can only be one of the fader templates, a position resolves to the nearest whole percentage, and the template cannot run unresolved or be a `multi_action` step. `set_mic_input_volume` itself can be a step or bound elsewhere in a hand-written profile; the editor offers only the fader template.
- **Limit.** macOS ignores the call for an input device without software gain (many USB and some built-in microphones); nothing is reported then. Not yet checked on hardware.

## OBS connection test

Settings › OBS Studio has a **Test connection** button and a status chip, refreshed every ten seconds while the panel is open and OBS is switched on. The test opens one short loopback connection, goes as far as the `Identified` reply and closes; it sends no OBS request and shows no text that OBS sent. The chip is one of:

| Chip | Meaning |
| --- | --- |
| Connected | Handshake and password accepted. |
| Not running | Nothing answers and no OBS process is running. |
| WebSocket off | Nothing answers but OBS is running: its WebSocket server is off or on another port. |
| Wrong port | Something answers that is not an obs-websocket v5 server. |
| Password needed | OBS asks for a password and none is set in the app. |
| Wrong password | OBS rejected the password. |
| Not connected | OBS answered but the handshake failed another way (OBS older than 28). |

Each state shows a one-line fix. OBS being a running process is found with `pgrep`; a failed lookup counts as not running. The password is kept in the macOS Keychain.

## OBS scene (`obs_scene`)

This is the first of the official app's OBS integrations ([app features](research/masterhub-1.0.0.113-app-features.md), "Integrations: OBS", about 18 actions over obs-websocket). Only switching the program scene is implemented; the rest are listed under [Not implemented](#not-implemented).

- JSON: `{"type":"obs_scene","scene":"Starting soon"}`. `scene` is required and follows the keypad label rules plus the text snippet's invisible-character rule (`Action::validate` in `profile.rs`, `obsSceneIssue` in `action-model.ts`): not empty or only spaces, at most 128 UTF-8 bytes (`Action::MAX_OBS_SCENE_BYTES`), no control characters, and none of the invisible format or separator characters listed for `text` above. The name is matched by OBS exactly (case and spaces included). Extra fields such as `endpoint` or `password` are rejected (`deny_unknown_fields`), so a profile can never name where it connects. The variant is additive: no schema bump, records stay at `schema_version` 1, and a build without it rejects such a profile with `unknown variant`.
- It can be bound wherever a discrete action can: flat bindings, keypad action tiles and dial function bindings. Faders accept only the fader templates (`volume_from_position`, `mic_input_volume_from_position`, `obs_input_volume_from_position`).
- **It is allowed as a `multi_action` step** (a decision to review): it is one bounded request of at most 3 s, it does not stop the service, and it fits the 15 s sequence deadline. A sequence such as "switch to Be right back, then mute" matches the official app's flat multi-action list. When the run has no OBS endpoint, a `multi_action` with an `obs_scene` step is reported as unavailable **as a whole** and none of its steps run, so a sequence is never half-run because OBS is not configured.

### What a run does

- The endpoint never comes from a profile. `masterhub-service run` takes `--obs-endpoint 127.0.0.1:PORT` (also `[::1]:PORT` or `localhost:PORT`; nothing else is accepted, not even `127.0.0.2`, `0.0.0.0`, a host name or a scheme). The desktop app passes it only for session-managed runs, and only when OBS is turned on in Settings (see below).
- Without `--obs-endpoint`, an `obs_scene` action (or a `multi_action` with one) is never dispatched. After its `event` record the service writes `{"type":"action_unavailable","schema_version":1,"event":{…},"action":{"type":"obs_scene","scene_chars":4},"reason":"OBS is not configured for this run (turn on OBS in OpenStreamHub Settings, or pass --obs-endpoint); no action was taken"}`. It is not an `action_failed`, it never ends the session (not even with `--stop-on-action-error`), and `service_end` counts it in the additive `unavailable_action_count` (not in `action_count`, `failed_action_count` or `incomplete_action_count`). Replay never has an endpoint, so replay reports it the same way.
- Logging. A scene name is user text, so records never contain it: an `obs_scene` action is logged as `{"type":"obs_scene","scene_chars":N}` (N Unicode scalar values) in every record, and the app's activity panel shows `OBS scene (N characters)`.
- A dry run with an endpoint plans the action (`planned`) and never connects. Only `--execute` connects.
- With `--execute`, the action runs on the service's action worker, like every other action, so it never blocks input reading. Each action is one connection: TCP connect, WebSocket upgrade (`Sec-WebSocket-Protocol: obswebsocket.json`), `Hello`, `Identify` with `rpcVersion` 1 and `eventSubscriptions` 0 (no events are ever requested), the challenge answer when OBS asks for authentication, `Identified`, one `SetCurrentProgramScene` request whose `requestData` is `{"sceneName":…}` built by `serde_json` (the name is never formatted into a string), its `RequestResponse` with the matching `requestId`, then a close frame. Nothing is cached, retried or kept open.
- One deadline of 3 s (`obs::DEFAULT_TIMEOUT`) covers connect, upgrade, handshake and response. Cancellation (Ctrl-C, SIGTERM, stop) is checked before connecting and every 50 ms while waiting for OBS.
- Errors come back as `action_failed` with the endpoint and a reason: OBS not running (connection refused), timed out, wrong password (OBS closes with code 4009), no such scene (request status 600), protocol violations. A request comment from OBS is shown only for requests that carry no data (it is cleaned of control and invisible characters and cut to 120 characters); for any request that carried a name it is dropped, because OBS repeats the name. Close reasons are never shown, only their codes.

### Protocol limits (`crates/cooltek-sdk-service/src/obs.rs`)

The WebSocket client is hand-written over `std::net::TcpStream` (no WebSocket library, no TLS, no async runtime):

- The upgrade response must be `HTTP/1.1 101` with `Upgrade: websocket`, a `Connection` header containing `upgrade`, the correct `Sec-WebSocket-Accept` (SHA-1 of the random key and the RFC 6455 GUID), no extensions, and either no subprotocol or `obswebsocket.json`. Any other status, including a redirect, fails; redirects are never followed. Headers are limited to 8 KiB, and a repeated header fails.
- Server frames must be final (fragmented messages and continuation frames fail), unmasked, without reserved bits, and text, ping, pong or close (binary and reserved opcodes fail). Payloads are limited to 64 KiB (`obs::MAX_FRAME_BYTES`), checked from the header before the payload is read; lengths must be minimally encoded, control frames at most 125 bytes, text valid UTF-8 and valid JSON. Pings are answered with pongs; at most 16 control frames per message and 16 skipped event messages are read.
- Client frames are final, masked with a fresh key per frame, and small.
- After connecting, the peer address must be the loopback address that was dialled. `localhost` is mapped to 127.0.0.1, then ::1, without DNS, so neither `/etc/hosts` nor a resolver can redirect it. No proxy environment variable is read.
- Randomness for the WebSocket key, masking keys and request ids comes from `std::hash::RandomState` (seeded by the operating system). These values only satisfy RFC 6455; they protect no secret.

### The OBS password

- The password is never in a profile, the settings file, an argument (`ps` shows only `--obs-endpoint 127.0.0.1:4455 --obs-password-stdin`), an environment variable, a JSONL record, the activity log or a diagnostics bundle.
- Channel: the desktop app keeps it in the **macOS Keychain** (a generic password item, service `dev.cooltek.openstreamhub`, account `obs-websocket-password`; `src-tauri/src/keychain.rs`) and reads it back once per launch, the first time it is needed (Settings > OBS Studio > WebSocket password). If the Keychain refuses, the password is held in memory for that launch only and the panel says so. Each session-managed `masterhub-service` process started while OBS is on gets `--obs-password-stdin` and receives the password as one line on its **stdin pipe**, written right after spawn. The app keeps that pipe open (without writing to it again) until the process is reaped, for `--exit-on-stdin-eof` crash safety. The app overwrites the buffer it wrote from as a best effort only. The service reads at most 1 KiB of that line once, before opening the device, and keeps it in an `ObsPassword` whose `Debug` prints `<redacted>` and which has no `Display` or `Serialize`. Only the derived challenge answer goes to OBS.
- Residual risks, stated plainly:
  - **Port squatting (R1).** While OBS is not running, any local user or process can listen on the OBS port. The service checks only that the peer is a loopback address, so it would connect to that listener, receive a challenge it chose, and send back the challenge answer. That answer lets the squatter guess the password offline. A weak OBS password can be recovered this way. Use a long random OBS password (OBS can generate one in Tools > WebSocket Server Settings) and do not reuse it anywhere else. The scene name in the request is also visible to such a listener.
  - **Forgetting does not reach running sessions (R2).** Forget password (or a new password, or turning OBS off) applies only to later Starts. A running session keeps the copy it started with and sends it again to every respawned service (wake, reconnect, crash, profile switch) until you Stop that session.
  - **Plaintext in memory (R3).** Nothing is zeroed. Plaintext copies of the password stay in the app's memory (the setting and each running session's snapshot) for the life of the app, and in each service process (its copy and the stdin buffer it was read through) for the life of that process. A core dump or a memory read by another process running as you would contain it.
- Consequence: **the password is entered once.** Releases are not signed with a Developer ID, so macOS ties the Keychain item to the build that created it: after an update it may ask once whether OpenStreamHub can use the item (choose Always Allow), and if that is declined the password has to be entered again. Development builds (`npm run desktop`) avoid that prompt: every rebuild is a new binary, so they keep the item through Apple's `/usr/bin/security` tool under the service name `dev.cooltek.openstreamhub.dev` (the password stored as hex, sent on the tool's stdin, never in an argument); release builds never do. Forget password removes the Keychain item. Turning OBS on without a password works when OBS authentication is off. A run started without a password against an OBS that requires one fails each action with "OBS requires a WebSocket password".
- Why not the Keychain yet: `security add-generic-password` takes a new password either from `-w <password>` in its argument list (visible in `ps`; its own help calls this insecure) or, with `-w` as the last option, from an interactive prompt. Because `-w` must then be last, no keychain can be named, so the prompt path could not be tested without writing to the user's default keychain, and whether it reads a pipe when the app has no terminal is **unverified**. Its `-i` mode reads commands from stdin, but the password would have to be quoted into a command line for an undocumented tokenizer. None of these was accepted. Native Keychain storage through the Security framework (`SecItemAdd`/`SecItemCopyMatching`) is the planned follow-up; it needs a signed app so the item's access list stays stable across rebuilds.
- The standalone service reads the password the same way: `printf '%s\n' "$OBS_PASSWORD" | masterhub-service run … --obs-endpoint 127.0.0.1:4455 --obs-password-stdin`.

### Desktop app setting

Settings > **OBS Studio**: a switch (off by default), a host (127.0.0.1, ::1 or localhost) and a port (1–65535, default 4455), stored in `<app data>/obs-settings.json` with the same bounded, symlink-safe read and atomic owner-only (0600) write as `display-settings.json` (`src-tauri/src/obs.rs`). A missing file means off; a damaged file (including a hand-edited non-loopback host) is reported, never overwritten, and treated as off. The commands are `obs_settings`, `set_obs_settings` and `set_obs_password` (`{"password": "…"}` or `null` to forget it); replies carry `password_set`, never the password. Like the display setting, the session manager reads it once per user Start and keeps it with the session (`StartRequest.obs`, which the frontend cannot send), so wake, reconnect, crash and profile-switch restarts reuse it and changing it never restarts anything. See [session manager](masterhub-session-manager.md#obs-websocket).

### The editor's OBS listings

The OBS editors also ask OBS what it currently has, so the user picks names instead of typing them blind. Three commands over the same loopback transport, the same 3 s deadline and the in-memory password: `obs_catalog` lists scene, source (input), scene collection and profile names on one connection (`GetSceneList`, `GetInputList`, `GetSceneCollectionList`, `GetProfileList`; `ObsClient::catalog`), `obs_source_filters` (`GetSourceFilterList`) lists the filters of one source, and `obs_scene_sources` (`GetSceneItemList`) lists the items of one scene. The reply shapes are the obs-websocket v5 protocol's exactly: scenes, inputs, filters and scene items are arrays of objects with `sceneName`, `inputName`, `filterName` and `sourceName`, while scene collections and profiles are bare string arrays. All three commands are refused while OBS is off in Settings ("OBS is off in OpenStreamHub Settings"), and every name travels only as a JSON string value. Replies are bounded: at most 1000 entries per list (`obs::MAX_LISTED_NAMES`), and names beyond the profile's own 128-byte name limit (`obs::MAX_LISTED_NAME_BYTES`, which is `Action::MAX_OBS_NAME_BYTES`) are dropped from the list — nothing a profile could save is lost, the list never fails over one long name, and the editor applies the same rule, so every offered name is savable. A malformed entry (a missing or non-string name) is an error, never a silently partial list.

In the editor (`ObsFields` in `ActionFields.tsx`) the scene, source, filter, scene collection and profile fields become comboboxes while the matching list is loaded: clicking the field or pressing ArrowDown opens the choices, typing narrows them case-insensitively, picking fills the exact name. Typing always works (the press still matches names exactly), and **Refresh from OBS** asks again — a listing is only as current as the last successful ask. When the exact saved name is not in the loaded list, the field says so inline ("OBS does not list this scene right now. Refresh the OBS lists, or check the exact spelling.") without claiming the press must fail; while OBS is being asked the choices say so, and a failed listing shows its reason in the connection note while the fields stay plain inputs. Fields never offer names a profile cannot save (over 128 bytes). A filter's source choices are OBS's inputs and scenes together (OBS attaches filters to both); a source visibility's choices are the items of its named scene, fetched when that field's choices open. The frontend store is `obs-catalog.ts`; the shared IPC fixture is `fixtures/obs-catalog.json`.

The Execute confirmation lists "Switches OBS scenes through the OBS WebSocket on this Mac" with every scene name (invisible characters made visible). It does not raise the risk level: the action only changes what OBS shows.

## Diagnostics bundle redaction

When redaction is on (`redact_serials: true`), the diagnostics bundle (`apps/open-stream-hub/src-tauri/src/diagnostics.rs`) also removes action payloads from the included profile and from the client data (including `client.activity`). Any JSON object with `"type":"text"` has its `text` replaced with `<N chars>`, where N is the number of Unicode scalar values; any object with `"type":"timer"` has its `label`, and any object with `"type":"obs_scene"` its `scene`, replaced the same way. The OBS password is never in a bundle, redacted or not, because the app never writes it anywhere. Any object with `"type":"open_url"` has its `url` cut down to `scheme://host[:port]`, dropping user information, path, query and fragment. A value without `scheme://` becomes `<url>`. This applies at any depth: top-level bindings, `multi_action` steps, keypad tiles, `dial_functions` bindings and activity records. Raw snippets and URLs of 8 or more characters are also scrubbed from free text elsewhere in the bundle, such as error messages. Shorter ones are replaced only in their own field, so a short snippet that also appears inside free text survives there. Scene names are the exception: a shorter scene name found in the profile or client data is also replaced wherever it appears as a whole word in profile and client text (activity lines, errors), so `Live` or `BRB` never survives in the activity. When redaction is off, profile and client data, snippets and URLs included, appear in the bundle exactly as given.

## Security considerations

Actions run with your user account's permissions, in whatever application is focused. Accessibility and Automation permission lets this backend type into any app. Before you load a profile, understand what these actions can do:

- A `text` action with `press_enter: true`, or with a newline in its text, that is pasted into a focused terminal **executes commands**. The snippet is pasted, and Return (or the newline) submits it to the shell.
- `multi_action` can combine `launch_app` or `open_url` with `text` and `shortcut` steps. One key press can open Terminal and run arbitrary commands, or open a web page and paste the current clipboard (Command-V) into it.
- `system` `screenshot` copies the whole screen to the clipboard. A later paste, possibly by another step in the same multi-action, can put it into any application or web page.
- `media` keys go to whichever app owns media keys, not to a chosen app. `stop` only affects Apple Music and Spotify.
- `obs_scene` changes what OBS shows or streams. A profile can switch a live stream to any scene that exists in OBS, but it cannot choose where it connects (the endpoint is an app setting, loopback only) and never sees the password.
- `text` replaces the clipboard while it runs, and may restore earlier clipboard text afterwards (see the concealed-item caveat above).
- Only run profiles you wrote yourself or reviewed line by line. Treat a shared profile like a shell script from a stranger.
- The app's confirmation dialog for profiles with these actions is being reworded to state these risks plainly. Until then, don't treat that dialog as a full description of what a profile can do.

## Limits

Profile limits are unchanged: 1 MiB of JSON and 256 bindings, where a multi-action counts as one binding. Each action is bounded on its own: URLs ≤ 2048 bytes, text ≤ 2048 bytes, at most 8 steps per multi-action, no nesting. Unknown fields and unknown enum values are rejected for every new variant (`deny_unknown_fields`). Existing profile files parse and re-serialize unchanged (`existing_profile_files_round_trip_unchanged`). A shared fixture, `crates/cooltek-sdk-service/tests/fixtures/actions.json`, is checked by both the Rust tests and the app's TypeScript tests.

## Untrusted inputs and how they are constrained

| Input | Reaches | Constraint |
| --- | --- | --- |
| `open_url.url` | `open` argv (one element, after `-u`) | Scheme allow-list, ASCII URL character set, host grammar, no userinfo, ≤ 2048 bytes; no shell |
| `shortcut.key` | AppleScript text | One `[A-Za-z0-9 ]` character (cannot form AppleScript syntax) or a name mapped to an integer key code |
| `shortcut.modifiers` | AppleScript text | Closed enum mapped to fixed strings |
| `system.command` | Fixed argv / fixed script | Closed enum |
| `media.command` | `osascript` argv: a fixed JavaScript script plus, for keys, `--`, one item from `16`/`17`/`18` and one of `request`/`check` | Closed enum mapped in Rust; the script accepts only those values; `stop` takes no argv |
| `text.text` | `pbcopy` stdin only | ≤ 2048 bytes, no control characters except newline and tab, no invisible format or separator characters; stdin is capped at 8 KiB so the write cannot block |
| Previous clipboard text | `pbcopy` stdin only | Restored only when plain text with no `nspasteboard`/`concealed`/`transient` type marker, valid UTF-8, ≤ 8 KiB, and the clipboard still holds exactly the snippet after the wait |
| `launch_app.bundle_id` (existing; the app's Choose app… picker only fills it) | `open -b` argv | Dotted ASCII identifier |
| `multi_action.actions` | Each step as above | 2–8 steps, no nesting, every step validated first, 15 s overall deadline checked before each step |
| `profile.direction` | A `profile_switch_requested` record, then the app's session driver | Closed enum (`next`, `previous`); no process is started; the driver re-parses the record strictly and only switches between saved, valid profiles |
| `timer.label` | `osascript` argv (one item after `--`, read through `on run argv`) | ≤ 64 bytes, trimmed, no control characters (no newline or tab), no invisible format or separator characters; never part of the script text; never logged |
| `timer.sound` | `afplay` argv | Closed enum mapped to fixed `/System/Library/Sounds/*.aiff` paths |
| `timer.seconds` | The worker's countdown | 1–86400; at most 8 countdowns per process |
| `obs_scene.scene` | A JSON string value in one `SetCurrentProgramScene` request to a loopback OBS | Nonempty, ≤ 128 bytes, no control or invisible characters; serialized by `serde_json`, never formatted into text |
| `--obs-endpoint` (not a profile field) | TCP connect | Exactly `127.0.0.1`, `[::1]` or `localhost` (no DNS) with port 1–65535; the connected peer must be the dialled loopback address |
| OBS replies (untrusted) | The client parser, error messages | 8 KiB headers, 64 KiB frames, strict framing, 3 s deadline; comments cleaned and cut to 120 characters; close reasons never shown |

Every subprocess uses an absolute system path, has a 5 s timeout (3 s for the timer notification, 6 s for the timer sound) and 16 KiB stdout/stderr limits, and is never retried.

## Not implemented

- Selecting a profile by name with the `profile` action (see above). Profile next/previous itself works only under the OpenStreamHub session manager; the CLI loads one profile per process.
- Importing official timers from `.mhz` files.
- Clocks on keys, launch/close app variants beyond `launch_app`, lighting actions, and the Twitch, Discord, Streamlabs, Adobe and Master CTRL presets.
- OBS: a connection kept open between actions, and any OBS event or state display (all 17 official actions exist; see [the action list](research/masterhub-obs-actions.md)).
- A media stop for players other than Apple Music and Spotify, fast-forward/rewind (`media-fast-back-forward`) and playing an audio file (`play-pause`).

Conditional tests also include `{"variable_is":{"name":"takes","value":3}}` and `{"variable_at_least":{"name":"takes","value":3}}`. `all` / `any` groups nest and mix (3 levels, 16 tests in all), and a conditional may be a step of another conditional's lists (3 levels, 32 steps in all), never of a `multi_action`.
