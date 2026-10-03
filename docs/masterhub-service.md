# MasterHUB binding and action service

`masterhub-service` maps M003K knob and M131E dial events to a small set of macOS actions. Profiles and replay work without a device. Live runs read one explicitly selected HID interface and default to logging planned actions. Physical input behavior and actual macOS action execution remain untested.

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

Validation checks the entire profile before printing `profile_valid`. Replay accepts the schema-1 JSONL input captures described in the [diagnostic guide](masterhub-diagnostics.md#capture-format-and-replay). It constructs no HID context, opens no interfaces, and executes no macOS actions. Both bundled input captures above are synthetic.

Replay streams reports and planned bindings as they are read. An invalid or truncated capture, missing footer, or transport-error footer produces a failing exit status even when earlier output was emitted. Check the exit status and final `service_end` before treating the stream as a complete session.

## Read an interface

Obtain its exact `path_hex` from inventory, then run a bounded preview:

```sh
cargo run -p cooltek-sdk-cli --bin masterhub-diag -- list

cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  run --profile examples/profiles/macos.json \
  --path-hex HEX_FROM_INVENTORY --duration-secs 30
```

The path contains the lossless interface bytes without a trailing NUL. The service refuses missing, ambiguous, stale, or unsupported selections; it never falls back to the first matching VID/PID. M003K and M131E interfaces are supported under both known PID generations. Each process owns one selected interface.

Without `--execute`, matched actions are validated and logged without touching volume, sending shortcuts, or opening applications. Without `--software-mode`, the service sends no configuration commands. Reads default to a 100 ms timeout; `--timeout-ms` accepts 1–1000. Optional `--max-reports` stops after that many nonempty input reports. `--duration-secs` accepts 1–86400; omitting both duration and report limits runs until interrupted or an error occurs.

On macOS, add `--execute` to enable the profile's actions:

```sh
cargo run -p cooltek-sdk-cli --bin masterhub-service -- \
  run --profile examples/profiles/macos.json \
  --path-hex DIAL_PATH_HEX_FROM_INVENTORY --duration-secs 30 \
  --execute --software-mode
```

The two flags are independent. `--execute` enables macOS actions and does not enable dial software mode. `--software-mode` requests software mode only for M131E and can be used during a dry run. Once enabling has been attempted, the service attempts to restore dial hardware mode when the session ends, including after an enable failure, action failure, input error, or output error.

Ctrl-C and SIGTERM request graceful termination. The service lets a current action finish, checks cancellation before another action, clears input state, and attempts requested dial cleanup. Forced termination can prevent cleanup. A cleanup failure is reported and causes a failing exit status; it is not silently treated as a successful session.

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

Control indexes are wire indexes. Their physical placement needs hardware checking. Dial `right` and `left` retain the Windows application's labels; their physical clockwise mapping is unverified.

A profile requires `schema_version: 1`, a nonempty `name`, and a `bindings` array. A binding requires `module`, `control`, `gesture`, and `action`; optional `serial_number` matches the exact, case-sensitive HID enumeration serial shown by inventory. The service does not query vendor serial feature reports. Omitting the serial matches any device serial, including an unavailable serial. An exact serial binding takes precedence over a wildcard for the same module, control, and gesture, independent of array order. Duplicate complete selectors are rejected.

| Module | Allowed controls | Allowed gestures |
| --- | --- | --- |
| `m003k` | `0`, `1`, `2` | `clockwise`, `counter_clockwise`, `pressed`, `released` |
| `m131e` | `0` | `right`, `left`, `pressed`, `released`, `short_press`, `long_press` |

Dial short/long gestures are classified on release after an observed press: less than 600 ms is short; at least 600 ms is long. A release emits `released` before the classified gesture, so bindings for both can execute two actions. Repeated button snapshots do not retrigger transitions; repeated rotations remain separate events. A release without a matching observed press produces no short/long gesture. Session reset clears pending presses without synthesizing actions.

Unknown reports retain their raw bytes in the log and emit no events. Malformed input is logged as `parse_error` without changing decoder/button state. Those packet diagnostics do not end a session; transport, action, and output failures do.

JSON parsing rejects unknown fields, unsupported schema versions, invalid module/gesture combinations, and invalid actions. Profiles are bounded to 1 MiB and 256 bindings. Names are at most 128 bytes; serial selectors are at most 256 bytes. Names and serial selectors cannot be empty or contain control characters.

## Actions and permissions

| Action JSON | Behavior and constraints |
| --- | --- |
| `{"type":"volume","delta":5}` | Reads current system output volume, adds signed percentage points, and clamps to 0–100. Delta must be nonzero and within −100…100; the setter specifies only output volume. |
| `{"type":"toggle_mute"}` | Reads and inverts the system output's current mute state. |
| `{"type":"shortcut","key":"c","modifiers":["command"]}` | Sends a keystroke to the current application through System Events. Key is exactly one ASCII letter, digit, or space. Distinct modifiers are `command`, `control`, `option`, and `shift`; an empty array is allowed. Named keys, punctuation, Unicode keys, and sequences are unsupported. |
| `{"type":"launch_app","bundle_id":"com.apple.finder"}` | Uses `open -b` to request an installed application by bundle ID. IDs are dotted ASCII identifiers of at most 255 bytes: at least two nonempty components, each starting with a letter/digit and containing only letters, digits, or hyphens. |

The backend validates actions again before execution. It invokes absolute `/usr/bin/osascript` and `/usr/bin/open` paths with separate arguments, without a shell or arbitrary script actions. Each subprocess has a five-second timeout and separate 16 KiB stdout/stderr limits; failure cleanup allows up to one additional second to reap a killed process. Volume and mute actions use a query followed by a setter, so one action can involve two subprocesses.

Shortcut execution may require Accessibility and Automation permissions. The identity shown by macOS depends on the launching context, such as Terminal versus a bundled application. Only explicit execution can invoke these tools or prompt for permissions; validation, replay, and dry runs do not. Permission failures preserve the system tool's diagnostic and end the session. No permission settings are changed automatically.

Apple documents the underlying [volume commands](https://developer.apple.com/library/archive/documentation/AppleScript/Conceptual/AppleScriptLangGuide/reference/ASLR_cmds.html), [System Events UI scripting and Accessibility](https://developer.apple.com/library/archive/documentation/LanguagesUtilities/Conceptual/MacAutomationScriptingGuide/AutomatetheUserInterface.html), and [Automation permissions](https://support.apple.com/en-hk/guide/mac-help/mchl108e1718/mac). These sources establish syntax and permission requirements; they do not verify this service's execution on a particular Mac or audio output.

## Output, failures, and integration

Output is JSON Lines: `service_start`, raw `report` diagnostics, normalized `event` records with planned/unbound bindings, optional `action_executed`/`action_failed`, and `service_end` counts. The service emits a planned binding before dispatch; an output failure prevents the corresponding unlogged action. `action_count` counts successfully dispatched bindings, including dry-run plans. `action_executed` records system-tool completion rather than independent verification of the resulting application or system state.

An action failure stops further dispatch and is never retried. A timeout or process error can occur after an action has taken effect. Input, action, cleanup, and output errors are combined when necessary and produce a failing exit status. A broken output stream can prevent error/footer records, so stderr and exit status remain relevant.

The `cooltek-sdk-service` crate exposes `Profile`, `InputSession`, `BindingEngine`, and `ActionDispatcher`. Callers provide independently established module/serial identity, nondecreasing input timestamps, and a dispatcher. `DryRunDispatcher` records validated actions; `MacOsDispatcher` executes explicit actions. Use a separate input session per physical device and reset it on disconnect, suspension, or transport failure.

The CLI loads its profile once and handles one interface per process. Automatic discovery/reconnection, profile reload, native sleep/wake observation, a menu-bar UI, and image/display management are outside this service. Dispatch is synchronous and timestamps are assigned when reports are read; slow actions can delay later reports and distort live press durations. Sustained high-rate rotation throughput requires hardware and runtime testing. Log writes use blocking stdout: a stalled output consumer can delay interruption and cleanup, so live output needs an active reader.
