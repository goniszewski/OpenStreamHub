# Plugins: research and proposed design

Status (6 November 2026): **proposal, decisions agreed, nothing built.** Plugins are not in the official MasterHUB 1.0.0.113 either: there "plugin" means built-in integrations (OBS, Twitch, Discord, Streamlabs), and a plugin SDK is absent ([app features](research/masterhub-1.0.0.113-app-features.md)). So plugins would be our own extension, not a parity item, and need no change to the device protocol.

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

**Audience.** The maintainer writes the first plugins; community plugins come later. The first versions therefore keep the manifest minimal and may change it. A stable, versioned manifest schema, author docs and the full trust UI come before any community distribution (see the suggested order).

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

1. **Data-only packs.** Built: the app preset packs. Still open: loading packs dropped into a folder in the app data folder, checked by the same rules.
2. **A "run program" action** (see below). A small change that covers much of what people would want a plugin for, and it settles the step, deadline and confirmation rules before the protocol exists.
3. **Plugin protocol and action providers.** Manifest, plugin socket, lifecycle, routing through the service, `Action::Plugin`.
4. **Events, conditions and key images.** Events for a plugin's own controls, condition providers, plugin-supplied images.
5. **Community readiness.** A stable, versioned manifest schema, author docs and the complete trust UI.
6. **Distribution.** Install by dropping a folder in first; a registry only if there is demand.

## The "run program" action

`run_program` runs one executable directly, without a shell or a Terminal window. Agreed rules:

- **Shape.** `{"type": "run_program", "program": "/usr/local/bin/tool", "args": ["--flag", "value"], "directory": "~/code"}`. `program` is an absolute path or starts with `~/`. `args` is optional, at most 32 items. `directory` is optional, with the `terminal_command` rules (`~`, `/…` or `~/…`); without it the program starts in the home folder. Every field is one line without control or invisible characters, so what runs is what was reviewed.
- **No shell.** The arguments are passed as they are. Nothing is expanded, quoted or split, and a leading `~/` is expanded only in `program` and `directory`.
- **Fixed arguments first.** Variable references in arguments come later, with the syntax chosen for plugin parameters.
- **Deadline.** 15 s as a key's own action. As a `multi_action` or `conditional` step it gets what is left of the sequence's 15 s deadline. At the deadline the program and every process in its process group are killed, and the press fails. Processes it leaves running after a normal exit are not touched, so a launcher still works.
- **Result.** A nonzero exit, or a program that cannot start, fails the press. The program's output is discarded and never logged or shown, as for `terminal_command`, because it can echo tokens passed in the arguments. The log records only lengths and the argument count.
- **Confirmation.** Each distinct program, arguments and directory is one capability item in the Execute confirmation, shown in full. Changing any of them in a running Live profile pauses the session for a new confirmation. An imported profile that holds one needs review, like `terminal_command`.
- **Packs and starters.** Neither may contain `run_program`.

## Non-goals

- Running Elgato Stream Deck plugins. Their manifests, events and property-inspector HTML are tied to Elgato's runtime; a compatibility layer would be a project of its own.
- A sandbox in the first versions.
- Windows or Linux plugin hosts.
- Plugins receiving all deck input.

## Open questions

- The variable reference syntax for text parameters and "run program" arguments, and how a reference to an undeclared variable behaves.
- The exact plugin socket limits and heartbeat interval.
- Whether a plugin may also be started without a profile using it (a state provider for the menu bar, for example).
