# Writing a plugin

Status (6 November 2026): **first version, for the maintainer's own plugins.** The manifest and the protocol may still change; a stable, versioned schema comes before community plugins ([design](plugins-design.md), step 5). Built on macOS only.

A plugin adds actions a key can run, such as switching smart lights. It is a separate program that the OpenStreamHub app starts and talks to over a local socket. It can be written in any language. A working example in Python, using only the standard library, is in [`examples/plugins/com.example.notify`](../examples/plugins/com.example.notify).

**A plugin runs as you.** It can do anything you could do. The permissions a manifest lists are shown to the user but nothing enforces them, apart from the variables it may set.

## Installing

Put the plugin's folder in the plugins folder, `~/Library/Application Support/<app id>/plugins/`; Settings > Plugins has a button that opens it. The folder's name must be the plugin's id, such as `com.example.notify`. The folder and `plugin.json` must belong to you and must not be writable by other users (`chmod go-w`). Settings > Plugins lists every folder with its manifest, or the reason it cannot be used.

## When it runs

- The app starts a plugin only while an **executing (Live)** module run uses it, and keeps one instance for all modules. A Test run never starts a plugin; a press there only logs what would run.
- It stops the plugin 5 seconds after the last Live run that uses it stops, so a module that restarts (a large profile change, a replug) keeps the same plugin process. On quit, every plugin stops.
- A plugin that exits, closes its connection, misses a ping or has not said hello 10 seconds after it started is restarted after 1, 2, 4, 8, 16, then 30 seconds. After 5 failures within 5 minutes it stays stopped until a profile that uses it is executed again.
- Stopping sends SIGTERM to the plugin's process group, then SIGKILL after 1 second, so helpers it started stop with it.
- The plugin starts in its folder with the app's environment plus `OSH_PLUGIN_SOCKET`, `OSH_PLUGIN_ID` and `OSH_PLUGIN_TOKEN`. Started from the Finder, `PATH` holds only the system folders, so name interpreters by their full path in `command`. Its stdout is discarded and its stderr is kept, with its log, in Settings > Plugins.

## Manifest: `plugin.json`

```json
{
  "id": "com.example.lights",
  "name": "Lights",
  "version": "1.2.0",
  "command": ["/usr/bin/python3", "plugin.py"],
  "actions": [
    { "id": "toggle", "label": "Toggle a light", "params": [
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

Unknown fields are refused, so a misspelt field is an error, not silently ignored.

| Field | Rules |
| --- | --- |
| `id` | Reverse-DNS: at least two dot-separated labels of `a-z`, `0-9` and `-`, at most 128 bytes. Equal to the folder name |
| `name` | 1 to 64 bytes, one line |
| `version` | `MAJOR.MINOR.PATCH`, compared numerically |
| `command` | 1 to 16 items: the program, then its arguments, passed as they are (no shell). The program is a path inside the folder without `..`, or an absolute path |
| `actions` | 1 to 64, ids unique: 1 to 64 of `a-z`, `0-9`, `_`, `-` and `.`, starting with a letter. `label` 1 to 64 bytes |
| `params` | Up to 32 per action, ids unique, with the variable name rules (1 to 32 of `a-z`, `0-9` and `_`, starting with a letter) |
| `type` | `text`, `number`, `bool` or `choice` |
| `required` | The press fails, and the editor shows an error, when the parameter is missing |
| `variables` | Text only: `{{name}}` references to variables the profile declares are filled in when the key is pressed |
| `secret` | Text only: masked in the editor and logged only by length. It is still saved in the profile, so an exported profile carries it |
| `min`, `max` | Number only |
| `choices` | Choice only, required there: 1 to 64, each with an id (action id rules) and a label. The value sent is the id |
| `default` | The value the editor starts with, of the parameter's type |
| `permissions.network` | Hosts it says it connects to. Shown only |
| `permissions.variables` | Variables it may set. Enforced: `set_variable` for any other name is refused |
| `permissions.programs` | Whether it says it starts other programs. Shown only |

## Protocol

The plugin connects to `OSH_PLUGIN_SOCKET` (a Unix socket in a 0700 folder). Each message is one JSON object on one line, at most 64 KiB, in both directions. A longer line from the plugin closes the connection.

1. Within 2 seconds the plugin sends `{"type":"hello","id":"<OSH_PLUGIN_ID>","token":"<OSH_PLUGIN_TOKEN>"}`. The app answers `{"type":"welcome"}`, or `{"type":"error","error":"…"}` and closes. The token is new at every start and valid only for this plugin; a second connection is refused while one is open.
2. For each press, the app sends `{"type":"invoke","request":7,"action":"set","params":{"entity":"light.desk","level":40},"timeout_ms":15000}`. Parameters with variables arrive with them filled in. The plugin answers each invoke once, in any order: `{"type":"result","request":7,"ok":true}` or `{"type":"result","request":7,"ok":false,"error":"the light is offline"}`. The error, cut to 512 bytes, is what the activity log shows.
3. At any time the plugin may send:
   - `{"type":"set_variable","id":1,"name":"light_state","value":"on"}`: sets a variable in every running module that declares it, as `osh` does. At most 10 a second, only names in `permissions.variables`, never `agent_decision` other than `none`. The app answers `{"type":"reply","id":1,"ok":true}` or `…"ok":false,"error":"…"}`.
   - `{"type":"log","message":"…"}`: kept in Settings > Plugins. At most 1 KiB each and 20 a second; more are counted and dropped.
   - `{"type":"ping"}`: the app answers `{"type":"pong"}`.
4. After 10 seconds without a line from the plugin, the app sends `{"type":"ping"}`. The plugin must send any line (`{"type":"pong"}`) within 5 seconds, or it is killed and restarted.

## How a press reaches the plugin

1. The module service runs the `plugin` action. With variables listed, it fills them in; a filled-in text that breaks the rules (one line, no control or invisible characters, at most 2048 bytes, 8192 for all text) fails the press before anything is sent.
2. The service writes a `plugin_invoke` record on its stdout. The app checks it against the installed manifest (installed, at least `min_version`, the action exists, every parameter is declared and fits, required ones are present, only parameters that allow it take variables) and sends `invoke`.
3. The plugin's `result` goes back to the service as a `plugin_result` control request. A press fails when the plugin is not running or still starting, has 8 invokes waiting, stops before answering, or answers later than 15 seconds (as a step of a multi-action or if / else, what is left of the sequence's 15 seconds).

While a press waits, other presses on the same module wait behind it, as for `run_program`.

## The action in a profile

```json
{ "type": "plugin", "plugin": "com.example.lights", "min_version": "1.2.0", "action": "toggle",
  "params": { "entity": "light.{{room}}" }, "variables": ["entity"] }
```

`variables` lists the text parameters whose references are filled in; the editor manages it (a parameter that allows variables is listed exactly when it holds `{{`). Every reference must name a declared variable, or the profile is refused when it loads. A profile that uses a missing plugin, or one older than `min_version`, still loads; that key's press fails with the reason.

The Execute confirmation names each plugin action with its parameters and the variables that feed it, and rates the profile high risk. A Live run whose saved profile adds a plugin action, or changes one's plugin, action, parameters or variables, pauses for a new confirmation. A JSON import holding a plugin action runs as a dry run until it is reviewed and allowed. Preset packs and starters cannot hold one.

## Not built yet

- Events for the plugin's own controls, condition providers and plugin-supplied key images (design step 4).
- A first-enable dialog, Keychain storage for secret parameters, a stable manifest schema and an installer (steps 5 and 6).
- Plugins on Windows or Linux.
