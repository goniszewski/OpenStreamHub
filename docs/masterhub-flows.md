# Conditional flows: design

Status (23 October 2026): the `conditional` action, nesting of AND / OR groups and of if / else blocks, user-defined variables (schema 5) and the app editor with drag-and-drop are built, tested with fakes and unit and render tests only. The drag gesture itself was tried in a browser by the owner, not on hardware; the hardware check was skipped by decision. The System Events script for the front app was run once by hand. The dial function condition is not built (see below). Decisions taken with the owner:

- Shape: one new `conditional` action (if / else), not a graph.
- Conditions: OBS state, focused app, system state, dial function.
- Evaluated in the service at press time, so flows work with the app closed.

The official app has no conditional flows, so this is our own extension. It needs no change to the device protocol.

## The action

```json
{
  "type": "conditional",
  "when": { "obs_recording": true },
  "then": [ { "type": "obs_record", "op": "stop" } ],
  "else": [ { "type": "obs_record", "op": "start" } ]
}
```

- `when` holds one condition (see below), or `{"all":[…]}` (AND) / `{"any":[…]}` (OR) of 2 to 8 plain conditions. They nest and mix (see Nesting), so the grouping is the precedence; every member is read (no early exit), and one that cannot be read fails the press. `not` is covered by swapping `then` and `else`, or by writing the condition with `false`.
- `then` has 1 to 8 steps, `else` is optional (0 to 8). A missing or empty `else` means do nothing.
- Steps are the same executable actions a `multi_action` step may be, with the same exclusions (no nesting, no `macro`, no `profile`, no `timer`, no `*_from_position` templates). A `conditional` is therefore not a step of another `conditional` or of a `multi_action`. Keeps one deadline and no loops.
- A branch runs like a `multi_action`: steps in order, fixed spacing, a failure stops the rest of the branch.
- Binds wherever a `multi_action` does (it is not a fader template).
- Schema: this is a new action type, so the profile schema is now 4. Older builds then refuse the profile (the accepted behaviour from schema 3).

## Conditions

One condition per `when`, a single key with a value:

| Group | Key | Value | Read from |
| --- | --- | --- | --- |
| OBS | `obs_streaming`, `obs_recording`, `obs_virtual_camera`, `obs_replay_buffer` (built) | bool | `Get*Status` over the existing obs-websocket session |
| OBS | `obs_scene` | scene name | `GetCurrentProgramScene` (already used) |
| OBS | `obs_input_muted` | `{ "input": name, "muted": bool }` | `GetInputMute` |
| System | `output_muted` (built) | bool | the same reader `toggle_mute` uses |
| System | `output_volume_at_least` (built) | percent 0 to 100 | the same reader the volume actions use |
| System | `timer_running` (built) | bool | the service's own timer table; counts only timers started by this module's bindings |
| Dial | `dial_function` (not built) | function name | the dial's process; see below |
| Focus | `focused_app` (built) | bundle id string | System Events, read in the service at press time |

## Evaluation rules

- A condition is read once, right before the branch starts, never re-checked mid-branch.
- **A condition that cannot be read is an error, not "false".** OBS unreachable, no endpoint, authentication rejected: the press fails with the same error text the OBS actions give today, and neither branch runs. Otherwise a missing OBS would silently take the `else` branch and start a recording that is already running.
- The read gets the same overall deadline as the branch; the total (reads plus steps plus spacing) must stay inside the action timeout already used for `multi_action`.
- Events: `action_executed` reports which branch ran (`then` or `else`) so the test mode and the logs show it.

## Dial function: not built, and why

Each module runs in its own process, and the dial's bindings already live inside its functions, so a function is only active for bindings that belong to it. A `dial_function` condition on a dial binding would always be true for the function it sits in. It would only mean something on another module (for example a key that acts differently while the dial is on Volume), and that process cannot see the dial. It needs the desktop app to relay the dial's function to the other modules' services over their control channel, plus a new control message and a state in the service. That is the cross-module routing that was left to the app when one process per module was decided, so it is a separate piece of work.

## Where each condition comes from

