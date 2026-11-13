# Plugins: research and proposed design

Status (13 November 2026): **steps 1 to 3 and 5 built**: preset packs, the `run_program` action, the plugin protocol with action providers, and community readiness (manifest schema 1, the first-enable dialog, Keychain secrets; author guide: [writing a plugin](plugins.md)). Of steps 4 and 6, input events ([4a](#step-4a-input-events)), plugin conditions ([4b](#step-4b-plugin-conditions)) and installing from Settings ([6a](#step-6a-installing-a-plugin)) are built; plugin-supplied key images and a registry are not started. Plugins are not in the official MasterHUB 1.0.0.113 either: there "plugin" means built-in integrations (OBS, Twitch, Discord, Streamlabs), and a plugin SDK is absent ([app features](research/masterhub-1.0.0.113-app-features.md)). So plugins would be our own extension, not a parity item, and need no change to the device protocol.

## What exists today

- **Actions are a closed Rust enum.** `Action` in `crates/cooltek-sdk-service/src/profile.rs` is tagged by `type` and uses `deny_unknown_fields`. It is executed in `macos.rs` and validated when a profile loads (schema 5 today). Nothing outside the app can add an action.
- **Only the module service processes execute actions.** Each module runs as its own `masterhub-service` child process (`apps/open-stream-hub/src-tauri/src/service.rs`). The app writes a profile snapshot, starts the process, relays its JSONL stdout and writes control messages to its stdin. It never runs an action itself. Live / Test mode is applied by restarting that process.
- **A local socket exists** (`apps/open-stream-hub/src-tauri/src/ingress.rs`): JSON lines, a 0700 directory, one token that is new at every launch. `osh` is its client. It supports `ping`, `set_variable` and `get_variables`, and refuses outside writes to `agent_decision` other than `none`. It is built for short requests: each line must arrive within 2 s, lines are at most 8 KB, and at most 8 connections are served at once.
- **Variables feed the UI.** Key faces can follow a variable and if / else conditions can read one. `set_variable` is sent to every running module and succeeds only when a module declares the variable, so a profile must declare the variables an outside tool sets.
- **Data-only packs exist.** App preset packs (`apps/open-stream-hub/presets/*.json`, checked by `preset-packs.ts`) are ready-made profiles with a licence and a source, limited to shortcuts, media, system, volume and simple OBS actions, and they run only as a dry run until reviewed. They are bundled at build time; a pack cannot yet be added by dropping a file in a folder.
- **Live sessions confirm capabilities.** The Execute confirmation lists what a profile can do, and `capabilities.rs` pauses a Live session for a new confirmation when a saved profile adds a capability the running one did not have.

So data going **into** the deck works: an outside tool sets a variable and faces and conditions react. What is missing:

- A key press cannot call an arbitrary external program. The only route is `terminal_command`, which opens a Terminal window and is allowed only as a key's own action, never as a `multi_action` or `conditional` step.
- A third party cannot add an action, a condition or a key image to the editor.
- The socket is short request / response. A plugin cannot stay connected, and cannot be told that a key was pressed.

## What a plugin could be

1. **Action provider.** A press does something (Home Assistant, Spotify, a webhook).
2. **State provider.** It pushes variables that drive faces and conditions (CPU load, now playing, meeting status).
3. **Condition provider.** It adds new checks to if / else.
4. **Event consumer.** It is told about input on the controls bound to its own actions.
5. **Face renderer.** It supplies images for keys or the dial (a clock, a graph). The heaviest kind.
6. **Data-only pack.** Starter profiles and icons, no code. The cheapest, and carries no execution risk.

## How others do it

- **Stream Deck** (Elgato SDK docs). A plugin is a folder with a `manifest.json` listing its actions, icons and supported controllers. It runs as its own process, Node.js by default, and connects back over a local WebSocket; the app passes it a port, a plugin UUID and a register event. It receives events (key down, dial rotate) and sends commands (set title, set image, settings). Configuration UI is plugin-supplied HTML (the "property inspector"). Native binaries are possible but "not recommended".
- **Implementation techniques**, from general sources on plugin architectures:

| Approach | For | Against |
| --- | --- | --- |
| Separate process over IPC | Any language, crash isolation | Per-call overhead, no real sandbox |
| WebAssembly (wasmtime, Extism) | Sandboxed, one binary format | Large dependency, awkward for network and OS calls |
| Embedded scripting (Lua, Rhai, JS) | Easy for users | Adds a runtime; sandboxing is our job |
| Dynamic libraries | Fastest | Unsafe ABI, a crash takes the app down |

## Proposal: plugins as separate processes, hosted by the app

Why: plugins can be a shell script, Node, Python or Rust, no runtime is added to the app, and a crashing plugin takes down neither the app nor a module process. The protocol reuses the ingress style (JSON lines, a 0700 directory, tokens), so a plugin is conceptually "`osh` that stays connected and can also receive invokes". It keeps the existing rules: only a module service process decides to run an action, and only the deck answers `agent_decision`.

**Audience.** The maintainer writes the first plugins; community plugins come later. A stable, versioned manifest schema, author docs and the full trust UI come before any community distribution (see the suggested order); they are built ([as built](#trust-ui-as-built)).

### Decisions

| Question | Decision |
| --- | --- |
| Where plugin processes run | The app starts one instance of each plugin. Module processes reach it through the app. |
| How plugins connect | A separate plugin socket with a per-plugin token. The ingress socket keeps its short-request limits. |
| Plugin actions as steps | Allowed in `multi_action` and `conditional`, under the step deadline. |
| A "run program" action | Ships first, on its own, before the plugin protocol. |
| Variables in parameters | Text parameters only, when the manifest allows it, substituted at press time. |
| Plugin identity in profiles | A reverse-DNS id and a minimum version. |
| Input events | Only for controls bound to the plugin's own actions. |

### Manifest

`plugins/<id>/plugin.json` in the app data folder:

- `id` (reverse-DNS, such as `com.example.homeassistant`), `name`, `version`, and the command that starts the plugin.
- `actions[]`: id, label, icon, and typed parameters (text, number, bool, choice). A text parameter may set `variables: true` to allow variable substitution.
- `variables[]` it provides, and `conditions[]` it offers (later phase).
- `permissions`: what it says it needs (network hosts, which variables it writes, whether it runs programs).

### Lifecycle

The app starts a plugin when a running profile uses it, and stops it when no running profile uses it, and on quit. The plugin gets the plugin socket path and its own token in the environment. A token is new at every start and is valid only for that plugin's id. If the plugin crashes, the app restarts it with backoff. Invokes that arrive while it is down fail at once.

One instance serves all modules, so a state provider runs once, and a Test / Live restart of a module process does not restart its plugins.

### Routing a press

1. A module service process runs `Action::Plugin` and writes an invoke request to its stdout, with a request id.
2. The app forwards it to the plugin over the plugin socket.
3. The plugin answers with `result`. The app writes the result to the service's stdin with the same id.
4. The service treats the result like any other action outcome.

The service, not the app, decides that the press runs. The app only forwards the request, as it already relays service records.

### Protocol

A separate socket, `<app data>/plugins/plugins.sock`, in a 0700 directory like ingress. A connection is long-lived and authenticated by the plugin's `hello` with its token. One JSON object per line, in both directions:

- Plugin to app: `hello {id, token}`, `set_variable`, `result {request, ok, error?}`, `log`.
- App to plugin: `invoke {request, action, params}`; later, events for the controls bound to its actions.

The plugin socket has its own limits (line size, one connection per plugin, a heartbeat in place of the 2 s ingress line timeout). The ingress socket and `osh` stay unchanged.

An invoke has a deadline like a `multi_action` step (15 s overall, `MULTI_ACTION_DEADLINE`), and a failed or late result fails the press, as other actions do.

### Profile format

One new variant, `Action::Plugin { plugin, min_version, action, params }`. `plugin` is the manifest's reverse-DNS id. Params are validated against the manifest. Rules:

- A profile that uses a missing plugin, or one older than `min_version`, loads with a clear warning and does not run that key. The same applies to an imported profile.
- A build without plugin support refuses the profile, because it does not know the action type. This is how `terminal_command` shipped within schema 5, so a new action does not need a schema bump; one is needed only if a change would make an older build misread a profile (`Profile::CURRENT_SCHEMA`).
- It works as a step of `multi_action` and of a conditional, under the step deadline. The reason `terminal_command` is barred (it opens a window) does not apply.

### Variables in parameters

A text parameter whose manifest entry sets `variables: true` may contain variable references, substituted when the key is pressed. The log records the substituted value. No template syntax exists in profiles today; it is defined as part of this work and reused by "run program" arguments. Number, bool and choice parameters are static.

### Editor

Generate the form from the manifest's parameter types. Do not allow plugin-supplied HTML at first: it is the riskiest part of the Stream Deck model (script running inside the app's webview).

### Safety

- Test mode never invokes a plugin; it only logs "Would run: ...", as for every other action.
- Each plugin action is a capability (`capabilities.rs`): the Execute confirmation names the plugin and action, and adding one to a running Live profile pauses the session for confirmation.
- A first-enable dialog lists what the manifest declares.
- Plugin secrets go in the Keychain (`keychain.rs`), not in the profile.
- Plugins cannot write `agent_decision` allow / deny, and cannot press keys on the deck's behalf.
- A plugin hears only presses and turns on controls bound to its own actions. It cannot observe other input.
- **A plugin runs as you.** Without OS sandboxing, manifest permissions are consent information, not enforcement. The docs and the dialog must say so.

## Suggested order

1. **Data-only packs.** Built: the app preset packs, and packs dropped into `packs/` in the app data folder, checked by the same rules (see `apps/open-stream-hub/presets/README.md`).
2. **A "run program" action** (see below). A small change that covers much of what people would want a plugin for, and it settles the step, deadline and confirmation rules before the protocol exists.
3. **Plugin protocol and action providers.** Built: manifest, plugin socket, lifecycle, routing through the service, `Action::Plugin`, the editor form and Settings > Plugins ([writing a plugin](plugins.md)). How it differs from this proposal is listed under [as built](#plugin-protocol-as-built).
4. **Events, conditions and key images.** Events for a plugin's own controls, condition providers, plugin-supplied images. Events and conditions are built ([4a](#input-events-as-built), [4b](#plugin-conditions-as-built)); images are not.
5. **Community readiness.** Built: a versioned manifest schema, author docs and the trust UI ([as built](#trust-ui-as-built)).
6. **Distribution.** Install by dropping a folder in first; a registry only if there is demand. Installing, updating and removing from Settings is built ([6a](#installing-a-plugin-as-built)); no registry.

## The "run program" action

`run_program` runs one executable directly, without a shell or a Terminal window. Agreed rules:

- **Shape.** `{"type": "run_program", "program": "/usr/local/bin/tool", "args": ["--flag", "value"], "directory": "~/code"}`. `program` is an absolute path or starts with `~/`. `args` is optional, at most 32 items. `directory` is optional, with the `terminal_command` rules (`~`, `/…` or `~/…`); without it the program starts in the home folder. Every field is one line without control or invisible characters, so what runs is what was reviewed.
- **No shell.** The arguments are passed as they are. Nothing is expanded, quoted or split, and a leading `~/` is expanded only in `program` and `directory`.
- **Variables.** With `"variables": true`, `{{name}}` references in the arguments are filled in when the key is pressed. Built as proposed in open question 1 below.
- **Deadline.** 15 s as a key's own action. As a `multi_action` or `conditional` step it gets what is left of the sequence's 15 s deadline. At the deadline the program and every process in its process group are killed, and the press fails. Processes it leaves running after a normal exit are not touched, so a launcher still works. The group is also killed on cancellation, on the service's forced exit and on a panic. A service killed with SIGKILL cannot do that, so a program running at that moment keeps running.
- **Environment.** The program gets the service's own environment, with no fixed variables added. Started from the Finder, `PATH` holds only the system folders, so a `#!/usr/bin/env` script may not find a Homebrew interpreter; exit 127 or 126 adds a hint saying so. A missing start folder fails before anything starts.
- **Result.** A nonzero exit, or a program that cannot start, fails the press. The program's output is discarded and never logged or shown, as for `terminal_command`, because it can echo tokens passed in the arguments. The log records only lengths and the argument count.
- **Confirmation.** Each distinct program, arguments and directory is one capability item in the Execute confirmation, shown in full. Changing any of them in a running Live profile pauses the session for a new confirmation. An imported profile that holds one needs review, like `terminal_command`.
- **Packs and starters.** Neither may contain `run_program`.

## Non-goals

- Running Elgato Stream Deck plugins. Their manifests, events and property-inspector HTML are tied to Elgato's runtime; a compatibility layer would be a project of its own.
- A sandbox in the first versions.
- Windows or Linux plugin hosts.
- Plugins receiving all deck input.

## Open questions: proposals

Three questions were left open, each with a proposal below. All three are built as proposed, with the changes listed under [as built](#plugin-protocol-as-built).

### 1. Variable references in text parameters and run_program arguments

**Status:** built for `run_program` as described here (`variable_references` and `fill_in_run_program_args` in `profile.rs`, mirrored in `action-model.ts`). The Execute confirmation names the variables after the arguments, and the activity log adds `"variables": true`.

**Syntax: `{{name}}`.** `name` follows the existing variable name rules (1 to 32 characters of `a-z`, `0-9` and `_`, starting with a letter), with no spaces inside the braces. Examples: `--scene={{scene}}`, `{{count}}`.

- `$name` and `${name}` are rejected because they look like shell expansion. Since `run_program` runs no shell, people would expect `$HOME` or `${PATH}` to work, and they would silently do nothing.
- `{{…}}` has no meaning in a shell, in JSON or on a command line, and it is the most widely recognised template form.

**Substitution is opt-in per field, so existing text is never reinterpreted.**

- A plugin's text parameter is substituted only when its manifest entry sets `variables: true`.
- `run_program` gains an optional `"variables": true`, which defaults to off. With it on, the arguments are substituted. `program` and `directory` never are, so the confirmed program is always the one that runs.
- With substitution off, `{{` is plain text, as it is today.

**Rules when substitution is on:**

- **Checked when the profile loads.** Every `{{` must begin a well-formed reference. Each name must be a variable the profile declares. Otherwise the profile is refused with the field and the name in the error. This is how `set_variable` and `variable_is` already treat undeclared variables. A reference to an undeclared variable therefore never reaches a press.
- **Escaping.** There is none in the first version. A field that needs a literal `{{` leaves substitution off. One can be added later without breaking profiles.
- **Values.** A text value is inserted as it is. A number is written in decimal (`-3`, `42`), and a bool as `true` or `false`.
- **One argument stays one argument.** A value is never split, quoted or expanded, so a value with a space stays inside its argument.
- **The press-time check fails closed.** After substitution, each `run_program` argument must still pass the load-time rules: one line, no control or invisible characters, at most 512 bytes each and 4096 in total. If it doesn't, the press fails and nothing runs. Today variable text may hold invisible characters, so this check is what keeps "what runs is what was reviewed".
- **No leading `-` from a variable.** When a reference starts a `run_program` argument and its value begins with `-`, the press fails. Variables can be set from outside the deck (`osh`, plugins), and this stops such a writer from turning a value into an option for the program, such as `--delete`. Write `--name={{name}}` to pass a value that may start with `-`.
- **Confirmation.** The Execute confirmation shows each field as written, with its `{{name}}` references, and names the variables that feed it. It adds a line saying those values can be changed by processes outside the app. Values never appear there, because they change after confirming.
- **Logs.** `run_program` keeps logging only lengths and the argument count. For plugin parameters, the log records the substituted value, as already decided, except for parameters the manifest marks `secret`, which are logged by length.

### 2. Plugin socket limits and heartbeat

The numbers below start from the ingress socket's limits: 8 KiB lines, 8 connections, a 2 s line timeout and a 1 s forward timeout. They change where a long-lived connection needs something different.

| Limit | Proposal | Why |
| --- | --- | --- |
| Line size | 64 KiB each way; a longer line closes the connection | Results and logs can be longer than ingress requests, but are still bounded |
| Connections before `hello` | At most 8, and `hello` must arrive within 2 s | Same as ingress: a stray client cannot hold slots |
| Connections per plugin | One. A second `hello` with the same token is refused while the first is connected | The token is new at each start and given only to the process the app started |
| Wrong token | The answer is an error and the connection is closed, as for ingress | |
| Invokes in flight | At most 8 per plugin; more fail at once with "plugin busy" | Each holds a module's worker, which runs one action at a time |
| Invoke deadline | 15 s for a key's own action; as a step, what is left of the sequence's deadline (as for `run_program`) | Already decided |
| `set_variable` | At most 10 per second per plugin; more are answered with an error | Each write goes to every running module with a 1 s forward timeout |
| `log` | At most 1 KiB per line and 20 lines per second; the rest are dropped and counted in one "dropped N lines" record | Keeps the service log readable |
| Heartbeat | After 10 s without a line from the plugin, the app sends `ping` and expects `pong` within 5 s | An idle but healthy plugin costs almost nothing |
| A missed heartbeat | The app closes the socket, kills the plugin's process group, and restarts it with backoff | Same as a crash |
| Restart backoff | 1, 2, 4, 8, 16, then 30 s. After 5 failures within 5 minutes the plugin stays stopped until the user starts it again or the profile is executed again | Stops a crash loop |

On close, the app kills the plugin's process group, as `run_program` does, so helper processes a plugin started do not outlive it.

### 3. Starting a plugin without a profile that uses it

**Proposal: no, not in the first versions.** A plugin runs only while a running profile uses it.

- **Its writes would go nowhere.** Variables live in the running module processes and are declared by their profile, so with nothing running there is nothing to write to.
- **Consent.** The Execute confirmation is where a user agrees to what a profile does. A plugin that starts on its own would run as the user in the background, with no confirmation tied to it.
- **The menu bar has nothing to show.** Today it offers only Show, Updates and Quit, and would need its own status design first.

**Later.** Add an optional manifest field such as `"background": true` with its own toggle in Settings, off by default, and an enable dialog that says the plugin will run whenever the app is open. Not built: step 5 shipped without it.

## Step 4a: input events

Status: proposed 10 November 2026, built 12 November 2026 ([as built](#input-events-as-built)).

Today a plugin hears only the gesture its action is bound to, as an `invoke`. It cannot hold a key (push to talk), tell a knob's turn direction or read a fader's position. Step 4a lets it hear the rest of the input on the controls it owns, and only those.

### What a plugin hears

- **A control is the plugin's** while a binding on it runs one of the plugin's actions directly, as the key's own action, and the manifest marks that action `"events": true`. A plugin action inside a `multi_action` or an if / else does not count: that key does several things and belongs to no plugin.
- **Gestures with no binding of their own** on that control are sent to the plugin as `event`. A gesture bound to the plugin's action stays an `invoke`, as today. A gesture bound to anything else runs that action, and the plugin is not told. So a key with **Press** bound to "Talk" sends the release as an event, but a release bound to an OBS action stays the OBS action's.
- **Keypad pages and dial functions.** A key is the plugin's only while its page is shown, and a release goes to the tile that took the press even if the page changed meanwhile, as releases already resolve (`ProfileSession`'s held keys). The dial is the plugin's only while the function holding the binding is selected.
- **Live only.** A Test run never starts a plugin, so it sends no events and logs none.

### Messages

App to plugin, with no answer expected:

```json
{"type":"event","action":"talk","gesture":"released","control":{"module":"m4315","index":3},"params":{"channel":"team"}}
{"type":"event","action":"volume","gesture":"clockwise","control":{"module":"m003k","index":1},"params":{}}
```

- `gesture` uses the profile's names (`released`, `clockwise`, `counter_clockwise`, `right`, `left`, `long_press`, `up`, `down`...). A fader position adds `"position":{"raw":512,"maximum":1023}`.
- `params` are the bound action's parameters with variables filled in, as an invoke would have them, **without secrets**. They tell two keys bound to the same action apart, such as two lights.
- **`invoke` gains the same `input`**: `{"gesture":"pressed"}`, and the position for a fader. An action bound to both turn directions, or to a fader, can then tell what happened. Plugins must ignore fields they do not know. The author guide will say so; the Python example already does.

### Delivery

- The module service writes a `plugin_event` record when it handles the input, in input order. A release that comes while the press's invoke is waiting is handled after it, as today, so the plugin never hears a release before its press.
- The service does not wait. Nothing comes back, and the module's next input is not held up.
- The app forwards an event only when the plugin is enabled, running and has said hello, and the manifest action has `events`. Otherwise it drops it without logging.
- **At most 50 events a second per plugin.** More are dropped and counted, and the count goes in the plugin's log, as for its own log lines. Fast knob turns can exceed this, so the guide will tell authors to treat each event as one step and not rely on counting every one.
- Events do not go in the activity log, which would fill up on every turn.

### Manifest and consent

- An action may set `"events": true`. Schema stays 1: an older app refuses the unknown field, it does not misread it.
- The first-enable dialog says, for each such action, that the plugin hears the other input on keys and controls bound to it. A changed `plugin.json` needs a new approval anyway.
- The Execute confirmation does not change: it already names each plugin action, and the plugin can do nothing with an event that it could not do on an invoke.

### Not in 4a

- Telling a plugin when a key appears or disappears (a page switch, a profile stop). That belongs with key images, in 4c.
- Events for gestures bound to other actions, or for keys that run a plugin action only as a step.
- Coalescing knob turns into one event with a step count.

### Work

1. **Service:** decide which inputs produce a `plugin_event` and write it (flat bindings, keypad pages with held keys, dial functions), and add `input` to `plugin_invoke`. Unit tests for each routing case.
2. **App:** forward events behind the checks above, apply the rate limit, add `events` to the manifest parser, and add `input` to invoke.
3. **Frontend:** the dialog line, and the manifest type.
4. **Example:** a "Hold" action in `com.example.notify` that reports how long the key was held.
5. **Docs:** the author guide (manifest field, `event` message, `input`, ignoring unknown fields) and this section marked as built.

## Input events as built

Built on 12 November 2026 as proposed above, with these differences:

- **Who owns a control is decided when the input is planned**, before routing moves a held key or the dial function on (`ProfileSession::plugin_event_targets`), from the bindings that apply to that module's serial (an exact serial's binding hides a wildcard's for the same gesture, as in routing). A gesture is an event only when it has no binding of its own there. When several plugin actions are bound directly on one control, each hears it, listed once. The service does not read manifests, so it writes the event for every such action and the app drops those whose manifest action lacks `events`.
- **`input` carries the control as well**: `{"gesture":…,"control":{"module":…,"index":…}}`, and `position` for a fader, on `plugin_invoke` and `plugin_event` records and in the `invoke` message. The `event` message puts the same fields at its top level. The app rebuilds `input` from these known fields, so nothing else a record carries reaches the plugin.
- **The app checks an event** like an invoke, against the manifest current when it arrives: the run still executes with the plugin, `min_version`, the action has `events`, every parameter is declared and fits, only parameters that allow it take variables, and the profile holds no secret's value. Required parameters may be missing, since secrets are never added.
- **Live coalescing.** When the module service falls behind, it merges unbound knob, roller and dial steps on one control into the latest. Steps a plugin hears are not merged, so it hears each, until the pending backlog is half full (128 events): then they merge like the rest, so one long turn does not push out what comes after it, such as the plugin's own release. Steps that alternate (two knobs turned together, or both directions) do not merge, as for any knob, and can still fill the backlog. The service cannot see `events`, so this also applies to a knob whose plugin action does not take events; the app then drops those events.
- **Faders.** A fader takes only fader actions, so a plugin never owns one. `input` can carry `position`, but nothing sends it yet.
- **A release with no press.** The service records a held key when it plans the press. If the live backlog then drops that press, the release still goes to the plugin, with no invoke before it. The author guide says so.
- **Dial functions.** With `dial_functions`, the dial's raw `pressed` and `released` are never events: they are the parts of the click that becomes `short_press` (a function switch) or `long_press`.
- **Nothing waits for the plugin.** Every line to a plugin (welcome, invokes, events, pings, replies) goes into its connection's outbox and is written whole and in order by the connection's writer thread. A slow reader therefore never splits a line, holds up the module run, or reorders an event and an invoke sent after it. A write that blocks is not a failure: the writer retries it for as long as the connection stands, and a plugin that reads nothing is found by its pings, as before. While lines wait, at most 64 events are queued; more are dropped and counted. Past 1 MiB waiting, events are dropped and any other line ends the connection. An invoke whose deadline passes while it waits is not written (the module has given up on it), and one that is written carries in `timeout_ms` what is left of its time.
- **A dropped count.** Events dropped by the 50 a second limit or a full outbox are added to the plugin's log, as one line, with the next event queued. The window and count start again with each plugin process.
- **Known gap.** The app counts an invoke's deadline from when its record arrived, a moment after the service started counting. A press that waits for a Keychain read and times out in the service just before the app's deadline can still be sent after the release event the service then writes.
- **Not built:** the frontend shows nothing new beyond the enable dialog's line; the editor does not mark keys whose other input a plugin hears.

## Step 4b: plugin conditions

Status: proposed 13 November 2026, built 13 November 2026 ([as built](#plugin-conditions-as-built)).

Today an if / else tests only what the service reads itself: OBS outputs, the default output's mute and volume, timers, the frontmost app and declared variables (`Condition` in `profile.rs`). A plugin can already feed a test through a variable it sets, but only by pushing state ahead of time, and the profile must declare that variable. Step 4b lets a plugin offer checks of its own ("the desk light is on", "a meeting is running"), which are asked when the key is pressed, as OBS conditions are.

### What the user sees

- In the if / else editor, each enabled plugin's conditions are listed with the built-in tests, under the plugin's name. Picking one shows its parameters with the same form plugin actions use, and a **Holds** / **Does not hold** choice.
- A condition is a test like any other: it goes in AND and OR groups, nests, and counts toward the existing limits (8 per group, 3 levels deep, 16 tests in all).
- A missing plugin, an older version or an unknown condition still loads, as for a plugin action, and is shown in the editor as such; pressing that key fails with the reason.

### Manifest

```json
"conditions": [
  { "id": "is_on", "label": "Light is on", "params": [
    { "id": "entity", "label": "Light", "type": "text", "variables": true, "required": true },
    { "id": "token", "label": "Access token", "type": "text", "secret": true }
  ] }
]
```

- Optional, up to 64, ids unique among conditions (they may repeat an action's id). The id, label and `params` follow the action rules exactly, `secret` and `variables` included. There is no `events`.
- A manifest may now have no `actions` when it has `conditions`; it must have at least one of the two.
- Schema stays 1: an older app refuses the unknown field, it does not misread it.
- The first-enable dialog lists the conditions with their parameters, under the actions, and says the plugin is asked each time an if / else using one runs.

### Profile format

```json
{"type":"conditional","when":{"plugin":{"plugin":"com.example.lights","min_version":"1.2.0","condition":"is_on",
  "params":{"entity":"light.{{room}}"},"variables":["entity"],"value":true}},"then":[…],"else":[…]}
```

- A new leaf `Condition::Plugin`, written as the single key `plugin`, with the fields of a plugin action plus `value`: the answer it holds for, `true` or `false`, as `obs_streaming` takes one. That gives "does not hold" inside an AND or OR group, where swapping the branches cannot.
- Parameters, `variables`, `min_version` and the text limits are checked as for a plugin action, by the same code. A `secret` value in the profile is refused as there.
- Key faces cannot use it: they take only variable-only conditions (`is_variable_only`), and asking a plugin at every redraw would be wrong. A face can follow a variable the plugin sets.

### Messages

App to plugin:

```json
{"type":"check","request":9,"condition":"is_on","params":{"entity":"light.desk"},"timeout_ms":3000,"input":{"gesture":"pressed","control":{"module":"m4315","index":3}}}
```

Plugin to app, the existing `result` with a `value`:

```json
{"type":"result","request":9,"ok":true,"value":true}
{"type":"result","request":9,"ok":false,"error":"the light is offline"}
```

- `check` is its own message type, not an `invoke` with a flag, so a plugin that has no conditions never mistakes a check for a press, and each message's fields mean one thing. Request numbers are shared with invokes; the answer is matched by number as today.
- An `ok: true` answer without a boolean `value` fails the press ("answered a check without true or false"). `value` on an invoke's result is ignored.
- Parameters arrive as an invoke's do: variables filled in, secrets added from the Keychain, `timeout_ms` what is left when the line is written. `input` is the press that ran the if / else.
- A check should only read. Nothing can enforce that, so the author guide says so: it may be asked at every press, in Live runs only, and should answer quickly.

### When it is asked

- **Live only.** A Test run records an if / else without reading its condition, so it never asks a plugin, as now.
- When the if / else runs, in the order its tests are read today: every member of a group is read, without stopping early. A check gets what is left of the sequence's 15 s deadline, and at most **3 s**, the OBS client's limit.
- The service writes a `plugin_check` record and waits, as for `plugin_invoke`; the app answers on the control channel with `plugin_result`, which gains `value`. The bridge's answer becomes a value, so `invoke` and `check` share one waiting path.
- On the app side a check goes through the invoke path: the manifest check (against `conditions`, not `actions`), Keychain secrets on the plugin's invoke thread, the 8-waiting limit, the outbox, the deadline drop. A module's checks and presses stay in one order.
- **Any failure fails the press, and neither branch runs**: not enabled, not running or starting, a required secret missing, too many waiting, no answer in time, `ok: false`, or no `value`. The message names the plugin and the condition (`conditional could not read com.example.lights is_on: the light is offline; neither branch ran`). Taking the else branch when a plugin is down would make a key do the other thing silently. The editor already tells users that a test that cannot be read fails the press.
- The activity log records it like a plugin action: ids, numbers and bools shown, text parameters by length.

### Where a plugin use is counted

Everything that walks a profile's plugin actions must see plugin conditions too. They walk one list of plugin uses (action or condition: plugin, version, id, params, variables), not `Action::Plugin` alone:

- **Starting plugins.** `plugins_used` includes a plugin that is only ever asked a check, so a Live run starts it.
- **Execute confirmation.** Each plugin condition is named with its parameters, like an action, and makes the profile high risk. A Live run whose saved profile adds or changes one pauses for a new confirmation.
- **Imports, packs and starters.** A JSON import holding a plugin condition runs as a dry run until allowed; preset packs and starters cannot hold one.
- **Secrets.** Export drops secret values from plugin conditions too, and the editor offers **Move to the Keychain** for them.
- **Variables.** A reference in a condition's parameters must name a declared variable, and counts as a use of it.

### Not in 4b

- Conditions on key faces, and any caching or subscription of a plugin's state. A plugin that wants to drive a face sets a variable.
- Conditions that take a non-boolean answer (a number to compare). A plugin can expose "level at least" as a condition with a number parameter.
- Telling a plugin which keys use its conditions.

### Work

1. **Service:** `Condition::Plugin` (parse, validate with the plugin action rules, variables), `PluginBridge::check` returning a bool on a shared waiting path, `plugin_check` records, `value` on `plugin_result`, evaluation under the deadline and the 3 s cap, activity logging. Unit tests for the answers, failures and deadline.
2. **App:** `conditions` in the manifest parser and checks, `check` messages through the invoke path, `value` in results, and the walk over plugin uses for starting, confirmation, imports, secrets and export.
3. **Frontend:** condition types, the plugin conditions in the if / else picker with the parameter form and **Holds** / **Does not hold**, the enable dialog's list, and the confirmation lines.
4. **Example:** a "Pressed at least" condition in `com.example.notify`, true when its counter has reached a number.
5. **Docs:** the author guide (manifest, `check`, `value`, read only) and this section marked as built.

## Plugin conditions as built

Built on 13 November 2026 as proposed above, with these differences:

- **Both sides refuse a check answered without a value.** The app's host already knows a request is a check, so a plugin's `ok: true` without a boolean `value` fails there with the service's text ("the plugin answered a check without true or false"), and the service fails the same way if a `plugin_result` for a check comes without one. The app relays `value` only for a check; on an invoke's result it is dropped.
- **One secret per id across actions and conditions.** A secret parameter of a condition with the same id as an action's secret shares its Keychain item, as two actions' do. `list_plugins` lists such a secret once, as the first action (or, after the actions, condition) declaring it names it, and `set_plugin_secret` / `forget_plugin_secret` take a secret only conditions declare.
- **`list_plugins`.** A manifest's `conditions` is left out when there are none; `actions` is always there, `[]` for a plugin that offers only conditions.
- **Re-confirmation also follows `min_version` and `value`.** A plugin condition is a confirmation item of its own (plugin, condition, parameters, variables, without `min_version`, as for an action), but the if / else around it is one too, and that item is the whole test. So, unlike a plugin action's, a changed `min_version` of a condition still pauses a Live run for a new confirmation, as does a changed **Holds** / **Does not hold**. The confirmation's line for a condition names both (`com.example.lights: check is_on (holds; needs 1.2.0 or later) with …`), so such a pause shows the change as a new line. Variables filled into a condition's parameters feed it as they feed an action's.
- **Keychain reads count against a check's time.** The first check needing a secret not read yet waits for the Keychain within its 3 seconds, so a Keychain prompt can fail it; once read, the secret is held. Such a check waits on the plugin's invoke thread, so it keeps its order within its module, but another module's check or press whose secrets are held can overtake it.
- **Diagnostics.** A plugin condition's text parameters are replaced with their length in a diagnostics bundle's profile, as a plugin action's are.
- **The example's `count` is required**, so a check without it fails at once instead of in the plugin.

## Step 6a: installing a plugin

Status: built 13 November 2026 ([as built](#installing-a-plugin-as-built)).

Today a plugin is installed by hand: open the plugins folder from Settings > Plugins, copy the plugin's folder in, rename it to the plugin's id if needed, and fix its permissions if it is writable by others. Updating means deleting the old folder first, and a plugin that is running at the time keeps running from files that are being replaced. Removing it leaves its approval and Keychain secrets behind. Step 6a makes install, update and remove one action each in Settings > Plugins, with the review the first-enable dialog already gives.

### Install

- **Install…** in Settings > Plugins opens the macOS folder picker (`choose folder`, through `osascript`, as the app picker does). The user picks the plugin's folder wherever it is, such as Downloads. Its name does not matter: a download is often `com.example.notify-1.2.0` or `notify-main`.
- **A `.zip` is not accepted in 6a.** Safari unzips downloads by default, and Finder unzips one with a double click; refusing archives keeps the app out of archive formats (paths outside the folder, links, size bombs). The picker says to unzip first.
- The app reads the chosen folder's `plugin.json` by the usual rules (schema, size, strict fields), except the folder-name rule. A folder with no usable manifest is refused with the reason, before anything is copied.
- **One review, then the copy.** The dialog is the first-enable dialog's review (name, id, version, program, actions, conditions, permissions, secrets, "a plugin runs as you"), plus what installing does:
  - where it comes from, how many files and how large;
  - when the id is installed already: the version it replaces (and a warning when the new one is older), and that its approval ends because `plugin.json` changed (unless the bytes are identical);
  - **Install** copies it and leaves it not enabled; **Install and enable** also enables exactly the `plugin.json` shown (the same fingerprint rule as **Enable…**, checked again against the copy).
- If the chosen folder's `plugin.json` changes while the dialog is open, nothing is installed and the dialog says to review it again, as for enabling.

### The copy

- **Into a hidden staging folder** in the plugins folder (`.install-<random>`, which the catalog skips because it starts with a dot), then renamed into place as `<id>`. A failure at any point removes the staging folder, so the plugins folder never holds half a plugin.
- **Only regular files and folders.** A symbolic link, a device or another special file anywhere in the tree refuses the install and names it. Hidden files are copied (a plugin may need them); `.git` folders are skipped.
- **Limits:** at most 5,000 files and 256 MiB, so a picked home folder is refused, not copied.
- **Permissions:** everything belongs to the user (it is a new copy) and nothing is writable by others (`go-w`, keeping execute bits), so the folder passes the owner check that `load` makes.
- **Quarantine is kept.** Files downloaded from the web carry macOS's quarantine flag, and the copy keeps it; the app does not clear it. An unsigned program in a plugin may then be stopped by Gatekeeper when the plugin starts, and its error shows in Settings > Plugins. The author guide says to sign native programs, or to use an interpreter such as `/usr/bin/python3` with a script.
- **The copy is checked again** with `load` (now including the folder name) before it is renamed into place.
- The source folder is never changed or deleted.

### Update

- Installing an id that is installed already replaces it. The host first stops that plugin (pending presses fail, as on disable), the old folder is renamed aside (`.old-<id>-<random>`), the new one renamed into place, and the old one deleted. If the rename fails, the old folder is put back.
- **The approval follows the bytes.** A `plugin.json` that changed needs enabling again (or **Install and enable**); an identical one stays enabled, and a Live run that uses it starts the new copy.
- **Plugin data.** The folder is replaced whole, so a plugin must not keep its own state there. 6a gives every plugin `OSH_PLUGIN_DATA`: `<app data>/plugin-data/<id>/`, made 0700 on start, kept across updates and deleted on remove. The author guide says to keep state there.
- Installing the folder that is installed already (the source is inside the plugins folder) is refused.

### Remove

- **Remove…** on a plugin, with a confirmation naming it and saying that profiles using it keep their keys, which then fail with "not installed".
- It stops the plugin, removes its approval, forgets its Keychain secrets, deletes its folder and its `plugin-data` folder. A folder that cannot be used (a broken manifest) can be removed too.
- Profiles are not changed.

### Not in 6a

- `.zip` archives, drag and drop onto Settings > Plugins, and a registry or update checks (step 6b, if there is demand).
- Signature checks of a plugin's author. The review and the fingerprint approval stay the trust model.

### Work

1. **App:** `choose_plugin_folder` (osascript picker), `review_plugin_install(path)` (manifest, counts, size, the installed version, the fingerprint), `install_plugin(path, fingerprint, enable)` (staging copy with the checks above, stop, swap, approval), `remove_plugin(id)` (stop, approval, secrets, folders), `OSH_PLUGIN_DATA` in the host. Tests for each refusal (link, special file, limits, manifest changed, source inside the plugins folder), the older-version warning, update keeping or ending the approval, a failed swap restoring the old folder, and remove.
2. **Frontend:** **Install…** and the install review (reusing the enable dialog's sections), **Remove…** with its confirmation.
3. **Docs:** the author guide's "Installing" (the button, unzip first, quarantine and signing, `OSH_PLUGIN_DATA`) and this section marked as built.

## Installing a plugin as built

Built on 13 November 2026 as proposed above, with these differences:

- **Staging is `.install-<random>/<id>`**, a folder named by the id inside the hidden staging folder, so the copy is checked with `load` folder name included before the rename into place; the rename moves `<id>` out of it. Installs and removes run one at a time. At app start and before each install or remove, leftovers of an interrupted one are tidied: an installed copy moved aside by an update (`.old-<id>-<random>`) is renamed back to `<id>` when `<id>` is missing (the newest by change time, when there are several), and deleted when `<id>` exists; `.install-*` and `.removed-*` (a remove renames the folder to `.removed-<name>-<random>` before deleting it, so an interrupted remove is never restored) are deleted. Only names ending in the 12 hex digits these steps make are touched, so a backup named `.old-<id>-mine` by hand is left alone. An interrupted remove leaves the plugin's data folder and Keychain secrets (only its folder is tidied); installing it again and removing it clears them. Nothing is tidied in a plugins folder that is a symbolic link.
- **Only a real folder at `plugins/<id>` is replaced.** A symbolic link or a file there (the socket, say) refuses the review and the install, and the id `plugins.sock`, the socket's name, is refused by the manifest check itself, so the catalog, enable and install all refuse it.
- **The limit is 5,000 files and folders together** (`.git` not counted), so a tree of empty folders is bounded too; the review's count is of files only.
- **Copied permissions are `mode & 0755`**: execute bits are kept, set-id and sticky bits are dropped, and folders always keep `u+rwx` so the copy can be deleted. File data is read and written (each file only up to what is left of 256 MiB plus one byte, so a file that grows while it is copied refuses the copy instead of passing the limit) and extended attributes are copied (`fcopyfile` with `COPYFILE_XATTR`), so the quarantine flag stays; ACLs are not copied, so none can let others write. While copying, every folder and file is opened from its parent folder's descriptor (`openat` with `O_NOFOLLOW`, folders with `O_DIRECTORY`), so a folder or file swapped for a symbolic link after the scan is refused (naming the link), never followed, and a folder swapped for a file says it "was replaced since it was read". The picked folder itself is opened with `O_NOFOLLOW` and must be the folder (device and inode) that was scanned, or the copy is refused ("review it again"). **A hard link is refused**, naming it ("bin/x is a hard link; a plugin may hold only its own files"): a regular file with more than one name (checked at the scan and again on the open file at copy time) may be a name for a file the user does not own. A folder made with tools that hard-link files (pnpm's `node_modules`) is refused for that reason.
- **The picked folder's `plugin.json` is read without the owner check** as well as the folder-name rule (a download may be writable by others; the copy is not). A folder that holds the plugins folder is refused too, not only one inside it.
- **A plain Install keeps the plugin enabled only when the copy installed now has identical `plugin.json` bytes and is enabled.** In every other case it removes the plugin's approval before the swap (if that fails, nothing is installed; if the swap then fails, the approval is put back), rather than leaving it to show as `changed`: an approval of other bytes, and also one of these very bytes left behind by a folder deleted by hand or held while the installed copy shows `changed`, so nothing approved before the review enables what is installed. The review's `plugin.approval` is what a plain Install leaves (`enabled` or `not_enabled`); **Install and enable** always enables. If enabling fails after the copy is in place, the error starts with exactly `installed: ` and reads on its own after it ("installed: Lights could not be enabled: …"; "installed: the earlier approval of Lights could not be removed: …" only when an enable lands during the install), so the dialog can say the plugin was installed; every error raised before or at the swap is plain text and means nothing was installed. Deleting the old copy after the swap and resuming the host report no error (a copy that could not be deleted is logged and deleted as a leftover later).
- **The host pauses the plugin** for the swap and the approval change: it is stopped (pending presses fail with "it is being installed or removed"), and the install or remove waits until none of its processes is left, helpers included (SIGTERM, SIGKILL after 1 s, then up to 3 s for the process group to be gone; a group that is still there, or holds a process that cannot be signalled, keeps blocking an install or remove for another 60 s), so the old copy never runs while its folder or data folder changes. If they are not gone in time, nothing is changed, the error says to try again, and the plugin is not started again at once (a process of the old copy may still run): it starts at the next ordinary start (a run that starts using it, an enable, a due restart). Nothing starts the plugin until the install or remove is done; then a Live run that uses it starts what is installed, as if it had just started using it.
- **A folder is used only under its exact name.** On a case-insensitive volume `plugins/<id>` also opens a folder named in other case; `load` checks the parent holds an entry of exactly the id, so such a folder is unusable ("rename the folder to its id") for the host too. Install replaces it (it is moved aside like any installed copy), and Remove by the listed name deletes it.
- **Remove forgets the secrets the folder's `plugin.json` declares**, read without the owner and name checks, so an unusable folder with a parseable manifest still has them forgotten. When `plugin.json` cannot be parsed at all, its Keychain items are left (the Keychain cannot be listed by account prefix through the same vault); they are inert, and installing the plugin again and removing it clears them. Remove of an id with nothing installed succeeds. Its approval is removed before its folder is moved aside, and put back if the move fails. A failure after the plugin's folder is gone (its data folder could not be deleted, the Keychain kept a secret) does not stop the other steps and is reported once at the end, starting with exactly `removed: ` and reading on its own after it ("removed: the Keychain kept its secret token (…)"); every error before is plain text and means nothing was removed.
- **Remove takes the listed folder name**, so a folder not named by a plugin id (an unusable one such as `notify-main`) can be removed too: the name must be one visible entry directly in the plugins folder (not empty, no `/`, not `.` or `..`, not starting with `.`, so staging and set-aside folders cannot be named) and a real folder, not a symbolic link; only that folder is deleted, since no approval, secret or data folder can belong to a name that is not an id. A symbolic link, under any name, is refused and never followed. Remove changes nothing when the plugins folder or `plugin-data` is a symbolic link; a link as `plugin-data/<id>` is removed itself, never followed.
- **A refused install closes the dialog** and the reason shows in Settings > Plugins (the dialog does not stay to say to review it again), as a refused enable does: what the dialog showed may be stale. While an install runs the dialog cannot be closed, since closing it would not stop the install.
- **`OSH_PLUGIN_DATA` is the folder's path without a trailing slash.** `<app data>/plugin-data` and the plugin's folder in it are made 0700 before each start; when that fails, the start counts as a failure.

## Plugin protocol as built

Built on 6 November 2026 as proposed above, with these differences:

- **Plugins run only for Live runs.** A Test run never starts a plugin, so no plugin code runs before the Execute confirmation. Switching a run between Test and Live in place starts or stops its plugins.
- **A 5 s linger.** A plugin stops 5 s after the last Live run that uses it, so a module run that restarts keeps the same plugin process ("a Test / Live restart of a module process does not restart its plugins").
- **Variables in the profile.** The service does not read manifests, so a plugin action lists the text parameters it fills in (`"variables": ["entity"]`), and the app checks at each invoke that the manifest allows variables for each. The editor keeps the list in step with the text.
- **Text parameters are logged by length.** The service cannot tell which parameters the manifest marks `secret`, so its activity records give every text parameter's length only, never its value. Numbers, bools and the ids are logged.
- **`set_variable` is limited to `permissions.variables`.** The other permissions are shown only.
- **Built later, in step 5:** the first-enable dialog and Keychain storage for secrets ([trust UI as built](#trust-ui-as-built)).
- **The plugins folder is the socket's folder**, made 0700, and the folder and `plugin.json` must belong to the user and not be writable by others. A folder's name must equal its manifest's id.

## Trust UI as built

Built on 9 November 2026 (step 5):

- **Manifest schema.** `plugin.json` has a required `"schema": 1`, and this app reads schema 1 only. The schema is read from the untyped JSON before the strict parse, so `"schema": 2` with new fields says "plugin.json is schema 2; this OpenStreamHub reads schema 1, update the app", not "unknown field". A missing, zero or non-integer schema is refused with a fix-it message. Schema 1 is stable: a change that would make an older app misread a manifest gets a new schema number.
- **Approval by fingerprint.** A plugin is enabled for the SHA-256 (hex) of the exact `plugin.json` bytes read. Any change to the file needs a new approval. Approvals live in `<app data>/plugin-approvals.json` (`{ "<id>": "<fingerprint>" }`, written atomically, 0600), outside the plugins folder so a plugin cannot approve itself. A missing or damaged file approves nothing; a damaged one is logged and replaced at the next change.
- **The host refuses to start an unapproved manifest.** At each start it compares the fingerprint of the bytes it just read with the approved one. A plugin that is not enabled, or whose manifest changed, is `not_enabled`: not a failure, so no backoff, no failure count and no restart; its presses fail at once with the reason. Enabling it while a Live run uses it starts it at once; disabling it stops it (pending presses fail, its process group is killed) and it is `not_enabled` again.
- **Commands.** `enable_plugin(id, fingerprint)` reads the manifest again and refuses when its fingerprint is not the one the dialog showed ("plugin.json changed since you reviewed it; review it again"), so the user approves exactly what they saw. `disable_plugin(id)` removes the approval. Both tell the host while they hold the approvals lock, so a quick enable and disable reach it in the order the file records them. A refused enable closes the dialog with the reason. A damaged approvals file is renamed to `plugin-approvals.json.damaged` at the next change. `list_plugins` gives each plugin its `fingerprint` and `approval`: `enabled`, `changed` or `not_enabled`.
- **The first-enable dialog** (Settings > Plugins, **Enable…** or **Review…**) lists name, id, version, folder, program and arguments, each action with its parameters (type, secret, variables, required), each permission, the secrets it uses and which of them are already stored (and will be sent), and says first that a plugin runs as you and can do anything you can, and that the permissions are what it declares, not limits, except the variables it may set.
- **Secrets in the Keychain.** A `secret` text parameter is not stored in profiles. Its value is one Keychain item per plugin and parameter id (account `plugin-secret:<plugin id>:<param id>`), shared by every key and action of that plugin with a secret of that id. The host, not the module service, fills it into the invoke before the manifest check, on an invoke thread of that plugin's own when a Keychain read is needed (one per plugin, made on first need, so plugins' reads run side by side and one plugin's presses stay in order; presses whose secrets are held in memory, or that have none, are sent at once), so a Keychain prompt holds up neither a module's record relay nor other presses. A press's deadline counts from when it arrived, and one still waiting when it passes fails and is never sent; so does one whose own run no longer executes with the plugin. The completed parameters are checked again with the SDK's text limits, and a line longer than 64 KiB is refused, before anything is sent. It reads the Keychain once and then holds the value in memory (replaced on set or forget); a failed read is held for a minute, so a denied Keychain is not asked at every press. A required secret that is not set fails the press at once; an invoke whose profile carries a value for a secret is refused, checked again against the manifest current when the invoke is sent. Known secret values (those sent to the plugin this session, kept per plugin up to 16 even after a change or forget, and those held now; at least 4 bytes) are replaced with `[secret]` in the plugin's log, stderr and error texts, in one pass; `set_variable` values are not checked. `set_plugin_secret(plugin, param, value)` (the plugin installed, the parameter a secret of some action, the value within the text limits) and `forget_plugin_secret(plugin, param)` change them; `list_plugins` says per secret only whether it is set, asking the Keychain whether the item exists without reading it (`set: null` when it cannot tell). An export drops secret values a profile still holds, for plugins installed at export time, and names them.
- **Editor.** A secret parameter has no input, only whether it is set and a pointer to Settings > Plugins, and a required one is never reported missing. A profile that still holds a secret's value is offered **Move to the Keychain** (or removal), after which the user saves.
- **Test mode is unchanged:** a Test run never starts a plugin.
