# App preset packs

A preset pack is one JSON file in this folder: ready-made bindings for one application. The app reads every `*.json`
file here at build time, checks it, and lists the valid ones under **App presets** in the starter chooser (first run,
**New from a starter…**, and when a module is connected). Adding a pack needs no code: add a file and open a pull request.

A pack becomes a new saved profile that can run only as a **dry run** until the person reviews it and allows it to
execute actions in the profile manager. If the file names an app, the app offers a focused-app rule that switches to
the profile when that app comes to the front.

## Format

```json
{
  "pack_version": 1,
  "id": "my-app",
  "name": "My App",
  "description": "One or two sentences: what the pack gives you and where it puts it.",
  "app": { "name": "My App", "bundle_id": "com.example.MyApp" },
  "license": "CC0-1.0",
  "source": "Where the shortcuts come from: the vendor's documentation, or original work.",
  "verified": false,
  "focus_rule": { "bundle_id": "com.example.MyApp" },
  "profile": { "schema_version": 3, "name": "My App", "bindings": [], "keypad": { "pages": [] } }
}
```

- `id`: 2–40 lowercase letters, digits or hyphens; unique among packs.
- `license`: one of `CC0-1.0`, `MIT`, `Apache-2.0`, `CC-BY-4.0`. By adding the file you agree it may be shipped and copied under it.
- `source`: say where the shortcuts and layout come from. Do not copy text from anyone's documentation; list the shortcuts themselves.
- `verified`: `true` only if you checked every shortcut against the application on a Mac. `false` is honest and fine; the chooser says so.
- `focus_rule` is optional; `app.bundle_id` is optional.
- `profile` is an ordinary keypad-page profile (schema 3 for new packs; schema 2 packs still load): flat bindings for knobs, the encoder, faders and rollers, and one `root` keypad page with 15 slots of single press tiles. Look at an existing pack.

## What a pack may bind

Shortcuts (a lowercase letter or digit, or a named key such as `space`, `return`, `arrow_left`, `f13` or `keypad_enter`,
with `command`, `control`, `option` and `shift`), media keys, system actions (undo, redo, screenshot), volume,
mute, timers and the simple OBS toggles. A pack cannot type text, open URLs, launch apps or run sequences, so reading one
shows everything it can do. A pack binds at least three controls.

## Checks

`npm test` checks every file here (the app's rules) and the Rust tests run each pack's profile through the service's own
validator, so a wrong key name fails in review, not on someone's desk. A file that fails is skipped and listed, with the reasons, in the chooser.
