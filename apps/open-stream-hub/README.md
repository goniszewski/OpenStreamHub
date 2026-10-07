# OpenStreamHub

Open-source control and automation for modular hardware. OpenStreamHub is an independent macOS configuration app for Cooler Master MasterHUB, built with Tauri, React and the CoolTEK Rust service. It supports profile editing, a live preview of connected modules, device-free action previews, offline display artwork preparation, official `.mhz` profile import, and running a profile's actions through a supervised `masterhub-service` process. Hardware behavior has not been verified on a device yet.

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

`npm run dev` opens a browser preview at `http://127.0.0.1:1420`. That preview provides the layout and sample data; profile storage and Rust-backed replay require the desktop app.

## Available workflow

- Select any of the five module types. Click a control on its module image or use the keyboard-accessible picker to edit its binding. Encoder, three-knob, 15-key, five-fader and two-roller bindings can be edited. Hardware behavior remains unverified.
- Create, edit, validate and save profiles using the service's JSON action format and strict validation. Schema-1 files remain readable and upgrade in memory; explicit Save stores schema 2.
- Organize the 15-key module into nested folders within one profile. Each folder reserves the top-left key for Back, with an `L1`–`L8` badge for the current level, and provides 14 content slots. The root keeps all 15 configurable keys. A breadcrumb, folder names, and per-page action editing are available; [folder behavior and limits](../../docs/masterhub-keypad-folders.md) are documented separately.
- Preview a selected binding or complete keypad click, or replay any of the five bundled synthetic module captures. Results show normalized events, planned/unbound actions, folder navigation and packet diagnostics. Navigation does not execute a newly displayed action when the original key is released.
- Preview absolute fader positions as output volume; bind key Press/Release and roller Up/Down events.
- Switch between system, light and dark appearance.
- **Rig preview:** arrange module instances in a rig strip. It is drawn from the draft profile, so edits appear as you make them. Connected modules are detected automatically (hot-plug and swap) and show live fader positions, pressed keys and knob rotation through a read-only feed; bundled synthetic captures can drive the same preview. The feed never enables software mode, writes to a device or runs actions.
- **Run profile:** run the current draft through a supervised `masterhub-service` child process. Dry run is the default; Execute (macOS only) needs an explicit confirmation naming the profile and device, and Stop is always one click. An imported `.mhz` profile (and anything derived from it) is dry run only until you allow it to execute actions. The service uses a snapshot of the draft and does not reload it, so the panel tells you when to restart after edits. On sleep the service is stopped cleanly; after wake, unplug or a crash it reconnects by module and serial. Execute runs never resume automatically and wait for your confirmation. See the [session manager](../../docs/masterhub-session-manager.md).
- **Focused-app rules (opt-in, macOS):** on the Profiles page, map an app bundle ID (typed, or filled in with *Use current app*) to a saved profile. When the rules are enabled and that app comes to the front, running sessions switch to the profile: dry runs restart at once, while runs that execute actions pause and wait for your confirmation before actions run with the new profile. Rules are off by default and stored in `focus-rules.json`, not in the profiles. See [focused-app rules](../../docs/masterhub-focus-rules.md).
- **Profiles:** duplicate, import and export. *Import* takes this app's JSON, or, with **Import official .mhz**, a MasterHUB profile export (a `.mhz` file or the `.zip` of several), read in memory with strict size limits. Either way it creates a new profile; a `.mhz` import is dry run only until you review its bindings and choose **Allow this imported profile to execute actions**, and unsupported or dangerous controls (programs, macros, timers, OBS and other integrations, images) stay unbound and are listed in a report. If the list of imported profiles is missing or damaged, no profile can run with actions until you choose **Mark all profiles as needing review**; see [importing MasterHUB profiles](../../docs/importing-masterhub-profiles.md). Export writes an owner-only JSON file in the app data folder (`.mhz` export is not implemented). Search bindings and see warnings for release/short-press overlaps and duplicate selectors. Undo/redo covers draft edits.
- **Profile keys, timers and OBS:** a `profile` action (next or previous) switches a session-managed run to another saved profile in name order, like a focused-app rule, and an executing run pauses for your confirmation; standalone `masterhub-service` runs cannot do this. A `timer` action counts down 1 s to 24 h and ends with a notification and a system sound (it is not drawn on the key); a second press cancels it, and stopping or sleeping cancels every running timer. An `obs_scene` action switches the OBS Studio program scene over a loopback obs-websocket (OBS 28 or later) connection. See the [action catalogue](../../docs/masterhub-actions.md).
- **Settings > OBS Studio:** off by default. Set the loopback host (127.0.0.1, ::1 or localhost) and port (OBS defaults to 4455); the endpoint is stored in `obs-settings.json`. The OBS password is kept in memory only, so enter it again after every app launch. The setting is read when you start a profile and does not change a running one.
- **Settings > Module displays:** an experimental, off-by-default toggle that sends images to the module displays, plus a choice of what the dial screen shows: the active dial function name, a digital clock, an analog clock, CPU usage (macOS only) or nothing. It applies only to profiles started with *Request device software mode*, is read at each Start, and has never run on hardware. See [session manager](../../docs/masterhub-session-manager.md#experimental-display-images).
- **Display artwork:** prepare a PNG or JPEG offline for the dial (240×240) or a key (72×72) with fit, rotation, background and quality controls, a live preview inside the module artwork, and an export to the app data folder. This tool only prepares and exports images; it does not send them (sending is the experimental module displays setting above).
- **Diagnostic bundle:** Settings can write one local JSON file with app and OS information, USB interfaces and HID descriptors (read without sending anything to a device), an optional short passive input capture, and recent run activity. Serial numbers are hidden by default, text snippets, URLs, timer labels and OBS scene names in embedded profiles are redacted when serial redaction is on, the OBS password is never included, and nothing is uploaded. See the [diagnostic bundle](../../docs/masterhub-diagnostic-bundle.md).
- **Shortcuts and notices:** ⌘S saves, ⌘D duplicates, ⌘Z / ⇧⌘Z undo and redo, ⌘1–4 switch pages, ⌘F searches on the Profiles page (Ctrl on other platforms). In-app notices report module connect, disconnect and swap, and a skippable getting-started guide opens on first launch.

Replay and previews use `DryRunDispatcher`. The app executes host actions only through a service run you start with Execute. It has no arbitrary file access, browser integration or user scripts. Physical control placement and dial/roller direction still require device validation.

Profiles are stored in the application's local data directory, shown in Settings. A fresh store starts with the sample profile. Saving uses an atomic replacement; invalid or corrupted data produces an error and is preserved. Bootstrap selects the most recently saved profile.

Alongside `profiles`, the app data folder holds `focus-rules.json` (focused-app rules, written atomically and owner-only), `exports/` (profile exports), `diagnostics/` (diagnostic bundles), `artwork/` (prepared images), `display-settings.json` and `obs-settings.json` (the two Settings above), `imported-profiles.json` (which profiles are dry run only) and `service-profiles/` (temporary profile snapshots for running services). Files there are created owner-only and the oldest are pruned.

On macOS, the app uses `~/Library/Application Support/dev.cooltek.openstreamhub/profiles`. If that store is absent, existing profiles from `dev.cooltek.masterhub/profiles` are validated and copied without changing their IDs or contents. The previous files remain intact. An existing OpenStreamHub store is never overwritten or merged automatically; invalid legacy data is reported for recovery.

Appearance reads the previous preference key when it is available in the webview's local storage. A new macOS bundle identity can have separate webview storage and start with System appearance; profile migration does not copy that storage.

Unsaved drafts prompt before profile changes, window closing, or Quit OpenStreamHub / Command-Q. Drafts are also saved to the webview's local storage as you edit, so after a crash, Dock Quit or system termination the app offers to restore them on the next launch. Saving to disk is still explicit; a draft restored over a file that changed meanwhile is protected by the save conflict check.

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

- Sending images to the module displays is experimental and unverified on hardware; GIF animation, dial backlight and colour, and firmware controls are not implemented.
- Remaining OBS actions (recording, streaming, studio mode, transitions, sources, audio, replay buffer, virtual camera) and Keychain storage for the OBS password; Twitch, Discord and Streamlabs integrations.
- Macro recording. Media keys and media stop (Apple Music and Spotify only) execute but are unverified on a real Mac.
- Official `.mhz` export (a nice-to-have), and checking the importer against a real user export.
- Multi-module live action routing, browser-tab cards, flows and script integrations (the last three do not exist in the official app).
- Hardware acceptance of input, displays, timers, OBS and sleep/wake, then signing and notarization (including the sidecar) before a public release.
