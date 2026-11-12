# Plugins: research and proposed design

Status (9 November 2026): **steps 1 to 3 and 5 built**: preset packs, the `run_program` action, the plugin protocol with action providers, and community readiness (manifest schema 1, the first-enable dialog, Keychain secrets; author guide: [writing a plugin](plugins.md)). Steps 4 and 6 are not started. Plugins are not in the official MasterHUB 1.0.0.113 either: there "plugin" means built-in integrations (OBS, Twitch, Discord, Streamlabs), and a plugin SDK is absent ([app features](research/masterhub-1.0.0.113-app-features.md)). So plugins would be our own extension, not a parity item, and need no change to the device protocol.

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
4. **Events, conditions and key images.** Events for a plugin's own controls, condition providers, plugin-supplied images.
5. **Community readiness.** Built: a versioned manifest schema, author docs and the trust UI ([as built](#trust-ui-as-built)).
6. **Distribution.** Install by dropping a folder in first; a registry only if there is demand.

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
