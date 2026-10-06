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

Live M4315 runs reject schema-2 keypad profiles before opening the selected device because synchronized physical display updates are not implemented. Other modules can use their flat bindings from the same profile. Schema-1 live input retains its existing behavior.

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

Without `--execute`, matched actions are validated and logged without touching volume, sending shortcuts, or opening applications. Without `--software-mode`, the service sends no configuration commands. Reads default to a 100 ms timeout; `--timeout-ms` accepts 1–1000. Optional `--max-reports` stops after that many nonempty input reports. `--duration-secs` accepts 1–86400; omitting both duration and report limits runs until interrupted or an error occurs.

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

Requested IPS module hardware-mode restoration is attempted once after the worker phase. Cleanup failures produce a failing exit status. HIDAPI feature writes are synchronous and expose no timeout here, so the worker/log deadlines do not establish a whole-process shutdown bound. Forced termination can prevent cleanup.

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
| `{"type":"shortcut","key":"c","modifiers":["command"]}` | Sends a keystroke to the current application through System Events. Key is exactly one ASCII letter, digit, or space. Distinct modifiers are `command`, `control`, `option`, and `shift`; an empty array is allowed. Named keys, punctuation, Unicode keys, and sequences are unsupported. |
| `{"type":"launch_app","bundle_id":"com.apple.finder"}` | Uses `open -b` to request an installed application by bundle ID. IDs are dotted ASCII identifiers of at most 255 bytes: at least two nonempty components, each starting with a letter/digit and containing only letters, digits, or hyphens. |

The backend validates actions again before execution. It invokes absolute `/usr/bin/osascript` and `/usr/bin/open` paths with separate arguments, without a shell or arbitrary script actions. Each subprocess has a five-second timeout and separate 16 KiB stdout/stderr limits; failure or cancellation cleanup allows up to one additional second to reap a killed process. Volume and mute actions use a query followed by a setter, so one action can involve two subprocesses.

Shortcut execution may require Accessibility and Automation permissions. The identity shown by macOS depends on the launching context, such as Terminal versus a bundled application. Only explicit execution can invoke these tools or prompt for permissions; validation, replay, and dry runs do not. Permission failures preserve the system tool's diagnostic and end the session. No permission settings are changed automatically.

Apple documents the underlying [volume commands](https://developer.apple.com/library/archive/documentation/AppleScript/Conceptual/AppleScriptLangGuide/reference/ASLR_cmds.html), [System Events UI scripting and Accessibility](https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/AutomatetheUserInterface.html), and [Automation permissions](https://support.apple.com/en-hk/guide/mac-help/mchl108e1718/mac). These sources establish syntax and permission requirements; they do not verify this service's execution on a particular Mac or audio output.

The [controlled smoke examples](masterhub-macos-smoke.md) use the same dispatcher without HID. Audio checks restore and verify an exact snapshot; scratch keystroke checks require independent observation. Neither example upgrades a successful system-tool return to verified host or hardware behavior.

## Output, failures, and integration

Output is JSON Lines: `service_start`, raw `report` diagnostics, normalized `event` records with planned/unbound bindings and resolved `action` values, optional `action_executed`/`action_failed` (each carrying `schema_version`), and `service_end` counts. The service emits the original binding and concrete resolved action before dispatch; an output failure prevents the corresponding unlogged action. `action_count` counts successfully dispatched bindings, including dry-run plans. `action_executed` records system-tool completion rather than independent verification of the resulting application or system state.

The input owner timestamps and normalizes reports before placing them in a FIFO queue holding at most 64 waiting reports, excluding the worker's current report. The worker serializes logging and actions, so slow host actions do not block input normalization until the queue fills. A full queue fails the session and cancels pending work; reports are not silently dropped while execution continues.

Live `service_end` records distinguish received `report_count`, `queued_report_count`, logged `processed_report_count`, and `discarded_report_count`; they also distinguish `decoded_event_count`, logged `event_count`, and `discarded_event_count`. `incomplete_action_count` counts matched bindings that did not complete successfully. Those counters expose overflow and cancellation loss rather than implying that every received report produced a completed action. Replay emits the same `service_end` counters, built by the same helper, with every report counted as queued.

Live records have a 500 ms total write deadline and a 65,536-byte size limit, including the newline. A partial or failed write permanently disables that writer: retrying it could corrupt the JSONL stream. The writer owns a close-on-exec stdout duplicate, restores the original file status flags when dropped, and has one writer at a time; the application does not run concurrent stdout writes through another owner.

An action failure stops further dispatch and is never retried. A timeout, cancellation, or process error can occur after an action has taken effect. Input, action, cleanup, and output errors are combined when necessary and produce a failing exit status. A failed output stream or unavailable worker result can prevent error/footer records, so stderr and exit status remain relevant.

The `cooltek-sdk-service` crate exposes `Profile`, `InputSession`, `BindingEngine`, and `ActionDispatcher`. Callers provide independently established module/serial identity, nondecreasing input timestamps, and a dispatcher. `DryRunDispatcher` records validated actions; `MacOsDispatcher` executes explicit actions and supports `with_cancellation(Arc<AtomicBool>)` for a shared stop flag. Use a separate input session per physical device and reset it on disconnect, suspension, or transport failure.

The CLI loads its profile once and handles one interface per process. Automatic discovery/reconnection, profile reload, native sleep/wake observation, a menu-bar UI, and image/display management are outside this service. Live timestamps represent host read arrival, not device-native event timing. Independent normalization and bounded queues do not establish sustained high-rate throughput, prevent HID/OS buffering loss, or verify physical press timing; those require hardware and runtime testing.