- OBS: the service already holds the obs-websocket connection; only the request calls are new.
- System, timer and dial function: state the service already owns or can read.
- **Focused app (built as the service-side option).** The frontmost-app watcher lives in the desktop app (`focus` worker, [focus rules](masterhub-focus-rules.md)), not in the service. Two options, to settle when this is built: the app pushes the current bundle id to the service on its control channel (cheap at press time, but stale or missing when the app is closed), or the service reads it itself at press time on macOS (about one extra process or API call per press, works with the app closed). Recommended: read it in the service, so the "works with the app closed" decision holds for every condition.

## Editor (desktop app)

- New action type "If / else" in the action picker.
- Condition picker: group, then key, then value. OBS conditions are disabled with a hint when the profile has no OBS endpoint.
- Two step lists, "If true" and "Otherwise", reusing the multi-action step editor and its limits.
- The preview line reads like "If recording: stop recording, otherwise start recording".

## Validation

- Reject `conditional` with an empty `then`, more than 8 steps in either branch, or a step that a `multi_action` would reject.
- An OBS condition in a profile is not rejected for a missing endpoint: the endpoint is a run option, not a profile field, so it stays a runtime `action_unavailable`, as for the OBS actions.
- Unknown condition keys fail strict parsing (`deny_unknown_fields`), like every other action.

## Tests and hardware checks

- Unit: validation matrix (limits, forbidden steps, empty branches), serde round trips, branch choice for each condition with a fake reader, unreadable condition runs neither branch.
- Fake obs-websocket peer (already used by the OBS tests) for the status requests, including auth failure and a dropped connection.
- Hardware, on the M4315 with a real OBS: a record toggle key in both states, OBS quit mid-session (must fail, not take `else`), a scene-dependent key, output mute, focused app.

## Suggested build order

1. `conditional` action, validation, schema 4, executor with the OBS record/stream/virtual-camera/replay-buffer conditions (covers the common toggle case on its own).
2. Remaining OBS conditions (`obs_scene`, `obs_input_muted`).
3. System and dial conditions.
4. Focused app.
5. Editor.
6. Hardware pass, then docs and release notes.

Each step is shippable, and step 1 is useful without the rest.

## Nesting (decided 23 October 2026)

- `all` / `any` nest in each other and mix: at most 3 levels, at most 16 tests in all, 2 to 8 members per group. The nesting is the grouping, so there are no precedence rules. Every test is still read, in order, with no early exit.
- A `conditional` may be a step of a `then` or `else` list of another `conditional` (3 levels at most, 32 steps in all branches counting nested ones). It is still never a step of a `multi_action`, and nothing loops. A nested one is read when it is reached, on the same 15 s deadline as the outer one.
- No schema change: it only loosens what schema 4 allows. An older schema-4 build refuses such a profile on validation.
- Editor: conditions and steps are blocks you drag to reorder, move between groups and lists, or into a nested group / if-else.

## Variables (decided 23 October 2026)

- Declared in the profile: `"variables":[{"name":"takes","value":0},{"name":"live","value":false}]`. A value is a bool, a whole number (-1 000 000 to 1 000 000) or text (up to 64 bytes). At most 16; a name is 1 to 32 characters of `a-z`, `0-9`, `_` starting with a letter. The kind never changes after the declaration.
- Per profile and per module process. Values start at the declared value when the service starts and are kept across presses, not across a service restart (a profile switch restarts it). Not shared between modules.
- Actions (steps of a `multi_action` or a branch too): `set_variable {name, value}`, `toggle_variable {name}` (bool), `add_to_variable {name, amount}` (number; a result outside the range stops at the limit).
- Conditions: `variable_is {name, value}` (any kind), `variable_at_least {name, value}` (number).
- A name that is not declared, or an action or test of the wrong kind, is refused when the profile is validated, not at press time.
- Schema 5.

## Not in scope

Loops, waiting for a state ("wait until recording"), conditions on a held key or gesture, variables shared between modules, variables saved across a service restart, `{name}` placeholders in text or URLs, and user scripts.
