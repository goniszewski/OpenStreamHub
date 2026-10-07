# M131E dial functions

Design recorded on 7 October 2026. The routing exists in `cooltek-sdk-service` only. The CLI replay output, the desktop app, and the profile editor do not use it yet; see the [integration checklist](#integration-checklist). Nothing here has been tested on hardware.

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
- **Open question:** CLI and app live runs already accept these profiles, although the dial screen cannot show the current function yet. The user has to keep track of which function is active.
- The index lives only in the running process. Every CLI process restart goes back to the first function, and the app restarts the process on wake from sleep, reconnect, crash retry, and focused-app profile switch.
- The 600 ms threshold and the click sequence come from static analysis and have not been checked on hardware.

## Integration checklist

The CLI and the app still call `resolve`. They build and their tests pass without changes. Running a profile with `dial_functions` through them today behaves like this:

- **CLI replay and live runs** switch functions correctly and run the current function's rotation and long-press actions. The event log shows a short click as `unbound` with `binding: null`, and actions selected through a function also have `binding: null` and no function name. The output is correct but hard to follow.
- **Desktop app replay and dry runs** behave the same way. The editor cannot create or show functions.

What remains:

1. **CLI event planning** (`crates/cooltek-sdk-cli/src/service_events.rs`): call `session.route` instead of `resolve`, and store a `ProfileResolution` in `PlannedEvent`. Log `DialFunction` as status `"dial_function"`, with `navigation` holding `from_function`/`to_function`/`index`/`count`, no action, and no dispatch, like keypad navigation. Add `dial_function` to planned action records. Bump the event record's `schema_version` only if consumers need it.
2. **CLI live coalescing** (`crates/cooltek-sdk-cli/src/live_runtime.rs`, the merge around `KeypadResolution` comparisons): never merge two `DialFunction` results. Each one is a separate state change.
3. **CLI dispatch**: dispatch only `ProfileResolution::Action`. Add tests: a click never dispatches, a long press dispatches the current function's action, and the function wraps around in replay output.
4. **CLI live gating** (`live_bindings` in `masterhub-service.rs`): decide whether live M131E runs accept `dial_functions` before the display shows the current function. M4315 folders were refused live for the same reason. Today they are accepted.
5. **CLI fixture and docs**: add an example profile under `examples/profiles/` and a synthetic M131E capture that has short and long presses. Document the new replay record in `docs/masterhub-service.md`.
6. **App replay** (`apps/open-stream-hub/src-tauri/src/replay.rs`): switch to `route` and send the current function and each switch to the frontend. Add the contract fixture in `contract_fixtures.rs`. `preview_binding` builds a fresh `ProfileSession` for each preview, so previews always use the first function. The preview request needs a function index, applied with `set_dial_function`, before it can preview the other functions.
7. **Frontend types** (`apps/open-stream-hub/src/contracts.ts`): partly done. The frontend `Profile` type carries `dial_functions`, and `apps/open-stream-hub/src/dial-functions.test.ts` shows that the field survives draft, undo, duplicate, import, and export. What remains is the editor UI (item 8).
8. **Editor**: add, rename, reorder, and delete functions with the limits above, and bind right/left/long press per function. Do not offer `short_press` while functions exist. Converting existing flat M131E bindings into a first function needs an explicit user action.
9. **Simulation and status UI**: show the current function, reset it when the session or profile changes, and optionally offer `set_dial_function`.
10. **Display**: decide whether a switch pushes a per-function image or label to the dial screen. This is display-sync work and is not covered here.
11. **Hardware check**: confirm the click sequence, the 600 ms threshold, and that physical clicks never run an action.
