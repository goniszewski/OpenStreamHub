# CoolTEK SDK

[![License](https://img.shields.io/badge/license-MIT%2FApache-blue.svg)](https://github.com/projectzero7/cooltek-sdk#License)
[![Conventional Commits](https://img.shields.io/badge/Conventional%20Commits-1.0.0-%23FE5196?logo=conventionalcommits&logoColor=white)](https://conventionalcommits.org)

An unofficial SDK for [Cooler Master](https://www.coolermaster.com/) products.

## MasterHUB macOS development

The [macOS roadmap](docs/masterhub-macos-plan.md) covers target modules,
Windows capture procedure, and Raspberry Pi USB limitations. The [diagnostic guide](docs/masterhub-diagnostics.md)
describes device inventory, bounded raw recording, dial command previews, JPEG packet plans,
and offline replay.

The [binding service guide](docs/masterhub-service.md) covers JSON profiles,
offline action plans, and explicit macOS volume, mute, shortcut, and app actions.
The [sample profile](examples/profiles/macos.json) works with both knob and dial captures.

The SDK builds on macOS and recognizes both USB ID generations for these modules. It includes
dial input decoding, explicit feature controls and firmware queries, descriptor-sized JPEG packets, and session
lifecycle hooks. Tests use synthetic reports and fake transports. Physical input, display acceptance,
and sleep/wake recovery still require hardware validation. See the
[implementation status](docs/masterhub-implementation-status.md) and
[protocol evidence table](docs/masterhub-protocol-evidence.md) for capability status.

## Supported Integrations

| Product | Status | Feature |
| --- | --- | --- |
| [MasterHUB](https://www.coolermaster.com/products/masterhub.html) | partial | `masterhub` |

## Getting Started

Add the following to your `Cargo.toml` file:

```toml, no_run
[dependencies]
cooltek-sdk = { git = "https://github.com/projectzero7/cooltek-sdk" }
```

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

### Guidelines

* Follow Rust conventions and idioms

* Follow [Rust Patterns & Engineering How-Tos](https://microsoft.github.io/RustTraining/rust-patterns-book/ch00-introduction.html) where appropriate

* Add tests for new features

* Update documentation and examples where appropriate

* Use [Conventional Commits](https://www.conventionalcommits.org/) for commit messages.

* Run `cargo fmt` and `cargo clippy` before submitting

### Development Setup

1. Clone the repository:

    ```bash, no_run
    git 'clone' 'https://github.com/projectzero7/cooltek-sdk'
    ```

1. Navigate to the repository root directory:

    ```bash, no_run
    cd 'cooltek-sdk'
    ```

1. Install dependencies and build:

    ```bash, no_run
    cargo build
    ```

### Development Actions

* Run tests:

    ```bash, no_run
    cargo test
    ```

* Run doctests:

    ```bash, no_run
    cargo test --doc
    ```

## License

Licensed under either of [Apache License, Version 2.0](./LICENSE-APACHE) or [MIT License](./LICENSE-MIT) at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, as defined in the Apache-2.0 License, shall be dual licensed as above, without any additional terms or conditions.
