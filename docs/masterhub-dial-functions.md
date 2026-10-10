# M131E dial functions

Design recorded on 7 October 2026. The routing lives in `cooltek-sdk-service`. CLI replay and live runs, the desktop app's replay and preview, and the profile editor use it; the dial screen and a hardware check are still open (see the [integration checklist](#integration-checklist)). Nothing here has been tested on hardware.

## Behavior in the official app

Static analysis of MasterHUB 1.0.0.113 ([dial input notes](research/masterhub-1.0.0.113-static-analysis.md#dial-input)) shows that the app decides long press on the host: a release at least 600 ms after the recorded press is long; anything shorter is short. A short click switches the dial's current function. A long press runs the press action configured for the current function. The firmware sends only press, release, and rotation. It has no long-press or function-switch event.

## Profile shape

A profile can add an optional `dial_functions` array. Each entry is a named group of M131E bindings:

```json
"dial_functions": [
  {
    "name": "Volume",
    "bindings": [
      {"gesture": "right", "action": {"type": "volume", "delta": 5}},
      {"gesture": "right", "serial_number": "desk-dial", "action": {"type": "volume", "delta": 1}},
      {"gesture": "left", "action": {"type": "volume", "delta": -5}},
      {"gesture": "long_press", "action": {"type": "toggle_mute"}}
    ]
  },
  {
    "name": "Apps",
    "bindings": [
      {"gesture": "long_press", "action": {"type": "launch_app", "bundle_id": "com.apple.finder"}}
    ]
  }
]
```

A complete example that also has flat knob bindings and a schema-2 keypad is in [`crates/cooltek-sdk-service/tests/fixtures/dial-functions.json`](../crates/cooltek-sdk-service/tests/fixtures/dial-functions.json).

There is no `module` or `control` field, because only the M131E dial, control 0, has functions.

### Validation

| Rule | Limit or reason |
| --- | --- |
| Number of functions | 1 through 16 (`DialFunction::MAX_FUNCTIONS`). An empty array is rejected. `null` counts as absent. |
| Function name | Nonempty, at most 64 bytes (`DialFunction::MAX_NAME_BYTES`), unique within the profile, and case-sensitive. Leading and trailing whitespace is rejected, so `"A"` and `"A "` cannot both exist. Control characters and Unicode format characters (General Category `Cf`) are also rejected. `Cf` covers the bidi embedding, override, isolate, and mark controls, zero-width characters, the byte order mark, and the soft hyphen. Spaces inside a name and non-ASCII letters are allowed. These rules apply only to function names. Profile names, page names, and tile labels keep their existing rules. |
| Empty function | `"bindings": []` is accepted on purpose. Every gesture in that function returns `Unbound`, so it acts as a "dial does nothing" function in the cycle. |
| Binding gestures | `right`, `left`, and `long_press` only. |
| `short_press` | Rejected. A short click always switches functions, so it cannot also run an action. |
| `pressed`, `released` | Rejected. The input session sends both for every click, so a binding on either would fire on every function switch. |
| Selectors | `(gesture, serial_number)` must be unique within a function. The serial follows the usual selector rules and additionally cannot be only whitespace. `DeviceContext` treats a blank device serial as absent, so such a binding could never match. Flat bindings keep their existing serial rule, so profiles that were accepted before still load. |
| Actions | Must be executable. `volume_from_position` is rejected because the dial has no position value. |
| Flat `bindings` | Must not contain any `m131e` binding while `dial_functions` is present. All dial bindings then live in the functions, so no rule decides between a flat binding and a function binding. |
| Total bindings | Function bindings count toward the profile's limit of 256 bindings, together with flat and keypad bindings. |
| Unknown fields | Rejected on functions and their bindings, as everywhere else in a profile. |

### Why there is no schema 3

`dial_functions` is an optional field and is allowed in schema 1 and schema 2:

- **Old readers fail safely.** `Profile` already uses `deny_unknown_fields`. A build without this feature rejects a profile that has `dial_functions` with an `unknown field` error. It does not load the profile while silently ignoring the dial functions. A new schema version would produce the same rejection, only with a different message.
- **Existing files do not change.** The field is skipped when absent. Every existing schema-1 and schema-2 profile parses and serializes exactly as before; a test re-encodes the bundled examples and compares them.
- **No migration needed.** Schema 2 needed `into_v2` because it moved M4315 bindings into keypad tiles. Dial functions move nothing: a profile either has no functions, with flat M131E bindings as before, or has functions and no flat M131E bindings. `into_v2` keeps `dial_functions` unchanged.
- **Downstream code keeps working.** The CLI and app compare `schema_version` with 1 and 2 in several places. A version 3 would need changes in all of them, while an optional field needs none.

The cost: a reader can tell that a profile uses dial functions only by looking for the field, not from the version number. Editors that rewrite profiles must carry the field through (see the checklist).

## Runtime routing

`ProfileSession` holds the current function index. It starts at 0, the first function in the array.

| M131E event | With `dial_functions` |
| --- | --- |
| `short_press` | Moves to the next function and wraps from the last back to the first. Returns `ProfileResolution::DialFunction` with `from_function`, `to_function`, the new `index`, and `count`. No host action. |
| `right`, `left`, `long_press` | Uses the current function's binding. An exact serial match beats a wildcard, whatever the order. Returns `ProfileResolution::Action` with `dial_function` set to the current name, or `Unbound`. Other functions are never consulted. A long press does not change the current function. |
| `pressed`, `released` | Always `Unbound`. |

A dial click arrives as `pressed`, `released`, `short_press`. With functions, it produces two `Unbound` results and one `DialFunction` result, and never a host action.

Profiles without `dial_functions` keep their flat M131E behavior, including `short_press` bindings. Other modules and keypad page routing are unchanged.

### API

- `ProfileSession::route(device, event) -> ProfileResolution` is the full result. It has the variants `Action { action, page_id, dial_function }`, `Navigation { navigation }` (keypad), `DialFunction { navigation }`, and `Unbound`. Its JSON is tagged by `type`, so a switch serializes as `{"type":"dial_function","navigation":{...}}`.
- `ProfileSession::resolve(device, event) -> KeypadResolution` is unchanged for existing callers. It calls `route` and converts the result with `From<ProfileResolution> for KeypadResolution`. A dial function switch still advances the state but shows up as `Unbound`, and the `dial_function` name is dropped. Existing callers therefore never dispatch anything for a short click.
- `dial_function_index()` and `current_dial_function()` return `None` for profiles without functions.
- `set_dial_function(index)` selects a function directly, for example from a UI. An index out of range leaves the state unchanged.
- `reset()` returns to the first function, the keypad root, and no held keys. A new `ProfileSession`, such as after a profile switch or reload, also starts at the first function. The index is never saved in the profile.
- `binding_for` returns `None` for M131E events when the profile has functions, like M4315 under schema 2.
- `BindingEngine::new` rejects profiles that have functions: the stateless engine cannot track the current function.

### Limitations

- One index per session. Two dials routed through the same `ProfileSession` share it, as keypads share the current page. Each CLI process owns one interface, so that does not happen in the CLI today.
- The official app's function switch probably also updates the dial screen. This crate only reports the switch. Showing it on the display is a caller's job (see the checklist).
- **Decided:** CLI and app live runs accept these profiles. With the app driving the display (Module control App), the dial screen shows the active function's name by default — every profile's `dial_display` content defaults to the function name; the standalone service draws it only behind `--display-sync --dial-content function`. A click never dispatches, so without display sync the user has to keep track of which function is active. `service_start` records that dial functions are active and that the function resets on restart, and a live run prints the same notice on stderr.
- The index lives only in the running process. Every CLI process restart goes back to the first function, and the app restarts the process on wake from sleep, reconnect, crash retry, and focused-app profile switch.
- The 600 ms threshold and the click sequence come from static analysis and have not been checked on hardware.

## Integration checklist

Status on 7 October 2026. Item numbers match the original list.

1. **CLI event planning** (`crates/cooltek-sdk-cli/src/service_events.rs`): done. `plan_report` calls `route` and keeps the `ProfileResolution`. A switch is logged as an `event` with status `dial_function` and `navigation` holding `from_function`, `to_function`, `index`, and `count`, with no action and no dispatch. Other dial events and `action_executed`/`action_failed` carry `dial_function: {name, index}`. The records stay at `schema_version` 1 because every field is additive. The records are documented in [the service docs](masterhub-service.md#dial-function-records).
2. **CLI live coalescing** (`crates/cooltek-sdk-cli/src/live_runtime.rs`): done. Two switches never merge, steps from different functions never merge, and a step never merges into the one before a switch, even when the pending item was full and the switch record was dropped. A dropped switch is counted in `dropped_dial_function_count` (report and `service_end`). Unit tests cover rotation, click, rotation under coalescing, a switch dropped under pressure, and a full live run with a capacity-1 queue.
3. **CLI dispatch**: done. Only `ProfileResolution::Action` is dispatched. Tests check that a click never dispatches, that a long press runs the current function's action, and that functions wrap around in replay output.
4. **CLI live gating** (`live_bindings` in `masterhub-service.rs`): decided and done. Live M131E runs accept `dial_functions`. `service_start.details.dial_functions` records the names, `initial_index: 0`, and `resets_on_restart: true`, and the run prints a notice on stderr.
5. **CLI fixture and docs**: done. [`examples/profiles/dial-functions.json`](../examples/profiles/dial-functions.json) and the synthetic capture `crates/cooltek-sdk-cli/tests/fixtures/m131e-dial-functions-synthetic.jsonl` cover short and long clicks, both sides of the 600 ms threshold, wrap-around, and an unbound long press. An integration test in `crates/cooltek-sdk-cli/tests/service.rs` asserts the JSONL.
6. **App replay** (`apps/open-stream-hub/src-tauri/src/replay.rs`): done. Replay and preview call `route`. `ReplayEvent` adds optional `dial_function` and `dial_function_switch` fields, which are absent for other profiles. `preview_binding` takes an optional `dialFunction` index, applied with `set_dial_function`; without it the first function is previewed as before. A `short_press` preview returns the planned switch and no action. The shared fixture is `apps/open-stream-hub/fixtures/dial-function-preview.json`.
7. **Frontend types** (`apps/open-stream-hub/src/contracts.ts`): done. `Profile.dial_functions`, and `ReplayEvent.dial_function` and `dial_function_switch` (shared `DialFunctionSwitch`), match the Rust serialization. The editor and the Profiles replay table use the shared type, and a contract test pins the field names.
8. **Editor**: done. `DialFunctionsPanel` in the encoder inspector adds, renames, reorders, and deletes functions (1 to 16, with inline name checks that mirror the service). Since 10 October 2026 the dial editor is a two-tab card — Functions and Display; the Display tab edits the profile's `dial_display` setting (the dial screen face, including an uploaded picture) and previews it with the same rasteriser the device gets. It edits each function's Right, Left, and Long press bindings with an optional serial, using the shared action picker. All edits go through the draft's undo history. Without functions the profile is unchanged until the user adds one, and deleting the last function removes `dial_functions`. Moving flat encoder bindings into a function is a separate button. It names every binding it drops: gestures no function can hold (Short press, Press, Release) and bindings whose gesture and serial the target function already has. Deleting a function or moving bindings asks for no confirmation; undo restores them. Previews sit in the same editor as the bindings: each function’s gestures and its Short press can be tested where the function is edited. While functions exist, the encoder inspector offers no flat editor — the dial editor replaces it. The binding list and its warnings include function bindings, and they warn about flat encoder bindings next to functions, functions without bindings, and invalid or duplicate names.
9. **Simulation and status UI**: partly done. The editor previews any function's Right, Left, Long press, and Short press, and shows which function is previewed. The Profiles replay table shows the active function on each row and a distinct "Dial function: A → B (2/3)" row for a switch, which is not counted as unbound. The service activity log shows the current function on events and executed actions, and each switch as its own line. **Not done:** there is no dedicated live status field for the current function (it appears only in the activity log), and the Devices simulation still previews only the first function.
10. **Display**: the faces drew correctly on a real M131E on 8 October 2026; short-click cycling through three functions also worked ([results](masterhub-hardware-results.md#m131e-dial)). `masterhub-service run --display-sync --dial-content function` draws the active function's name and position on the dial screen after each switch, at most once a second ([display sync](masterhub-display-sync.md#dial-opt-in-not-verified)). Since 10 October 2026 the desktop app derives `--dial-content` from each profile's own `dial_display` setting — the default is the function name, and the app's Display tab edits it; the global Settings choice (earlier, an experimental toggle) is gone. The `image` face has not run on hardware.
11. **Hardware check**: not done. The click sequence, the 600 ms threshold, and that physical clicks never run an action still need checking on a device.
