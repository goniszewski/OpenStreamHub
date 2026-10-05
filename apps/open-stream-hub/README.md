# OpenStreamHub

Open-source control and automation for modular hardware. OpenStreamHub is an independent macOS configuration app for Cooler Master MasterHUB, built with Tauri, React and the CoolTEK Rust service. This first version supports profile editing and device-free action previews. It does not connect to hardware or execute Mac actions.

OpenStreamHub is not affiliated with or endorsed by Cooler Master. MasterHUB identifies the supported hardware, not the app's publisher.

## Run locally

The app requires macOS 12.3 or later. For development, use Node.js 22.12 or later, Rust 1.97 or later, and the macOS Xcode command-line tools. From this directory:

```sh
npm ci
npm run desktop
```

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

All replay and preview actions use `DryRunDispatcher`. There is no command for HID connection, host action execution, arbitrary file access, browser integration or user scripts in this version. Physical control placement and dial/roller direction still require device validation.

Profiles are stored in the application's local data directory, shown in Settings. A fresh store starts with the sample profile. Saving uses an atomic replacement; invalid or corrupted data produces an error and is preserved. Bootstrap selects the most recently saved profile.

On macOS, the app uses `~/Library/Application Support/dev.cooltek.openstreamhub/profiles`. If that store is absent, existing profiles from `dev.cooltek.masterhub/profiles` are validated and copied without changing their IDs or contents. The previous files remain intact. An existing OpenStreamHub store is never overwritten or merged automatically; invalid legacy data is reported for recovery.

Appearance reads the previous preference key when it is available in the webview's local storage. A new macOS bundle identity can have separate webview storage and start with System appearance; profile migration does not copy that storage.

Unsaved drafts prompt before profile changes, window closing, or Quit OpenStreamHub / Command-Q. Save before using Dock Quit or system termination: those macOS termination paths can bypass the app's draft prompt. Unsaved drafts are not recovered after termination.

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

Focused-app rules, browser-tab cards, live device sessions, dial artwork, flows and script integrations remain separate implementation milestones. Hardware acceptance and signing/notarization are required before a public release.
