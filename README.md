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
| Profiles and bindings | JSON profiles, offline action plans, and explicit macOS volume, mute, shortcut, and app actions |
| Desktop editor | All-module catalog and binding editor, nested folders for the 15-key module, a live module rig preview, hot-plug module detection, profile import/export/duplicate, undo/redo, search and conflict warnings, offline display artwork preparation, and a local diagnostic bundle |
| Running actions | The desktop app can run a profile through a supervised `masterhub-service` process: dry run by default, explicit confirmation before Execute, clean stop on sleep, and reconnect after wake or unplug (Execute runs never resume without confirmation) |

Nothing is uploaded anywhere: profiles, diagnostic bundles and prepared images stay on your Mac.
Sending images to a module display is off by default, because display behavior is not verified on hardware. An experimental Settings toggle can turn it on for testing (placeholder key images and folder pages, and a dial function name or clock on the encoder's screen); it has never run on real hardware.
For detailed capability status, see the [implementation status](docs/masterhub-implementation-status.md)
and [protocol evidence table](docs/masterhub-protocol-evidence.md).

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
| [Binding service](docs/masterhub-service.md) | JSON profiles, offline action plans, and explicit macOS actions |
| [Keypad folders](docs/masterhub-keypad-folders.md) | Nested folders within one profile, with a Back key and level badge |
| [Windows parity tracker](docs/windows-parity-gaps.md) | Gaps against the official Windows application |
| [First hardware session](docs/masterhub-first-hardware-session.md) | Checklist for the first session with physical modules |
| [Release process](docs/release-process.md) | Signing, notarization and release packaging |
| [Focused-app rules](docs/masterhub-focus-rules.md) | Rules for choosing a profile by the focused application |
| [Session manager](docs/masterhub-session-manager.md) | Sleep/wake handling, reconnect rules and the Execute safety rule |
| [Diagnostic bundle](docs/masterhub-diagnostic-bundle.md) | What the local diagnostic file collects, redaction and what is never collected |
| [Building the desktop app](docs/building-the-desktop-app.md) | Sidecar bundling, environment override and signing caveat |
| [Desktop app](apps/open-stream-hub/README.md) | App setup and development |
| [Implementation status](docs/masterhub-implementation-status.md) | Capability status and remaining validation |
| [Protocol evidence](docs/masterhub-protocol-evidence.md) | Evidence supporting protocol behavior |

### Example profiles

- [macOS profile](examples/profiles/macos.json): knob and dial captures.
- [All-module profile](examples/profiles/all-modules.json): also covers keys, faders, and rollers.

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
