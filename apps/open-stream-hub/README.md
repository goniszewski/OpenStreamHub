# OpenStreamHub

Open-source control and automation for modular hardware. OpenStreamHub is an independent macOS configuration app for Cooler Master MasterHUB, built with Tauri, React and the CoolTEK Rust service. It supports profile editing, a live preview of connected modules, dry-run action previews, key and dial artwork, official `.mhz` profile import, and running a profile's actions through a supervised `masterhub-service` process. Behaviour is verified on hardware per module; the Compatibility page and `docs/masterhub-hardware-results.md` list what has been checked.

OpenStreamHub is not affiliated with or endorsed by Cooler Master. MasterHUB identifies the supported hardware, not the app's publisher.

## Run locally

The app requires macOS 12.3 or later. For development, use Node.js 22.12 or later, Rust 1.97 or later, and the macOS Xcode command-line tools. From this directory:

```sh
npm ci
npm run desktop
```

`npm run desktop` and `npm run desktop:build` first build the `masterhub-service` sidecar (`npm run sidecar`). On a fresh checkout, run `npm run sidecar` once before `cargo test` or `cargo clippy` in `src-tauri`, because Tauri requires the sidecar file to exist. See [building the desktop app](../../docs/building-the-desktop-app.md).

Create a local debug application bundle:

```sh
npm run desktop:build
```

The app is written to `src-tauri/target/debug/bundle/macos/OpenStreamHub.app`. Bundled builds contain their frontend and do not require a development server.

`npm run dev` opens a browser preview at `http://127.0.0.1:1420`. That preview provides the layout and sample data; profile storage and hardware access require the desktop app.

## Available workflow

