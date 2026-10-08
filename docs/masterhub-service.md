# MasterHUB binding and action service

`masterhub-service` maps all five control modules' events to a small set of macOS actions. Profiles and replay work without a device. Live runs read one explicitly selected HID interface and default to logging planned actions. Physical input remains untested. Device-free host checks and their observed limitations are documented in the [macOS smoke guide](masterhub-macos-smoke.md).

## Validate and replay

Run these commands from the repository root:

```sh
cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  validate examples/profiles/macos.json

cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  replay --profile examples/profiles/macos.json \
  crates/cooltek-sdk-cli/tests/fixtures/m003k-synthetic.jsonl

cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  replay --profile examples/profiles/macos.json \
  crates/cooltek-sdk-cli/tests/fixtures/m131e-software-derived-synthetic.jsonl
```

Validation checks the entire profile before printing `profile_valid`. Replay accepts the schema-1 JSONL input captures described in the [diagnostic guide](masterhub-diagnostics.md#capture-format-and-replay). It constructs no HID context, opens no interfaces, and executes no macOS actions. All bundled input captures are synthetic. To try the additional modules, use `examples/profiles/all-modules.json` with `m4315-software-derived-synthetic.jsonl`, `m005s-software-derived-synthetic.jsonl`, or `m032le-software-derived-synthetic.jsonl` in the same fixture directory.

Schema-2 profiles add [nested keypad folders](masterhub-keypad-folders.md); see the [example profile](../examples/profiles/keypad-folders.json). Validation counts actions across all pages. Replay routes key events through a stateful `ProfileSession` and reports navigation separately from planned host actions. A folder's top-left Back key returns one level, with a generated `L1`–`L8` badge in the desktop editor. Key releases retain the page where the press began.

Live M4315 runs reject schema-2 keypad profiles before opening the selected device unless the experimental `--display-sync` opt-in (run once on a real M4315 and once on a real M131E) is given (see [keypad display sync](#keypad-display-sync-experimental)). The same opt-in with `--dial-content` draws the M131E dial screen (see [dial display sync](#dial-display-sync-experimental)). Other modules can use their flat bindings from the same profile. Schema-1 live input retains its existing behavior.

Replay streams reports and planned bindings as they are read. An invalid or truncated capture, missing footer, or transport-error footer produces a failing exit status even when earlier output was emitted. Check the exit status and final `service_end` before treating the stream as a complete session.

## Read an interface

Obtain its exact `path_hex` from inventory, then run a bounded preview:

```sh
cargo run -p cooltek-sdk-cli --bin masterhub-diag -- list

cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  run --profile examples/profiles/macos.json \
  --path-hex HEX_FROM_INVENTORY --duration-secs 30
```

The path contains the lossless interface bytes without a trailing NUL. The service refuses missing, ambiguous, stale, or unsupported selections; it never falls back to the first matching VID/PID. All five control modules are supported under both known PID generations; see the [additional-module evidence](research/masterhub-additional-modules-1.0.0.113.md). Each process owns one selected interface.

Without `--execute`, matched actions are validated and logged without touching volume, sending shortcuts, or opening applications. Without `--software-mode`, the service sends no configuration commands. Reads default to a 100 ms timeout; `--timeout-ms` accepts 1–1000. `--stop-on-action-error` ends the session on the first failed action instead of logging it and continuing. Optional `--max-reports` stops after that many nonempty input reports. `--duration-secs` accepts 1–86400; omitting both duration and report limits runs until interrupted or an error occurs.

Live stdout must be a terminal, pipe, or socket. Direct regular-file redirection is rejected because it cannot provide the required write deadline. To save a live session in zsh or bash, enable pipeline failure reporting and use an actively consumed pipe:

```sh
set -o pipefail
cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  run --profile examples/profiles/macos.json \
  --path-hex HEX_FROM_INVENTORY --duration-secs 30 | tee actions.jsonl
```

Validation and replay keep their ordinary stdout behavior and support direct file redirection.

On macOS, add `--execute` to enable the profile's actions:

```sh
cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  run --profile examples/profiles/macos.json \
  --path-hex DIAL_PATH_HEX_FROM_INVENTORY --duration-secs 30 \
  --execute --software-mode
```

The two flags are independent. `--execute` enables macOS actions and does not enable device software mode. `--software-mode` requests software mode for M131E or M4315 and can be used during a dry run. Once enabling has been attempted, the service attempts to restore the selected IPS module's hardware mode when the session ends, including after an enable failure, action failure, input error, or output error.

When a duration or report limit is reached, the service stops reading, closes the work queue, and allows up to ten seconds to drain accepted work. An expired drain deadline cancels pending work and fails the session. Ctrl-C, SIGTERM, input/action/output failures, and queue overflow request cancellation of pending work; the coordinator allows up to two seconds to observe worker completion after cancellation. Input state is cleared on either path.

The native backend checks the shared cancellation flag before each subprocess and between volume/mute queries and setters. An in-flight system-tool process is stopped with bounded kill/reap cleanup when cancellation is observed; completed effects cannot be undone. The worker owns logging and action dispatch, and returns its writer to the coordinator before the final footer is written after requested dial cleanup. A worker whose completion cannot be observed is not joined indefinitely.

Requested IPS module hardware-mode restoration is attempted once after the worker phase, also after a contained panic in the input loop (reported as `input loop panicked`) or later on the coordinator thread during drain and worker completion (reported as `live session coordinator panicked`, without a `service_end` footer). On that coordinator-panic path the worker is cancelled but not awaited, and the restore runs immediately; the worker never touches the device transport, so this causes no device contention. Cleanup failures produce a failing exit status. HIDAPI feature writes are synchronous and expose no timeout here, so the worker/log deadlines do not establish a whole-process shutdown bound. Forced termination can prevent cleanup.

### Run options

All options of `masterhub-service run`, as defined in `crates/cooltek-sdk-cli/src/bin/masterhub-service.rs`:

| Option | Meaning |
| --- | --- |
| `--profile PATH` | Profile to load (required). |
| `--path-hex HEX` | Exact interface from `masterhub-diag list` (required). |
| `--execute` | Run actions on the host (macOS only). Without it actions are only logged. |
| `--software-mode` | Request IPS dial or key software mode and restore hardware mode on exit. |
| `--duration-secs N` | Stop after 1–86400 s. |
| `--max-reports N` | Stop after N nonempty input reports. |
| `--timeout-ms N` | Read timeout, 1–1000 ms (default 100). |
| `--stop-on-action-error` | End the session on the first failed action. |
| `--display-sync` | Experimental: draw M4315 key images, or the dial screen with `--dial-content`. Requires `--software-mode`. |
| `--dial-content off\|function\|clock\|analog-clock\|cpu` | Experimental: what the M131E dial screen shows (default `off`). Any other value requires `--display-sync`. |
| `--profile-switch-requests` | Report `profile` actions as `profile_switch_requested` for a supervising session manager. |
| `--obs-endpoint ADDR` | Loopback obs-websocket endpoint for `obs_scene` (`127.0.0.1:PORT`, `[::1]:PORT` or `localhost:PORT`). |
| `--obs-password-stdin` | Read the OBS password from the first line of stdin. Requires `--obs-endpoint`. |

`validate` takes a profile path; `replay` takes `--profile PATH` and a capture path.

### Keypad display sync (experimental)

`--display-sync` draws M4315 key images for a schema-2 keypad profile. It is off by default. It ran once on a real M4315 on 8 October 2026: the first page and folder navigation worked with no failed writes. Physical key order, how the tiles look after navigation, and page-transition latency are still unverified, and wider hardware acceptance is pending (see [display sync](masterhub-display-sync.md)).

```sh
cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  run --profile examples/profiles/keypad-folders.json \
  --path-hex KEYPAD_PATH_HEX_FROM_INVENTORY --duration-secs 30 \
  --software-mode --display-sync
```

- The flag requires `--software-mode` and a schema-2 keypad profile; both are checked before any HID access. A selected interface that is not an M4315 is rejected before it is opened. The flag is independent of `--execute`.
- Key images are generated placeholders (`PreparedKeyArtwork`, prepared before the device is opened): blank, action tile, folder silhouette, and a Back arrow with an `L1`–`L8` badge. Profiles have no image field yet. For the dial, see [dial display sync](#dial-display-sync-experimental).
- The one opened device is shared by the input reader and a dedicated display thread through a mutex with bounded waits (`SharedTransport`). A display writer yields to a waiting input read after at most 16 output reports, and to a hardware-mode command before every report.
- Setup fails closed. If the display sink cannot be created (for example, no output report `0x02` in the descriptor) or the display thread cannot start, the run exits with an error starting `--display-sync setup failed` before software mode is entered: no mode command is sent, no worker starts, and nothing is written to stdout. Folder navigation never runs live without its display.
- The first sync is requested right after software mode is enabled. Another is requested after each report whose events include a keypad navigation, only after that report's work was queued or coalesced. The display thread holds at most one pending request; a newer page replaces an older one. Requests never wait for the display thread, and display failures never dispatch, suppress, or reorder an action. An in-flight display write can delay reading input reports: while a write holds the shared device, reads report a normal timeout, input reports queue in the OS, and they are read afterwards. A write stuck inside the HID backend delays input, and unplug detection, until it returns.
- When the input loop ends (limit, interrupt, or error), the service closes a write gate before anything else. No sync starts, and an in-flight sync fails its remaining keys at once: the gate is checked before every key and every output report, so at most the report already inside the backend completes. The coordinator then waits up to the cancellation budget (2 s) for the display thread. Restoring hardware mode takes priority over display writes. If the device is still busy after the 2 s lock bound (a write stuck in the backend), the restore is retried once, and the error reports both attempts. `service_end.display_stopped` is `false` when the display thread outlived the wait; the closed gate still refuses any further report.
- Once the run has started, a display failure, short write, or stalled sink never ends the session, dispatches or suppresses an action, or changes the exit status by itself. A hardware-mode restore that still fails after its retry does fail the run, as any cleanup failure does. It appears only in the records below. The service has no reconnect or wake path (the desktop app restarts the process), so each run starts with every key dirty and redraws all 15.

Each sync writes one `display_sync` record (`schema_version` 1), logged by the action worker:

```json
{"type":"display_sync","schema_version":1,"trigger":"navigation","page_id":"development","depth":1,
 "written":[0,1,2],"failed":[{"key_index":3,"short_write":true,"attempts":1,"error":"display write returned 10 bytes for a 65-byte report"}],
 "not_attempted":[4,5,6,7,8,9,10,11,12,13,14],"missing":[],"error":null,"complete":false}
```

`trigger` is `software_mode` or `navigation`. `written`, `not_attempted`, and `missing` (keys without artwork) list wire key indexes; `failed` holds at most one entry per key; error text is truncated to 240 characters, so a record stays well below the 64 KiB limit. `error` is set when planning or applying failed before any write. At most 32 records wait for the worker; more are dropped and counted. With the flag, `service_start.details.display_sync` is `{"enabled":true,"hardware_verified":false}`, and `service_end` adds `display_sync_count`, `display_failure_count` (syncs with an error or any key not written), `display_coalesced_request_count`, `display_dropped_record_count`, and `display_stopped` (false when the display thread did not finish within the budget, typically because a write is stuck in the backend; it is left running behind the closed gate). Runs without the flag have none of these fields.

### Dial display sync (experimental)

`--display-sync --dial-content <off|function|clock|analog-clock|cpu>` draws the M131E dial screen. It is off by default. All four faces ran on a real M131E on 8 October 2026 and were legible and upright; function cycling and long-run behavior are unverified (see the [hardware results](masterhub-hardware-results.md#m131e-dial)).

```sh
cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  run --profile examples/profiles/dial-functions.json \
  --path-hex DIAL_PATH_HEX_FROM_INVENTORY --duration-secs 120 \
  --software-mode --display-sync --dial-content clock
```

- `--dial-content` is parsed strictly (exactly `off`, `function`, `clock`, `analog-clock`, `cpu`; default `off`). A value other than `off` requires `--display-sync` and `--software-mode` (checked before any HID access) and an M131E interface (checked before it is opened). `--display-sync` without `--dial-content` keeps its keypad meaning. Any profile works; a schema-2 profile is flattened for the dial as before.
- `function` shows the active dial function's name and position (the profile name without functions); `clock` shows local `HH:MM`; `analog-clock` shows hands; `cpu` shows whole-machine CPU usage as a whole percentage with a ring gauge (orange from 80 %). Faces are generated by `DialArtwork` with a built-in bitmap font and seven-segment digits; function faces are prepared before the device is opened.
- `cpu` reads the stable Mach call `host_processor_info(PROCESSOR_CPU_LOAD_INFO)` on macOS, with no extra dependency. Usage is the busy share (user, system, nice) of the ticks all cores gained since the previous sample. It is sampled on the display thread only, at most once a second, and only after software mode is on; the input loop never waits for it. The first sample is a baseline, so the first CPU face is drawn about a second after software mode is on, with a real percentage. An interval with no elapsed ticks keeps the last percentage. Off macOS, or when a sample fails or the sampler panics, the dial shows `--` and the service prints `masterhub-service: CPU usage is unavailable; the dial shows --` on stderr once per run. GPU usage and temperatures are not available: they need private IOKit or SMC interfaces.
- The dial is redrawn right after software mode is enabled, after a dial function switch, when the clock's minute changes, and when the displayed CPU percentage changes, only when the face changed and at most once a second. Requests never wait for the display thread. A short write is never retried; the next change redraws it. A plain transport failure (for example a busy shared device) is retried once for the same face after the interval (`trigger: retry`), then left until the next change. Setup, the write gate, the shared device, and the hardware-mode restore behave exactly as for keys, and a display failure never changes the exit status by itself.

Each attempt writes one `display_sync` record with `"target":"dial"`:

```json
{"type":"display_sync","schema_version":1,"target":"dial","trigger":"dial_function","face":"function:1",
 "written":true,"failed":null,"missing":false,"error":null,"complete":true}
```

`trigger` is `software_mode`, `dial_function` (function content), `clock` (any redraw of a clock face, including the first after a click in clock mode), `cpu` (a redraw for a new CPU percentage, likewise), `retry`, or `panic` (choosing the face panicked before a face existed; `face` is `null`, `error` is the display-panic message, and the display is off for the rest of the run). `face` names the drawn face (`function:<index>`, `profile`, `clock:HH:MM`, `analog:HH:MM`, `cpu:<0-100>`, or `cpu:--` while no sample is available). `failed` is `null` or `{"short_write":…,"attempts":…,"error":…}`. `service_start.details.display_sync` adds `"target":"dial"`, `"dial_content"` and `"min_interval_ms":1000`, `service_start.details.dial_content` records the flag when it is not `off` (default runs log exactly the fields they did before), and `service_end` carries the same display counters as for keys.

## How the desktop app finds the binary

The desktop app runs profile actions only through this binary. It looks in this order: the path in `OPEN_STREAM_HUB_SERVICE`, then `masterhub-service` next to the app executable, then (debug builds only) `target/debug` and `target/release` of each enclosing Cargo workspace. A built `OpenStreamHub.app` bundles the binary as a Tauri sidecar (`bundle.externalBin`), which the bundler installs at `Contents/MacOS/masterhub-service`, so the second location normally finds it. See [Building the desktop app](building-the-desktop-app.md).

## Profiles and gestures

The [sample profile](../examples/profiles/macos.json) defines these actions:

| Control | Gesture | Action |
| --- | --- | --- |
| M003K index 0 | Clockwise / counter-clockwise | Output volume +5 / −5 percentage points |
| M003K index 0 | Pressed | Toggle output mute |
| M003K index 1 | Pressed | Command-C |
| M003K index 2 | Pressed | Command-V |
| M131E index 0 | Right / left | Output volume +5 / −5 percentage points |
| M131E index 0 | Short press / long press | Toggle output mute / open Finder |

Control indexes are wire indexes. Their physical placement needs hardware checking. Dial `right` and `left` retain the Windows application's labels; their physical clockwise mapping is unverified. Roller `up` and `down` likewise retain application labels. The GUI numbers controls by wire index without claiming their physical placement.

A profile requires a supported `schema_version`, a nonempty `name`, and a `bindings` array. Schema 1 contains flat bindings; schema 2 also requires a validated `keypad` page layout and stores M4315 bindings in its action tiles. Other modules continue to use flat bindings. A binding requires `module`, `control`, `gesture`, and `action`; optional `serial_number` matches the exact, case-sensitive HID enumeration serial shown by inventory. The service does not query vendor serial feature reports. Omitting the serial matches any device serial, including an unavailable serial. An exact serial binding takes precedence over a wildcard for the same module, control, and gesture, independent of array order. Duplicate complete selectors are rejected.

| Module | Allowed controls | Allowed gestures |
| --- | --- | --- |
| `m003k` | `0`, `1`, `2` | `clockwise`, `counter_clockwise`, `pressed`, `released` |
| `m131e` | `0` | `right`, `left`, `pressed`, `released`, `short_press`, `long_press` |
| `m4315` | `0` through `14` | `pressed`, `released` |
| `m005s` | `0` through `4` | `position_changed` |
| `m032le` | `0`, `1` | `up`, `down` |

Dial short/long gestures are classified on release after an observed press: less than 600 ms is short; at least 600 ms is long. A release emits `released` before the classified gesture, so bindings for both can execute two actions. Repeated button snapshots do not retrigger transitions; repeated rotations remain separate events. A release without a matching observed press produces no short/long gesture. Session reset clears pending presses without synthesizing actions.

A profile in schema 1 or 2 can add an optional `dial_functions` array, which is the official app's model: a short click switches to the next function and runs no action; a long press and rotation run the current function's bindings. With `dial_functions`, flat `bindings` cannot contain `m131e`, and function bindings accept only `right`, `left`, and `long_press`. Profiles without the field keep the flat dial behavior above, and their files are unchanged. See [dial functions](masterhub-dial-functions.md). Replay and live runs route through `ProfileSession::route` and log each switch as its own event record (see [dial function records](#dial-function-records)). Live runs accept these profiles; the current function starts at the first one and resets to it whenever the process restarts, and the dial screen shows it only with the experimental `--display-sync --dial-content function` (see [dial display sync](#dial-display-sync-experimental)). The [example profile](../examples/profiles/dial-functions.json) and the synthetic capture `crates/cooltek-sdk-cli/tests/fixtures/m131e-dial-functions-synthetic.jsonl` exercise switching, wrap-around, and both sides of the 600 ms threshold.

Fader events carry an absolute position as `value: {"type":"position","raw":63,"maximum":100}`. The first valid snapshot emits all five positions; later snapshots emit only changed controls. Reset clears known positions, so a new session can plan the initial fader action immediately. Fader bindings currently require `volume_from_position`, which resolves the value into a concrete `set_volume` action. Keys deduplicate snapshots; roller updates remain separate events, with `ff` interpreted as idle and no inferred distance.

Unknown reports retain their raw bytes in the log and emit no events. Malformed input is logged as `parse_error` without changing decoder/button state. Those packet diagnostics do not end a session; transport, action, and output failures do.

JSON parsing rejects unknown fields, unsupported schema versions, invalid module/gesture combinations, and invalid actions. Profiles are bounded to 1 MiB and 256 bindings. Names are at most 128 bytes; serial selectors are at most 256 bytes. Names and serial selectors cannot be empty or contain control characters.

## Actions and permissions

| Action JSON | Behavior and constraints |
| --- | --- |
| `{"type":"volume","delta":5}` | Reads current system output volume, adds signed percentage points, and clamps to 0–100. Delta must be nonzero and within −100…100; the setter specifies only output volume. |
| `{"type":"volume_from_position"}` | Fader-only binding that resolves the received absolute position into `set_volume` before dispatch. Missing or invalid values fail; the marker itself cannot execute. |
| `{"type":"set_volume","percent":63}` | Sets absolute output volume to 0–100 with one setter and no volume query. Available for discrete bindings and emitted by the fader resolver. |
| `{"type":"toggle_mute"}` | Reads and inverts the system output's current mute state. |
| `{"type":"shortcut","key":"c","modifiers":["command"]}` | Sends a keystroke to the current application through System Events. Key is exactly one ASCII letter, digit, or space (sent as a literal keystroke), or one allow-listed named key such as `arrow_left`, `return`, `escape`, `page_up`, or `f1`…`f20` (sent as an integer key code). Distinct modifiers are `command`, `control`, `option`, and `shift`; an empty array is allowed. Punctuation, Unicode keys, and sequences are unsupported. |
| `{"type":"launch_app","bundle_id":"com.apple.finder"}` | Uses `open -b` to request an installed application by bundle ID. IDs are dotted ASCII identifiers of at most 255 bytes: at least two nonempty components, each starting with a letter/digit and containing only letters, digits, or hyphens. |
| `{"type":"open_url","url":"https://example.com"}` | Uses `open -u` to open an `http`/`https` URL of at most 2048 ASCII bytes with a host and no user information. |
| `{"type":"system","command":"undo"}` | `undo`/`redo` send Command-Z/Shift-Command-Z; `screenshot` runs `/usr/sbin/screencapture -c` (whole screen to the clipboard). |
| `{"type":"text","text":"Hello","press_enter":false}` | Pastes a snippet of at most 2048 bytes (no control characters except newline and tab) through `pbcopy` stdin and Command-V, optionally followed by Return. Restores the previous clipboard only when it was small plain text. |
| `{"type":"media","command":"play_pause"}` | `play_pause`, `next_track` and `previous_track` post a hardware media key (down then up) through `/usr/bin/osascript -l JavaScript` with a fixed script and, after `--`, the key type and a `request`/`check` mode (only a run's first media key may show the Accessibility prompt); needs Accessibility. `stop` (no stop media key exists) stops Apple Music and pauses Spotify, each only if already running; it does not start a player that is not running, but if a player quits in the instant between the check and the command, macOS may relaunch it, which does not start playback; with neither running it does nothing; needs Automation. Allowed as a `multi_action` step. Unverified on a real Mac. |
| `{"type":"multi_action","actions":[…]}` | Runs 2–8 non-nested executable steps in order, 280 ms apart, stopping at the first failure. |
| `{"type":"obs_scene","scene":"Live"}` | Switches the OBS program scene over the loopback obs-websocket v5 server given by `--obs-endpoint` (one connection per action, 3 s limit, no subprocess). Without `--obs-endpoint` it is logged as `action_unavailable` and does nothing; a dry run plans it without connecting. Scene: nonempty, ≤ 128 bytes, no control or invisible characters. Allowed as a `multi_action` step. |
| `{"type":"profile","direction":"next"}` | Never executed on the host. With `--profile-switch-requests` (passed by the desktop app for session-managed runs whose profile binds a `profile` action) it is logged as `profile_switch_requested` for the app's session manager, which switches to the next or previous saved profile; otherwise it is logged as `action_unavailable` and does nothing. Not allowed as a `multi_action` step. |
| `{"type":"timer","seconds":300,"label":"Tea","sound":"glass"}` | Never dispatched as a press. Starts a 1–86400 s countdown owned by the service worker; a second press of the same binding cancels it; at most 8 run at once. At the end the backend posts a notification through `/usr/bin/osascript` (fixed script, label as an argv item) (3 s bound) and plays a fixed `/System/Library/Sounds/<Name>.aiff` with `/usr/bin/afplay` (6 s bound), on a separate alert thread so actions never wait for it. Every countdown is cancelled when the run ends. Not allowed as a `multi_action` step or on a fader. |

Full rules, the clipboard clobber risk, and the list of untrusted inputs are in [the action catalogue](masterhub-actions.md). Recorded macros are not implemented.

The backend validates actions again before execution; dry runs and replay use the same executable validation. It invokes absolute `/usr/bin/osascript`, `/usr/bin/open`, `/usr/bin/pbcopy`, `/usr/bin/pbpaste`, `/usr/sbin/screencapture`, and `/usr/bin/afplay` paths with separate arguments, without a shell or arbitrary script actions. User text reaches a process only as one argv element (`open`, or a timer label read by a fixed AppleScript through `on run argv`) or on stdin (`pbcopy`); it is never interpolated into AppleScript. Each subprocess has a five-second timeout (a timer's notification three seconds, its sound six) and separate 16 KiB stdout/stderr limits; failure or cancellation cleanup allows up to one additional second to reap a killed process. Volume and mute actions use a query followed by a setter, so one action can involve two subprocesses; a text snippet uses up to five.

Shortcut, system undo/redo, media and text execution may require Accessibility and Automation permissions; a screenshot may require Screen Recording permission. The identity shown by macOS depends on the launching context, such as Terminal versus a bundled application. Only explicit execution can invoke these tools or prompt for permissions; validation, replay, and dry runs do not. A failed action, including one caused by a missing permission, preserves the system tool's diagnostic and is logged as `action_failed`. The exceptions are `media` actions and timer alerts: their errors give a fixed cause and the stderr byte count only, never stderr text (`text` actions keep the tool's diagnostic, since the snippet travels on stdin and never in argv); the session continues unless `--stop-on-action-error` is given. No permission settings are changed automatically.

Apple documents the underlying [volume commands](https://developer.apple.com/library/archive/documentation/AppleScript/Conceptual/AppleScriptLangGuide/reference/ASLR_cmds.html), [System Events UI scripting and Accessibility](https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/AutomatetheUserInterface.html), and [Automation permissions](https://support.apple.com/en-hk/guide/mac-help/mchl108e1718/mac). These sources establish syntax and permission requirements; they do not verify this service's execution on a particular Mac or audio output.

The [controlled smoke examples](masterhub-macos-smoke.md) use the same dispatcher without HID. Audio checks restore and verify an exact snapshot; scratch keystroke checks require independent observation. Neither example upgrades a successful system-tool return to verified host or hardware behavior.

## Output, failures, and integration

Output is JSON Lines: `service_start`, raw `report` diagnostics, normalized `event` records with planned/unbound bindings and resolved `action` values, optional `action_executed`/`action_failed` (each carrying `schema_version`), and `service_end` counts. The service emits the original binding and concrete resolved action before dispatch. Records never contain snippet text or URL paths and queries: a `text` action is logged as `{"type":"text","chars":N,"press_enter":bool}`, an `open_url` action as `{"type":"open_url","origin":"scheme://host[:port]"}`, and `multi_action` steps are redacted the same way, in `binding`, `action` and every `action_executed`/`action_failed` record. Other actions are logged unchanged. an output failure prevents the corresponding unlogged action. `action_count` counts successfully dispatched bindings, including dry-run plans. `action_executed` records system-tool completion rather than independent verification of the resulting application or system state.

An `obs_scene` action, or a `multi_action` with an `obs_scene` step, in a run without `--obs-endpoint` is never dispatched either: after its `event` record the service writes `action_unavailable` (`event`, `action`, `reason`; the action is logged as `{"type":"obs_scene","scene_chars":N}`, never with the scene name), it is not a failure, and `service_end` counts it in `unavailable_action_count` (not in `action_count`, `failed_action_count` or `incomplete_action_count`). With an endpoint, `service_start` details add `"obs":{"endpoint":"127.0.0.1:4455","password":true}` (whether a password was read, never the password).

### OBS options

- `--obs-endpoint 127.0.0.1:PORT` (or `[::1]:PORT`, `localhost:PORT`): the loopback obs-websocket endpoint for `obs_scene`. Anything else is rejected during argument parsing.
- `--obs-password-stdin` (requires `--obs-endpoint`): read the OBS WebSocket password from the first line of stdin (at most 1 KiB, no control characters) before the device is opened. There is no option or environment variable that takes the password itself. Example: `printf '%s\n' "$OBS_PASSWORD" | masterhub-service run --profile p.json --path-hex HEX --execute --obs-endpoint 127.0.0.1:4455 --obs-password-stdin | tee run.jsonl`.

Protocol, limits and the password channel are described in [the action catalogue](masterhub-actions.md#obs-scene-obs_scene).

A `profile` action is never dispatched. After its `event` record the service writes `profile_switch_requested` (`event`, `direction`) when `--profile-switch-requests` is given, or `action_unavailable` (`event`, `action`, `reason`) otherwise. Neither is a failure or ends the session. `service_end` adds `profile_request_count`; these actions are not in `action_count` or `incomplete_action_count`. The fields are additive, so records keep `schema_version` 1. See [the action catalogue](masterhub-actions.md#profile-nextprevious-profile).

A `timer` action is never dispatched either. After its `event` record the service writes `timer_started` or `timer_cancelled` (`reason` `pressed_again`); a press while 8 countdowns run is an `action_failed`. The worker checks deadlines between work items and writes `timer_expired` when one ends, then hands the alert to the session's alert thread, which sends results back for the worker to log. A stop cuts a running alert off at once and nothing starts after it; a normal end gives a running alert at most 2 s, so an alert never causes a drain error. A failed, skipped or cut-off alert is a `timer_alert_failed` record, an alert whose notification tool succeeded but wrote to stderr is a `timer_alert_warning` with a fixed category (never stderr text), and neither ends the session. When the run ends, every countdown still running is written as `timer_cancelled` with `reason` `session_end` (input limit or end of a replayed capture) or `stopped` (interrupt, stop, error); nothing expires after a stop. A timer action is logged as `{"type":"timer","seconds":N,"label_chars":K,"sound":"…"}`, never with its label. A press counts in `action_count`; `service_end` adds `timer_started_count`, `timer_cancelled_count`, `timer_expired_count`, `timer_alert_failed_count` and `timer_alert_warning_count` only when a timer started. Replay uses capture timestamps as the timer clock. See [the action catalogue](masterhub-actions.md#countdown-timer-timer).

### Dial function records

These fields are additive, so every record keeps `schema_version` 1. They appear only for an M131E device whose profile has `dial_functions`.

- `service_start.details.dial_functions`: `{"count":3,"names":["Volume","Scroll","Apps"],"initial_index":0,"short_press":"switches_function","resets_on_restart":true}`. A live run also prints a one-line notice on stderr.
- A short click is an `event` with `"status":"dial_function"`, `action` and `binding` both `null`, and `navigation` set to `{"from_function":"Volume","to_function":"Scroll","index":1,"count":3}`, where `index` is the zero-based new current function. It is never dispatched, so it never produces `action_executed` or `action_failed`, and it does not count toward `action_count`. The click's `pressed` and `released` events stay `unbound`.
- Every other dial `event`, and every `action_executed` or `action_failed`, carries `"dial_function":{"name":"Scroll","index":1}`, the function that was current when the event was routed. Function bindings are not flat bindings, so `binding` is `null`.
- `service_end` adds `dial_function_switch_count`, the switch records written, and `dropped_dial_function_count`. Both are zero for other profiles.

Live coalescing never merges a dial function switch, never sums steps selected through different functions, and never merges a step into the one before a switch, even when the switch itself had to be dropped. A switch dropped because the pending work item was full still took effect, because routing happens before queueing. It is counted in `dropped_event_count` and `dropped_dial_function_count`, and its report record carries `dropped_dial_function_count`.

The input owner timestamps and normalizes reports before placing them in a FIFO queue holding at most 64 waiting reports, excluding the worker's current report. The worker serializes logging and actions, so slow host actions do not block input normalization until the queue fills. A full queue fails the session and cancels pending work; reports are not silently dropped while execution continues.

Live `service_end` records distinguish received `report_count`, `queued_report_count`, logged `processed_report_count`, and `discarded_report_count`; they also distinguish `decoded_event_count`, logged `event_count`, and `discarded_event_count`. `incomplete_action_count` counts matched bindings that did not complete successfully. Those counters expose overflow and cancellation loss rather than implying that every received report produced a completed action. Replay emits the same `service_end` counters, built by the same helper, with every report counted as queued.

Live records have a 500 ms total write deadline and a 65,536-byte size limit, including the newline. A partial or failed write permanently disables that writer: retrying it could corrupt the JSONL stream. The writer owns a close-on-exec stdout duplicate, restores the original file status flags when dropped, and has one writer at a time; the application does not run concurrent stdout writes through another owner.

A failed action is logged as `action_failed` and never retried; later events still dispatch unless `--stop-on-action-error` ends the session on the first failure. A timeout, cancellation, or process error can occur after an action has taken effect. Input, action, cleanup, and output errors are combined when necessary and produce a failing exit status. A failed output stream or unavailable worker result can prevent error/footer records, so stderr and exit status remain relevant.

The `cooltek-sdk-service` crate exposes `Profile`, `InputSession`, `BindingEngine`, `ProfileSession`, and `ActionDispatcher`. `ProfileSession::route` returns a `ProfileResolution` that reports keypad navigation and dial function switches separately from host actions. `ProfileSession::resolve` keeps returning a `KeypadResolution`, in which a function switch shows up as `Unbound`. Callers provide independently established module/serial identity, nondecreasing input timestamps, and a dispatcher. `DryRunDispatcher` records validated actions; `MacOsDispatcher` executes explicit actions and supports `with_cancellation(Arc<AtomicBool>)` for a shared stop flag. Use a separate input session per physical device and reset it on disconnect, suspension, or transport failure.

The CLI loads its profile once and handles one interface per process. Automatic discovery/reconnection, profile reload, native sleep/wake observation, a menu-bar UI, and general image/display management are outside this service; the only display support is the opt-in, experimental, hardware-unverified [keypad](#keypad-display-sync-experimental) and [dial](#dial-display-sync-experimental) display sync. Live timestamps represent host read arrival, not device-native event timing. Independent normalization and bounded queues do not establish sustained high-rate throughput, prevent HID/OS buffering loss, or verify physical press timing; those require hardware and runtime testing.
