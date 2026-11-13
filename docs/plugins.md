# Writing a plugin

Status (12 November 2026): **manifest schema 1**, with the first-enable dialog and Keychain secrets ([design](plugins-design.md), step 5) and input events (step 4a). Schema 1 is stable: a change that would make an older app misread a manifest gets a new schema number. Built on macOS only.

A plugin adds actions a key can run, such as switching smart lights. It is a separate program that the OpenStreamHub app starts and talks to over a local socket. It can be written in any language. A working example in Python, using only the standard library, is in [`examples/plugins/com.example.notify`](../examples/plugins/com.example.notify).

**A plugin runs as you.** It can do anything you could do. The permissions a manifest lists are shown to the user but nothing enforces them, apart from the variables it may set.

## Enabling

A plugin does not run until you enable it. In Settings > Plugins a plugin shows **Enable…**, which opens a dialog listing what its manifest declares: name, id, version, folder, the program and arguments it starts, its actions with their parameters, its permissions and the secrets it uses, naming those already stored, which it will be sent. **Enable** approves exactly that `plugin.json`: the app keeps its SHA-256 in `plugin-approvals.json` in the app data folder (outside the plugins folder). If the file changed while the dialog was open, nothing is enabled, the dialog closes with the reason, and you review it again. A damaged approvals file enables nothing; it is kept as `plugin-approvals.json.damaged` at the next change.

Any change to `plugin.json`, even a space, needs a new approval: the plugin shows that its manifest changed and **Review…**, and it does not start until you enable it again. An update to the plugin's program alone does not. **Disable** removes the approval and stops the plugin at once.

A plugin that is not enabled is not a failure: it is not restarted, and a press on it fails at once with "not enabled: enable it in Settings > Plugins". Enabling it while a Live profile uses it starts it at once.

## Secrets

A text parameter with `"secret": true`, such as an access token, is never saved in a profile. Its value is kept in the macOS Keychain, one item per plugin id and parameter id (account `plugin-secret:<plugin id>:<param id>`), shared by every key and action of that plugin with a secret of that id. Set, change or forget it in Settings > Plugins; the value is never shown again. Settings and the editor show only whether it is set, which the app asks the Keychain without reading the value (or that the Keychain could not tell).

An export leaves out every value of a parameter that the installed manifest marks secret, and says which it left out. A plugin that is not installed when you export cannot be checked, so a value saved for one of its parameters stays in the file.

When a key is pressed, the app reads the value (the first time only; a read that fails is reported for a minute before the Keychain is asked again) and adds it to the parameters sent to the plugin. A required secret that is not set fails the press at once. Where the plugin's log, stderr or error texts contain a secret value the app has sent it this session (also one changed or forgotten since, up to 16 per plugin) or holds for it now, it is kept as `[secret]`, so a plugin that echoes one does not leak it into Settings or the activity log. This is a safety net, not a guarantee; the plugin chooses what it prints:

- Only values the app has sent the plugin or read in this session are known, so a secret no press has used yet is not taken out.
- Values shorter than 4 bytes are not taken out, since they would match ordinary text.
- A stderr line longer than 64 KiB is cut before it is checked, so a value split by the cut can remain.
- Values a plugin writes with `set_variable` are not checked.