- The workspace is one screen: the module catalog, the module image (canvas) with a collapsible bindings drawer, and the inspector on the right. Profiles — the saved list, create/duplicate/delete, import/export, the open profile's name, validation, earlier versions, the dry-run flag and focused-app rules — open as a sheet from the Manage profiles button next to the profile switcher in the header. While a profile uses dial functions, the encoder's inspector is the dial functions editor.
- Select any of the five module types. Click a control on its module image or use the keyboard-accessible picker to edit its binding. Actions are chosen in a searchable picker grouped by category (Audio, Keyboard, Apps & web, Media, System, OBS, Timers & sequences, Navigation), with the most recently used actions at the top; faders are offered only the position-driven action, and entries can be dragged from the picker onto a control on the module image or a keypad tile. Encoder, three-knob, 15-key, five-fader and two-roller bindings can be edited. Hardware behavior remains unverified.
- Create, edit, validate and save profiles using the service's JSON action format and strict validation. Schema-1 files remain readable and upgrade in memory; explicit Save stores schema 2.
- Organize the 15-key module into nested folders within one profile. Each folder reserves the top-left key for Back, with an `L1`–`L8` badge for the current level, and provides 14 content slots. The root keeps all 15 configurable keys. A breadcrumb, folder names, and per-page action editing are available; [folder behavior and limits](../../docs/masterhub-keypad-folders.md) are documented separately.
- Preview a selected binding or complete keypad click. Results show the events, what would run, controls with no action bound, folder navigation and packet diagnostics. Navigation does not execute a newly displayed action when the original key is released.
- Preview absolute fader positions as output volume; bind key Press/Release and roller Up/Down events.
- Switch between system, light and dark appearance.
- **Rig preview:** arrange module instances in a rig strip. It is drawn from the draft profile, so edits appear as you make them. Connected modules are detected automatically (hot-plug and swap) and show live fader positions, pressed keys and knob rotation through a read-only feed; the built-in module samples can drive the same preview. The feed never enables software mode, writes to a device or runs actions.
- **Run profile:** Run and Stop live in the app header, with the run state next to them (Stopped, Dry run, Executing, Paused, Waiting and more), so starting and stopping never needs scrolling. The Run button names the module it would start on ("Run on 15-key") and turns red whenever it would run with actions; when a run is blocked, a full-width strip under the header says why on every page. The workspace's Run profile section picks the device and the run mode for that control. The run itself goes through a supervised `masterhub-service` child process. Dry run is the default; Execute (macOS only) needs an explicit confirmation naming the profile and device. An imported `.mhz` profile (and anything derived from it) is dry run only until you allow it to execute actions. The service uses a snapshot of the draft and does not reload it, so the panel tells you when to restart after edits. On sleep the service is stopped cleanly; after wake, unplug or a crash it reconnects by module and serial. Execute runs never resume automatically and wait for your confirmation. See the [session manager](../../docs/masterhub-session-manager.md).
- **Focused-app rules (opt-in, macOS):** in the profile manager (the Manage profiles button next to the profile switcher in the header), map an app bundle ID (typed, or filled in with *Use current app*) to a saved profile. When the rules are enabled and that app comes to the front, running sessions switch to the profile: dry runs restart at once, while runs that execute actions pause and wait for your confirmation before actions run with the new profile. Rules are off by default and stored in `focus-rules.json`, not in the profiles. See [focused-app rules](../../docs/masterhub-focus-rules.md).
- **Profiles:** duplicate, import and export. *Import* takes this app's JSON, or, with **Import official .mhz**, a MasterHUB profile export (a `.mhz` file or the `.zip` of several), read in memory with strict size limits. Either way it creates a new profile; a `.mhz` import is dry run only until you review its bindings and choose **Allow this imported profile to execute actions**, and unsupported or dangerous controls (programs, macros, timers, OBS and other integrations, images) stay unbound and are listed in a report. If the list of imported profiles is missing or damaged, no profile can run with actions until you choose **Mark all profiles as needing review**; see [importing MasterHUB profiles](../../docs/importing-masterhub-profiles.md). Export writes an owner-only JSON file in the app data folder (`.mhz` export is not implemented). Search bindings and see warnings for release/short-press overlaps and duplicate selectors. Undo/redo covers draft edits.
- **Profile keys, timers and OBS:** a `profile` action (next or previous) switches a session-managed run to another saved profile in name order, like a focused-app rule, and an executing run pauses for your confirmation; standalone `masterhub-service` runs cannot do this. A `timer` action counts down 1 s to 24 h and ends with a notification and a system sound (it is not drawn on the key); a second press cancels it, and stopping or sleeping cancels every running timer. An `obs_scene` action switches the OBS Studio program scene over a loopback obs-websocket (OBS 28 or later) connection. See the [action catalogue](../../docs/masterhub-actions.md).
- **Settings > OBS Studio:** off by default. Set the loopback host (127.0.0.1, ::1 or localhost) and port (OBS defaults to 4455); the endpoint is stored in `obs-settings.json`. The OBS password is kept in the macOS Keychain and read back once per launch; if the Keychain refuses, it is held in memory for that launch only. The setting is read when you start a profile and does not change a running one.
- **Settings > Let OpenStreamHub drive the display:** **App** (the default) runs the 15-key module and the encoder dial in software mode whenever a profile runs on them and keeps their displays in sync (placeholder key images and folder pages for profiles with 15-key pages, nothing drawn for a flat one; on the dial, the active dial function name, a digital clock, an analog clock, CPU usage on macOS, or nothing). **Built-in animation** leaves them in their own mode, as the old toggle's off did; a profile with 15-key pages then cannot run live. Other modules are not affected. The setting is read at each Start and never changes a running profile. Tested on one M4315 and one M131E; see [session manager](../../docs/masterhub-session-manager.md#module-control) and [hardware results](../../docs/masterhub-hardware-results.md).
- **Lighting:** the Lighting page applies the encoder dial's backlight (0–100) and one solid colour to the hardware as you adjust them, per picked module or to every connected dial at once (Whole kit), with favourite colours kept locally. Applying puts the dial in software mode — its built-in animation stops and its screen stays dark until a run draws on it — and Reset hands the dial back so its own animation returns. A dial a run is driving is skipped and the controls say so; other modules show disabled controls with the reason (knob rings, base edge and fader lighting are not available yet). Lighting was verified on one M131E through the CLI; see [hardware results](../../docs/masterhub-hardware-results.md).
- **Dial and key artwork:** a key's picture is set in the key artwork editor in the workspace inspector (fit, rotation, background, quality and a design studio). The encoder dial's picture is set on the dial's Display tab in the inspector (Dial screen shows: Image). Both are prepared as the exact JPEG the module receives, kept in the profile and drawn during a run while OpenStreamHub drives the display. When a picture or GIF cannot be used, the editor says in plain language what to change (a smaller picture, fewer frames, another file type) and keeps the technical error text under a **Details** disclosure to read or copy.
- **Key artwork editor:** in the workspace inspector, build a key's artwork from a bundled icon library (search and recently used icons, all local — nothing is fetched), a suggested icon for the assigned action (including the installed application's own icon for an Open application action), and label text with four sizes, top/middle/bottom alignment and text/icon/background colours. The preview and the key receive the same prepared 72×72 JPEG. A design is saved with the artwork and the studio reopens with it (an icon that is no longer offered falls back to the default suggestion); applying an uploaded image clears the stored design. Uploaded PNG, JPEG or GIF images work as before. Assigning an Open application or Open URL action to a key that has no artwork draws it for you a moment after you settle on the application or address: the installed application's own icon, or a globe with the site's host (no site icon is fetched — that would need the network). Only artwork you have not designed is ever replaced (a key with no artwork, or one still exactly as drawn for another application), Remove artwork declines it for that assignment, and the drawing is one undo step that stays editable in the studio.
- **Diagnostic bundle:** Settings can write one local JSON file with app and OS information, USB interfaces and HID descriptors (read without sending anything to a device), an optional short passive input capture, and recent run activity. Serial numbers are hidden by default, text snippets, URLs, timer labels and OBS scene names in embedded profiles are redacted when serial redaction is on, the OBS password is never included, and nothing is uploaded. See the [diagnostic bundle](../../docs/masterhub-diagnostic-bundle.md).
- **Starters:** the first launch on a fresh, empty profile store offers three starter profiles — Mac basics, Streaming (OBS) and Editing and media. Picking one creates a new saved profile and opens it; starters hold only volume, playback, screenshot, timer and name-free OBS actions, so nothing is typed into apps and no program or web page is opened. When a module is connected later, the starters that use it are offered once for that appearance; the profile manager's **New from a starter…** opens the same chooser any time. The getting-started guide stays reachable from Help in the sidebar and from Settings.
- **Shortcuts and notices:** ⌘S saves, ⌘D duplicates, ⌘Z / ⇧⌘Z undo and redo, ⌘1–5 switch pages (Workspace, Lighting, Demos, Compatibility, Settings), ⌘F opens the bindings drawer and focuses its search (Ctrl on other platforms). In-app notices report module connect, disconnect and swap; the getting-started guide opens from Help and Settings.

