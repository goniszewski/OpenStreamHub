# MasterHUB action catalogue (macOS backend)

This page covers the actions added to close the cheapest gaps with the official MasterHUB 1.0.0.113 action catalogue ([app features](research/masterhub-1.0.0.113-app-features.md), "Actions" row of the [parity tracker](windows-parity-gaps.md)). It explains what each action does, which ones the macOS backend actually runs, and what limits the input. The base actions (volume, set volume, mute, ASCII shortcut, launch app) are described in [the service doc](masterhub-service.md#actions-and-permissions).

None of this has been checked on hardware or on a Mac with permissions granted. The macOS behaviour described here comes from tests that run against a mock process host and assert the exact argv; no real process is started in tests. Treat every "executes" entry as **hardware- and permission-unverified**.

## Summary

| Action JSON | macOS backend | Needs |
| --- | --- | --- |
| `{"type":"open_url","url":"https://example.com"}` | Executes: `/usr/bin/open -u <url>` | Nothing extra |
| `{"type":"shortcut","key":"arrow_left","modifiers":["command"]}` | Executes: System Events `key code 123 using {command down}` | Accessibility, Automation |
| `{"type":"system","command":"undo"}` / `"redo"` | Executes: System Events `keystroke "z"` with Command / Command+Shift | Accessibility, Automation |
| `{"type":"system","command":"screenshot"}` | Executes: `/usr/sbin/screencapture -c` (whole screen to the clipboard) | Screen Recording |
| `{"type":"text","text":"Hello","press_enter":false}` | Executes: pasteboard plus Command-V (see below) | Accessibility, Automation |
| `{"type":"media","command":"play_pause"}` (also `next_track`, `previous_track`, `stop`) | **Plan-only.** Validated and planned by dry runs and replay; execution returns `unsupported on the macOS backend` and does nothing | n/a |
| `{"type":"multi_action","actions":[…]}` | Executes each step in order, 280 ms apart, within a 15 s overall deadline | Whatever the steps need |

Dry runs, replay and preview call the same `Action::validate_executable` as the backend, so a profile that previews cleanly passes the same checks that run before execution. Plan-only media actions also pass in dry runs. They fail only when executed.

## open_url

- Only lowercase `http://` or `https://`. Other schemes (`file:`, `javascript:`, `data:`, `ftp:`, custom schemes) are rejected.
- At most 2048 bytes, printable ASCII only. Allowed characters are the RFC 3986 unreserved and reserved sets plus `%`. Every `%` must start a two-hex-digit escape. Spaces, quotes (`"`), backslashes, `<>`, backticks, `{}|^`, control characters and non-ASCII are rejected, so percent-encode them. Internationalized hosts must be given in punycode.
- The host is required. It is either dot-separated labels of letters, digits and hyphens (no empty labels, no leading or trailing hyphen, at most 63 bytes per label) or a bracketed IPv6 literal. An optional port must be 1–65535. User information (`user:pass@`) is rejected, and so is any `@` in the authority.
- It runs `/usr/bin/open -u <url>`, with the URL passed as a single argv element and no shell. `-u` makes `open` treat the value as a URL even if it matches a local path. Since the URL must start with `http`, it can never be read as an option.

## Hotkey chords (`shortcut`)

`shortcut` stays backward compatible. `key` is either:

- one ASCII letter, digit or space, sent as `keystroke "<key>"` as before, or
- exactly one of these lowercase names, sent as `key code <n>`: `return`, `tab`, `space`, `delete` (backspace), `escape`, `forward_delete`, `home`, `end`, `page_up`, `page_down`, `arrow_left`, `arrow_right`, `arrow_down`, `arrow_up`, `f1`…`f20`.

Any subset of `command`, `control`, `option`, `shift` can be added, with no duplicates. Named keys are mapped to integers from a closed table (`NAMED_KEYS` in `profile.rs`, Carbon `kVK_*` values), so no user text is placed into the AppleScript source. Uppercase names, unknown names, punctuation and sequences are rejected. Function keys may be captured by macOS or by the keyboard's media-key setting before the focused app sees them. That has not been checked.

## System actions

- `undo` and `redo` send Command-Z and Shift-Command-Z to the focused application. Each app decides what those keys do.
- `screenshot` runs `/usr/sbin/screencapture -c`. It captures the whole screen to the clipboard, which replaces the clipboard's contents, and plays the system shutter sound. Without Screen Recording permission, macOS may capture only the desktop background. The official app's other system actions (switch desktop, zoom, custom scaling, scroll) are not implemented.

## Media control (plan-only)

`media` accepts `play_pause`, `next_track`, `previous_track` and `stop`, matching the official `media-track-control` options. System Events can only send ordinary key codes. Hardware media keys (`NX_KEYTYPE_*`) need private or low-level event APIs, and this backend deliberately does not use those. Scripting one specific player such as Music would not be a system-wide media key, so it is not used either. The variant exists so profiles can carry it and replay can plan it. On macOS, execution returns an error saying no action was taken. `media-fast-back-forward` and the official `play-pause` (play an audio file) are not implemented.

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

- 2 to 8 steps, matching the official flat `multi-tasking` list. A step can be any executable action except `multi_action` (no nesting) and `volume_from_position` (a fader template that needs an input value). Fader bindings still accept only `volume_from_position`, so a multi-action cannot be bound to a fader.
- Every step is validated before the first one runs. Steps run in order with a fixed 280 ms pause between them, not before the first or after the last. The pause is an injected `Sleeper`, so tests never sleep.
- The first failing step stops the sequence. The error names it, for example `multi_action step 2 of 3 (system) failed: …`. Earlier steps have already taken effect and are not rolled back. Nothing is retried.
- Cancellation (`MacOsDispatcher::with_cancellation`) is checked before every subprocess, while monitoring each subprocess, and every 10 ms during the 280 ms pauses, so a cancelled sequence stops between or inside steps.
- Overall deadline: a whole multi-action gets 15 s (`MULTI_ACTION_DEADLINE`). The deadline is checked before each step starts, after the pause. Once it has passed, the sequence stops with `multi_action stopped before step N of M (<type>): the sequence exceeded its 15 s overall deadline; earlier steps already took effect`. A step that is already running is not interrupted. Its own subprocess timeouts bound it. The clock comes from the injected `Sleeper`, so tests use virtual time.
- Worst-case time per step, with every subprocess hitting its 5 s timeout:

  | Step | Subprocesses | Worst case |
  | --- | --- | --- |
  | `text` | up to 6 (clipboard info, pbpaste, pbcopy, paste script, pbpaste re-read, restore pbcopy) | 30 s plus the 0.5 s restore wait = 30.5 s |
  | `volume`, `toggle_mute` | 2 (query, set) | 10 s |
  | Every other executable step | 1 | 5 s |

- Worst case for a whole multi-action: the last step can start just before the 15 s deadline and then run its own worst case, so a sequence ending in a `text` step can take about 15 + 30.5 ≈ 45.5 s. Without the deadline, 8 `text` steps would take 8 × 30.5 + 7 × 0.28 ≈ 246 s.
- The service runs actions one at a time, so input that arrives while a multi-action runs waits behind it. The live runtime queues up to 64 reports. After that, later reports are coalesced into one pending work item: relative volume steps are summed, absolute positions keep the latest value, and other events are appended up to a fixed bound and then dropped and counted (see `live_runtime.rs` and [the service doc](masterhub-service.md)). Presses made during a long sequence may run late, be merged with others, or never run.

## Diagnostics bundle redaction

When redaction is on (`redact_serials: true`), the diagnostics bundle (`apps/open-stream-hub/src-tauri/src/diagnostics.rs`) also removes action payloads from the included profile and from the client data (including `client.activity`). Any JSON object with `"type":"text"` has its `text` replaced with `<N chars>`, where N is the number of Unicode scalar values. Any object with `"type":"open_url"` has its `url` cut down to `scheme://host[:port]`, dropping user information, path, query and fragment. A value without `scheme://` becomes `<url>`. This applies at any depth: top-level bindings, `multi_action` steps, keypad tiles, `dial_functions` bindings and activity records. Raw snippets and URLs of 8 or more characters are also scrubbed from free text elsewhere in the bundle, such as error messages. Shorter ones are replaced only in their own field, so a short snippet that also appears inside free text survives there. When redaction is off, profile and client data, snippets and URLs included, appear in the bundle exactly as given.

## Security considerations

Actions run with your user account's permissions, in whatever application is focused. Accessibility and Automation permission lets this backend type into any app. Before you load a profile, understand what these actions can do:

- A `text` action with `press_enter: true`, or with a newline in its text, that is pasted into a focused terminal **executes commands**. The snippet is pasted, and Return (or the newline) submits it to the shell.
- `multi_action` can combine `launch_app` or `open_url` with `text` and `shortcut` steps. One key press can open Terminal and run arbitrary commands, or open a web page and paste the current clipboard (Command-V) into it.
- `system` `screenshot` copies the whole screen to the clipboard. A later paste, possibly by another step in the same multi-action, can put it into any application or web page.
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
| `system.command`, `media.command` | Fixed argv / fixed script / no process | Closed enums |
| `text.text` | `pbcopy` stdin only | ≤ 2048 bytes, no control characters except newline and tab, no invisible format or separator characters; stdin is capped at 8 KiB so the write cannot block |
| Previous clipboard text | `pbcopy` stdin only | Restored only when plain text with no `nspasteboard`/`concealed`/`transient` type marker, valid UTF-8, ≤ 8 KiB, and the clipboard still holds exactly the snippet after the wait |
| `launch_app.bundle_id` (existing) | `open -b` argv | Dotted ASCII identifier |
| `multi_action.actions` | Each step as above | 2–8 steps, no nesting, every step validated first, 15 s overall deadline checked before each step |

Every subprocess uses an absolute system path, has a 5 s timeout and 16 KiB stdout/stderr limits, and is never retried.

## Not implemented

- Profile next/previous: needs a session manager that can swap the active profile at run time. The CLI loads one profile per process.
- Recorded key macros, timers and clocks, launch/close app variants beyond `launch_app`, lighting actions, and the OBS, Twitch, Discord, Streamlabs, Adobe and Master CTRL presets.
- Executable media keys on macOS (see above).
