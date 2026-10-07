# OpenStreamHub

[![License](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](#license)
[![Conventional Commits](https://img.shields.io/badge/Conventional%20Commits-1.0.0-%23FE5196?logo=conventionalcommits&logoColor=white)](https://conventionalcommits.org)

An independently maintained, unofficial SDK and desktop app for [Cooler Master MasterHUB](https://www.coolermaster.com/products/masterhub.html), focused on macOS support.

**Current scope: MasterHUB only.** OpenStreamHub brings together a Rust SDK,
diagnostic tools, a profile and binding service, and a desktop editor.
The Rust crates retain their `cooltek-sdk` names.

Based on [CoolTEK SDK by projectzero7](https://github.com/projectzero7/cooltek-sdk).
See [Origins and attribution](#origins-and-attribution) for project lineage.

## Contents

- [Current status](#current-status)
- [Privacy and telemetry](#privacy-and-telemetry)
- [Components](#components)
- [Getting started](#getting-started)
- [Documentation and examples](#documentation-and-examples)
- [Contributing](#contributing)
- [Origins and attribution](#origins-and-attribution)
- [License](#license)

## Current status

MasterHUB support is in development. The SDK builds on macOS, and tests use
synthetic reports and fake transports. **Physical input, display acceptance,
and sleep/wake recovery still require hardware validation.**

| Area | Implemented capabilities |
| --- | --- |
| Device support | Recognition of both USB ID generations and input decoding for all five control modules |
| Displays and sessions | Explicit IPS mode controls, firmware queries, descriptor-sized JPEG packets for both IPS modules, and session lifecycle hooks |
| Profiles and bindings | JSON profiles, offline action plans, and explicit macOS volume, mute, shortcut, text, URL and app actions, profile next/previous, an OBS scene switch, plus [countdown timers](docs/masterhub-actions.md#countdown-timer-timer) that end with a notification and a system sound (not shown on the key) |
| Desktop editor | All-module catalog and binding editor, nested folders for the 15-key module, a live module rig preview, hot-plug module detection, profile import/export/duplicate, import of official `.mhz` profiles (dry run only until allowed), focused-app profile rules, undo/redo, search and conflict warnings, offline display artwork preparation, and a local diagnostic bundle |
| Running actions | The desktop app can run a profile through a supervised `masterhub-service` process: dry run by default, explicit confirmation before Execute, clean stop on sleep, and reconnect after wake or unplug (Execute runs never resume without confirmation) |

Nothing is uploaded anywhere: profiles, diagnostic bundles and prepared images stay on your Mac. See [Privacy and telemetry](#privacy-and-telemetry).
A profile key can switch the OBS Studio program scene (`obs_scene`). It talks only to OBS on the same Mac (obs-websocket, OBS 28+), is off until you turn OBS on in Settings, and the OBS password is kept in memory only, so you enter it again after each app launch. The other OBS actions of the official app are not implemented yet; see [actions](docs/masterhub-actions.md#obs-scene-obs_scene).
Sending images to a module display is off by default, because display behavior is not verified on hardware. An experimental Settings toggle can turn it on for testing (placeholder key images and folder pages, and a dial function name, digital or analog clock, or CPU usage on the encoder's screen); it has never run on real hardware.
For detailed capability status, see the [implementation status](docs/masterhub-implementation-status.md)
and [protocol evidence table](docs/masterhub-protocol-evidence.md).

## Privacy and telemetry

OpenStreamHub has **no telemetry**. It does not collect usage data, analytics or crash reports, it does not check for updates, and it does not contact any server on its own. The desktop app, the `masterhub-service` sidecar and the command-line tools all behave this way, and there is no account, sign-in or cloud sync.

- **Network use.** The only network code in the project is the optional OBS scene switch (`obs_scene`). It is off until you turn it on in Settings, and it only connects to OBS on this same Mac (`127.0.0.1`, `::1` or `localhost`; the code refuses any other address). The OBS password is kept in memory only and is never written to disk, logs, diagnostics or the command line. See [actions](docs/masterhub-actions.md#obs-scene-obs_scene).
- **Local files.** Profiles, settings, exports, prepared artwork and diagnostic bundles are written to the app's data folder on your Mac (owner-only permissions) and nothing sends them anywhere. A diagnostic bundle is a file you create and choose whether to share; serial numbers, text snippets, web addresses, timer labels and OBS scene names are redacted by default. See the [diagnostic bundle](docs/masterhub-diagnostic-bundle.md).
- **Webview.** The app window can only load its own bundled files. Its content security policy allows connections to the app's internal IPC channel only (`connect-src ipc: http://ipc.localhost`, in `apps/open-stream-hub/src-tauri/tauri.conf.json`), and the Tauri configuration has no updater or HTTP plugin.
- **Dependencies.** None of the Rust or npm dependencies is an analytics, crash-reporting or HTTP client library. The only Tauri plugin is `tauri-plugin-single-instance`.
- **Scripts.** The hardware capture scripts in `scripts/hardware/` make no network calls.

Building from source downloads dependencies from crates.io and npm, as any build does; that is not the app. These statements come from reading the code and configuration and are not a third-party audit. To check them yourself, search the sources for network code or watch the app with a tool such as Little Snitch or `lsof -i` while it runs. If you find a way the app contacts anything other than your local OBS, please open an issue. Any future feature that needs the network, such as an updater, would be opt-in and documented here first.

## Components

| Component | Purpose |
| --- | --- |
| Rust SDK | MasterHUB device communication, input decoding, display packet preparation, and session handling |
| Diagnostic tools | Device inventory, bounded raw recording, command previews, image preparation, and offline replay |
| Profile and binding service | JSON profiles, action planning, and explicit macOS actions |
| [Desktop app](apps/open-stream-hub/README.md) | Profile editing, live module preview, supervised action runs, offline artwork preparation and diagnostics |

## Getting started

The repository is currently private; Git access requires an authorized GitHub account.

### Clone and build

```bash
git clone git@github.com:goniszewski/OpenStreamHub.git
cd OpenStreamHub
cargo build
```

### Run the desktop app

From the repository root:

```bash
cd apps/open-stream-hub
npm ci
npm run desktop
```

`npm run desktop` builds the `masterhub-service` sidecar first. See the
[desktop app guide](apps/open-stream-hub/README.md) for prerequisites and
app development details, and [building the desktop app](docs/building-the-desktop-app.md)
for sidecar bundling and signing notes.

### Use the Rust SDK

Add the following to your `Cargo.toml` file:

```toml
[dependencies]
cooltek-sdk = { git = "https://github.com/goniszewski/OpenStreamHub" }
```

## Documentation and examples

### Guides

| Guide | Covers |
| --- | --- |
| [macOS roadmap](docs/masterhub-macos-plan.md) | Target modules, Windows capture procedure, and Raspberry Pi USB limitations |
| [Diagnostics](docs/masterhub-diagnostics.md) | Device inventory, raw recording, dial command previews, static images, JPEG packet plans, and offline replay |
| [Binding service](docs/masterhub-service.md) | JSON profiles, offline action plans, explicit macOS actions, and the `masterhub-service` options |
| [Action catalogue](docs/masterhub-actions.md) | Every action, its limits and permissions, including `profile`, `timer` and `obs_scene` |
| [Dial functions](docs/masterhub-dial-functions.md) | Short click cycles the dial's functions; long press and rotation run the current one |
| [Display sync](docs/masterhub-display-sync.md) | Experimental, hardware-unverified key images and dial faces |
| [macOS smoke checks](docs/masterhub-macos-smoke.md) | Device-free audio and scratch-keystroke checks |
| [Keypad folders](docs/masterhub-keypad-folders.md) | Nested folders within one profile, with a Back key and level badge |
| [Windows parity tracker](docs/windows-parity-gaps.md) | Gaps against the official Windows application |
| [First hardware session](docs/masterhub-first-hardware-session.md) | Checklist for the first session with physical modules |
| [Release process](docs/release-process.md) | Signing, notarization and release packaging |
| [Focused-app rules](docs/masterhub-focus-rules.md) | Rules for choosing a profile by the focused application |
| [Importing MasterHUB profiles](docs/importing-masterhub-profiles.md) | Importing official `.mhz` files, the report, and the dry-run-only rule |
| [Session manager](docs/masterhub-session-manager.md) | Sleep/wake handling, reconnect rules and the Execute safety rule |
| [Diagnostic bundle](docs/masterhub-diagnostic-bundle.md) | What the local diagnostic file collects, redaction and what is never collected |
| [Building the desktop app](docs/building-the-desktop-app.md) | Sidecar bundling, environment override and signing caveat |
| [Desktop app](apps/open-stream-hub/README.md) | App setup and development |
| [Implementation status](docs/masterhub-implementation-status.md) | Capability status and remaining validation |
| [Protocol evidence](docs/masterhub-protocol-evidence.md) | Evidence supporting protocol behavior |

### Example profiles

- [macOS profile](examples/profiles/macos.json): knob and dial captures.
- [All-module profile](examples/profiles/all-modules.json): also covers keys, faders, and rollers.
- [OBS scenes profile](examples/profiles/obs-scenes.json): knob presses that switch OBS scenes, alone and in a multi-action.

## Contributing

Contributions are welcome through pull requests.

- Follow Rust conventions and idioms, using [Rust Patterns & Engineering How-Tos](https://microsoft.github.io/RustTraining/rust-patterns-book/ch00-introduction.html) where appropriate.
- Add tests for new features and update relevant documentation and examples.
- Use [Conventional Commits](https://www.conventionalcommits.org/) for commit messages.
- Run formatting, linting, and tests before submitting.

### Rust checks

Run from the repository root:

```bash
cargo fmt --check
cargo clippy
cargo test
cargo test --doc
```

See the [desktop app guide](apps/open-stream-hub/README.md) for app-specific development instructions.

## Origins and attribution

OpenStreamHub is based on [CoolTEK SDK by projectzero7](https://github.com/projectzero7/cooltek-sdk).
Thank you to the original author and contributors for the foundation this project builds on.

This repository preserves the original Git history and MIT/Apache-2.0 licensing,
and develops its own roadmap for MasterHUB macOS support, diagnostics, bindings,
and the desktop app.

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT License](./LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 License, shall be dual licensed as above, without any additional terms or conditions.