Sample tests and previews use `DryRunDispatcher`. The app executes host actions only through a service run you start with Execute. It has no arbitrary file access, browser integration or user scripts. Physical control placement and dial/roller direction still require device validation.

Profiles are stored in the application's local data directory, shown in Settings. A fresh store opens the starter chooser first; starting empty leaves an unsaved new profile open. Saving uses an atomic replacement; invalid or corrupted data produces an error and is preserved. Bootstrap selects the most recently saved profile. Overwrites keep the previous version in a hidden `.history/<profile id>` folder, spaced at least five minutes apart (the first overwrite after the app starts and a restore always keep one; newest 20 per profile, 16 MiB at most) so the profile manager can restore an earlier one.

Alongside `profiles`, the app data folder holds `focus-rules.json` (focused-app rules, written atomically and owner-only), `exports/` (profile exports), `diagnostics/` (diagnostic bundles), `artwork/` (prepared images), `display-settings.json` and `obs-settings.json` (the two Settings above), `imported-profiles.json` (which profiles are dry run only) and `service-profiles/` (temporary profile snapshots for running services). Files there are created owner-only and the oldest are pruned.

On macOS, the app uses `~/Library/Application Support/dev.cooltek.openstreamhub/profiles`. If that store is absent, existing profiles from `dev.cooltek.masterhub/profiles` are validated and copied without changing their IDs or contents. The previous files remain intact. An existing OpenStreamHub store is never overwritten or merged automatically; invalid legacy data is reported for recovery.

Appearance reads the previous preference key when it is available in the webview's local storage. A new macOS bundle identity can have separate webview storage and start with System appearance; profile migration does not copy that storage.

Saving is automatic by default (Settings > Profile storage > Save changes automatically): an edit is written to the profile file about a second after it settles, and a save keeps the previous version in the store's hidden `.history` folder (at most one every five minutes while editing, plus the first save after the app starts) — the profile manager lists the last 20 per profile, each with its binding count against the saved profile, under "Earlier versions" and can put one back (as an ordinary, conflict-checked save). Switching profiles, closing the window or starting a run settle the pending edit first, so there is normally no unsaved-changes prompt. With the setting off, edits prompt before profile changes or window closing, and ⌘S / Save profile writes the file. Changes that are not saved yet (a new profile, or a name still being retyped) are also kept as a recovery copy in the webview's local storage and offered after a crash, Dock Quit or system termination; a copy restored over a file that changed meanwhile is protected by the save conflict check.

## Privacy

OpenStreamHub has no telemetry, analytics, crash reporting or update checks, and it does not contact any server on its own. The only network connection is the optional, off-by-default OBS scene switch, which talks to OBS on this same Mac. Diagnostic bundles, profiles and artwork stay in the app data folder until you share them yourself. The details, and how to verify them, are in [Privacy and telemetry](../../README.md#privacy-and-telemetry) in the main README.

## Verify

```sh
npm run check
npm test
npm run build
npm run test:rust
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
```

The synthetic capture fixtures exercise the same capture parser, input normalization and binding engine as the CLI. They do not establish physical hardware compatibility.

## Repository structure

The app lives alongside the SDK so profile and protocol changes can be reviewed together. Its Rust manifest is a separate Cargo workspace with path dependencies on the SDK crates; its own Cargo and npm lockfiles keep desktop dependencies out of the SDK's normal workspace build. The SDK remains usable independently of the app.

Frontend code is under `src/`, the narrow desktop command bridge is under `src-tauri/`, and module illustrations are under `public/modules/`. Local design studies and visual QA artifacts are ignored. The app can move into a separate repository later if it needs an independent maintenance or release lifecycle.

## Next milestones

Still open:

- Sending images to the module displays is experimental; the user saw the dial picture (11–12 October 2026) and the dial GIF animation (19 October 2026) working on a real M131E, and key artwork and key GIF animation ran on an M4315; firmware controls are not implemented.
- Twitch, Discord and Streamlabs integrations.
- Media keys and media stop were checked on a real Mac with Apple Music only; browsers and Spotify are unchecked.
- Official `.mhz` export (a nice-to-have), and checking the importer against a real user export.
- Multi-module live action routing, browser-tab cards, flows and script integrations (the last three do not exist in the official app).
- Hardware acceptance of input, displays, timers, OBS and sleep/wake, then signing and notarization (including the sidecar) before a public release.