A press whose secret has not been read yet waits for that read on a thread the app keeps for that plugin, without holding up presses that need no read or other plugins' presses; that plugin's presses stay in order. If its time (15 seconds, or what is left of a sequence's) runs out while it waits, for example while macOS asks whether the app may use the Keychain item, it fails and is never sent, so allowing access later does not run presses that already failed. The same holds when the module run it came from stops executing (or switches to Test) meanwhile, even if another Live run uses the plugin. With its secrets filled in, a press must still fit the text limits (2048 bytes each, 8192 for all text), or it fails without being sent. A profile saved before this change may still hold a secret's value: its presses are refused until you open the key, choose **Move to the Keychain** (or remove the value) and save. When the Keychain already holds a value, removing it from the profile is offered first, and replacing the stored value needs a second confirmation.

## Installing

Put the plugin's folder in the plugins folder, `~/Library/Application Support/<app id>/plugins/`; Settings > Plugins has a button that opens it. The folder's name must be the plugin's id, such as `com.example.notify`. The folder and `plugin.json` must belong to you and must not be writable by other users (`chmod go-w`). Settings > Plugins lists every folder with its manifest, or the reason it cannot be used.

## When it runs

- The app starts a plugin only once it is [enabled](#enabling), and only while an **executing (Live)** module run uses it, and keeps one instance for all modules. A Test run never starts a plugin; a press there only logs what would run.
- It stops the plugin 5 seconds after the last Live run that uses it stops, so a module that restarts (a large profile change, a replug) keeps the same plugin process. On quit, every plugin stops.
- A plugin that exits, closes its connection, misses a ping or has not said hello 10 seconds after it started is restarted after 1, 2, 4, 8, 16, then 30 seconds. After 5 failures within 5 minutes it stays stopped until a profile that uses it is executed again.
- Stopping sends SIGTERM to the plugin's process group, then SIGKILL after 1 second, so helpers it started stop with it.
- The plugin starts in its folder with the app's environment plus `OSH_PLUGIN_SOCKET`, `OSH_PLUGIN_ID` and `OSH_PLUGIN_TOKEN`. Started from the Finder, `PATH` holds only the system folders, so name interpreters by their full path in `command`. Its stdout is discarded and its stderr is kept, with its log, in Settings > Plugins.

## Manifest: `plugin.json`

```json
{
  "schema": 1,
  "id": "com.example.lights",
  "name": "Lights",
  "version": "1.2.0",
  "command": ["/usr/bin/python3", "plugin.py"],
  "actions": [
    { "id": "toggle", "label": "Toggle a light", "events": true, "params": [
      { "id": "entity", "label": "Light", "type": "text", "variables": true, "required": true }
    ] },
    { "id": "set", "label": "Set brightness", "params": [
      { "id": "entity", "label": "Light", "type": "text", "required": true },
      { "id": "level", "label": "Level", "type": "number", "min": 0, "max": 100, "default": 50 },
      { "id": "fade", "label": "Fade", "type": "bool" },
      { "id": "mode", "label": "Mode", "type": "choice", "choices": [{ "id": "warm", "label": "Warm" }, { "id": "cold", "label": "Cold" }] },
      { "id": "token", "label": "Access token", "type": "text", "secret": true }
    ] }
  ],
  "permissions": { "network": ["homeassistant.local:8123"], "variables": ["light_state"], "programs": false }
}
```

Unknown fields are refused, so a misspelt field is an error, not silently ignored. `schema` is read first, so a manifest of a newer schema is reported as newer, not as one with unknown fields.

| Field | Rules |
| --- | --- |
| `schema` | Required, `1`. An app refuses a schema it does not read, saying which one it reads, so a newer manifest is never misread |
| `id` | Reverse-DNS: at least two dot-separated labels of `a-z`, `0-9` and `-`, at most 128 bytes. Equal to the folder name |
| `name` | 1 to 64 bytes, one line |
| `version` | `MAJOR.MINOR.PATCH`, compared numerically |
| `command` | 1 to 16 items: the program, then its arguments, passed as they are (no shell). The program is a path inside the folder without `..`, or an absolute path |
| `actions` | 1 to 64, ids unique: 1 to 64 of `a-z`, `0-9`, `_`, `-` and `.`, starting with a letter. `label` 1 to 64 bytes |
| `events` | Optional, `true` or `false` (the default). With `true` the plugin also hears the other input on keys and controls bound to this action (see [Input events](#input-events)) |
| `params` | Up to 32 per action, ids unique, with the variable name rules (1 to 32 of `a-z`, `0-9` and `_`, starting with a letter) |
| `type` | `text`, `number`, `bool` or `choice` |
| `required` | The press fails, and the editor shows an error, when the parameter is missing |
| `variables` | Text only: `{{name}}` references to variables the profile declares are filled in when the key is pressed |
| `secret` | Text only: the value is kept in the Keychain, never in the profile (see [Secrets](#secrets)), and logged only by length |
| `min`, `max` | Number only |
| `choices` | Choice only, required there: 1 to 64, each with an id (action id rules) and a label. The value sent is the id |
| `default` | The value the editor starts with, of the parameter's type. Not for a secret |
| `permissions.network` | Hosts it says it connects to. Shown only |
| `permissions.variables` | Variables it may set. Enforced: `set_variable` for any other name is refused |
| `permissions.programs` | Whether it says it starts other programs. Shown only |

## Protocol

The plugin connects to `OSH_PLUGIN_SOCKET` (a Unix socket in a 0700 folder). Each message is one JSON object on one line, at most 64 KiB, in both directions. A longer line from the plugin closes the connection.

1. Within 2 seconds the plugin sends `{"type":"hello","id":"<OSH_PLUGIN_ID>","token":"<OSH_PLUGIN_TOKEN>"}`. The app answers `{"type":"welcome"}`, or `{"type":"error","error":"…"}` and closes. The token is new at every start and valid only for this plugin; a second connection is refused while one is open.
2. For each press, the app sends `{"type":"invoke","request":7,"action":"set","params":{"entity":"light.desk","level":40},"timeout_ms":15000,"input":{"gesture":"pressed","control":{"module":"m4315","index":3}}}`. Parameters with variables arrive with them filled in. `timeout_ms` is what is left of the press's time when the line is written; a press whose time ran out before it could be written is not sent. `input` is the input that ran the action (see [Input events](#input-events) for its fields); it is missing when the app was not told. The plugin answers each invoke once, in any order: `{"type":"result","request":7,"ok":true}` or `{"type":"result","request":7,"ok":false,"error":"the light is offline"}`. The error, cut to 512 bytes, is what the activity log shows.
3. At any time the plugin may send:
   - `{"type":"set_variable","id":1,"name":"light_state","value":"on"}`: sets a variable in every running module that declares it, as `osh` does. At most 10 a second, only names in `permissions.variables`, never `agent_decision` other than `none`. The app answers `{"type":"reply","id":1,"ok":true}` or `…"ok":false,"error":"…"}`.
   - `{"type":"log","message":"…"}`: kept in Settings > Plugins. At most 1 KiB each and 20 a second; more are counted and dropped.
   - `{"type":"ping"}`: the app answers `{"type":"pong"}`.
4. After 10 seconds without a line from the plugin, the app sends `{"type":"ping"}`. The plugin must send any line (`{"type":"pong"}`) within 5 seconds, or it is killed and restarted.
5. For an action with `"events": true`, the app also sends `event` messages, which are not answered (see [Input events](#input-events)).

**Ignore what you do not know.** A newer app may add fields to any message, and new message types. A plugin must ignore both, never fail on them.

## Input events

An invoke tells a plugin about the one gesture its action is bound to. With `"events": true` on the action, it also hears the rest of the input on the keys and controls bound to it, for example to hold a key (push to talk) or tell a knob's turn direction. Faders take only fader actions, so a plugin never hears one.

```json
{"type":"event","action":"talk","params":{"channel":"team"},"gesture":"released","control":{"module":"m4315","index":3}}
{"type":"event","action":"volume","params":{},"gesture":"counter_clockwise","control":{"module":"m003k","index":1}}
```

- **Which input.** A key or control is the action's while one of its bindings runs that action directly, as its own action (not as a step of a multi-action or an if / else). A gesture with no binding of its own there is sent as an `event`. A gesture bound to the action stays an `invoke`; one bound to anything else runs that, and the plugin is not told. So a key whose **Press** runs "Talk" sends its release as an event, but a release bound to an OBS action stays the OBS action's. When several of a plugin's actions are bound on one control, each that takes events hears it: once per distinct action and parameters, so the same action bound twice with different parameters hears it twice.
- **Keypad pages and dial functions.** A key is the action's only while its page is shown, and a release goes to the key that took the press even if the page changed meanwhile. The dial is the action's only while the function holding it is selected; a short press, which switches functions, is never an event there, and neither are the raw `pressed` and `released` that make up a click. Without dial functions the dial reports `pressed` and `released`, then `short_press` or `long_press` once the click ends, so an action bound to `long_press` hears the release before its invoke.
- **Fields.** `gesture` uses the profile's names: `pressed`, `released`, `clockwise`, `counter_clockwise`, `right`, `left`, `short_press`, `long_press`, `up`, `down`. `control` names the module (`m003k`, `m131e`, `m4315`, `m032le`) and the control's zero-based index. `params` are the bound action's parameters with variables filled in, so two keys bound to the same action can be told apart, but **without its secrets**.
- **Delivery.** In input order with the plugin's invokes, and after the invoke of the press that came before, since a release is handled once its press is answered. Only while the plugin is enabled, running and connected, and only from Live runs. Nothing is answered and nothing goes in the activity log. At most 50 a second per plugin: more are dropped, and the next one queued adds to the plugin's log how many were. Up to 64 events wait for a plugin that is busy and not reading its socket; more are dropped and counted. A slow reader is not disconnected for being slow, but it must still answer pings in time, and once more than 1 MiB is waiting for it, anything other than an event ends the connection. When the module falls far behind, a plugin's steps on one knob are merged into the latest. Treat each event as one step and do not count on hearing every one of a fast knob turn.
- **A press may have no release, and a release may have no press.** The plugin is not told when a key goes away (a page switch, a profile stop, a switch to Test, a reconnect), so a held key may never send its release. A release may come for a press made in a Test run, or for a press the module dropped because it had fallen far behind. Do not keep a key "on" forever waiting for one.

The example plugin's **Hold** action logs how long a key was held.

## How a press reaches the plugin

1. The module service runs the `plugin` action. With variables listed, it fills them in; a filled-in text that breaks the rules (one line, no control or invisible characters, at most 2048 bytes, 8192 for all text) fails the press before anything is sent.
2. The service writes a `plugin_invoke` record on its stdout. The app fills in the action's secrets from the Keychain, checks the invoke against the installed manifest (installed, at least `min_version`, the action exists, every parameter is declared and fits, required ones are present, only parameters that allow it take variables) and sends `invoke`.
3. The plugin's `result` goes back to the service as a `plugin_result` control request. A press fails when the plugin is not enabled, not running or still starting, a required secret is not set, has 8 invokes waiting, stops before answering, or answers later than 15 seconds (as a step of a multi-action or if / else, what is left of the sequence's 15 seconds).

While a press waits, other presses on the same module wait behind it, as for `run_program`.

## The action in a profile

```json
{ "type": "plugin", "plugin": "com.example.lights", "min_version": "1.2.0", "action": "toggle",
  "params": { "entity": "light.{{room}}" }, "variables": ["entity"] }
```

`variables` lists the text parameters whose references are filled in; the editor manages it (a parameter that allows variables is listed exactly when it holds `{{`). Every reference must name a declared variable, or the profile is refused when it loads. A profile that uses a missing plugin, or one older than `min_version`, still loads; that key's press fails with the reason.

The Execute confirmation names each plugin action with its parameters and the variables that feed it, and rates the profile high risk. A Live run whose saved profile adds a plugin action, or changes one's plugin, action, parameters or variables, pauses for a new confirmation. A JSON import holding a plugin action runs as a dry run until it is reviewed and allowed. Preset packs and starters cannot hold one.

## Not built yet

- Condition providers and plugin-supplied key images (design step 4), and telling a plugin when its key appears or disappears (a page switch, a profile stop).
- An installer or registry (step 6), and an optional `"background": true` plugin that runs without a Live profile (later).
- Plugins on Windows or Linux.
