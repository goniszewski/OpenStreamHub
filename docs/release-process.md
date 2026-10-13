# Release process

This describes how the macOS desktop bundle is built in CI (`.github/workflows/release.yml`) and how to check signing and notarization. **Run so far (13 October 2026):** unsigned builds for both architectures through the release workflow, and local release builds of both (see below). **Not run:** signing, notarization, and a launch of the Intel app.

## Local builds (how the 0.1.0 release was made)

CI was blocked by GitHub billing, so 0.1.0 was built on an Apple Silicon Mac and attached to a draft release by hand.

- **Apple Silicon:** `cd apps/open-stream-hub && npx tauri build --bundles app,dmg`. `bundle_dmg.sh` is flaky; a retry works. Unmount stale `/Volumes/OpenStreamHub*` first.
- **Intel:** Homebrew Rust ships only the arm64 standard library, so install `rustup` (brew, keg-only), `rustup target add x86_64-apple-darwin`, then `PATH=/opt/homebrew/opt/rustup/bin:$PATH APPLE_SIGNING_IDENTITY="-" npx tauri build --bundles app,dmg --target x86_64-apple-darwin`. Without `APPLE_SIGNING_IDENTITY="-"` the cross-built binaries are not signed at all, and macOS refuses them. With it they are ad-hoc signed (the arm64 build is ad-hoc signed by the linker).
- **Check:** `codesign --verify --deep --strict` on each app and its `masterhub-service` sidecar; write `SHA256SUMS.txt` for the four files.
- **Publish:** keep the files in `dist-release/` (gitignored), move the tag, and `gh release create v0.1.0 --draft --notes-file docs/release-notes-0.1.0.md <files>`. Publishing is left to the owner. While the repo is private the in-app update check cannot see a release.
- **Not tested:** the Intel app has not been launched on Intel hardware or under Rosetta.

## Workflows

| File | Trigger | Purpose |
| --- | --- | --- |
| `.github/workflows/ci.yml` | pull request, push to `master`, manual | `cargo fmt --check`, clippy (`-D warnings`) and tests for the root workspace and `src-tauri`; `tsc --noEmit` and `npm test` for the frontend. Push and PR runs use `macos-15` (arm64) only; a manual run adds `macos-15-intel`. |
| `.github/workflows/release.yml` | `workflow_dispatch`, tags `v*` | Builds `OpenStreamHub.app` per architecture, signs and notarizes it if secrets exist, uploads a zip and a DMG as one workflow artifact. |

Both use `permissions: contents: read`. The release workflow does not create a GitHub release; publish by hand after checking the artifact.

Intel runner label: `macos-13` has been retired by GitHub, so `macos-15-intel` is used (it ran successfully on 13 October 2026). The Intel job takes about 6 minutes with a warm cache and about 16 without.

## Per-architecture, not universal

The release builds one app per architecture on native runners. A universal build is not used because `scripts/prepare-sidecar.mjs` builds a single `masterhub-service-<triple>` file, while Tauri would need `masterhub-service-universal-apple-darwin` for `--target universal-apple-darwin`. Supporting universal means extending that script to build both triples and `lipo` them. Each artifact holds the `.zip` and the `.dmg`, and is named `OpenStreamHub-<aarch64|x86_64>-signed` or `-UNSIGNED`.

## Prerequisites

- Apple Developer Program membership and a **Developer ID Application** certificate (not "Apple Development").
- Rust >= 1.97 (`rust-toolchain.toml` tracks `stable`), Node >= 22.12.
- Export the certificate with its private key as a `.p12` from Keychain Access, then base64 it: `base64 -i cert.p12 | pbcopy`.
- An app-specific password for the Apple ID (appleid.apple.com), used for notarization.

## Secrets

Set these as repository (or environment) secrets. Names follow Tauri 2's macOS signing docs; re-check against the current docs before first use.

| Secret | Content |
| --- | --- |
| `APPLE_CERTIFICATE` | base64 of the `.p12` |
| `APPLE_CERTIFICATE_PASSWORD` | password set when exporting the `.p12` |
| `APPLE_SIGNING_IDENTITY` | e.g. `Developer ID Application: Name (TEAMID)` |
| `APPLE_ID` | Apple ID email used for notarization |
| `APPLE_PASSWORD` | app-specific password (not the account password) |
| `APPLE_TEAM_ID` | 10-character team ID |

Tauri also supports App Store Connect API keys for notarization (`APPLE_API_ISSUER`, `APPLE_API_KEY`, `APPLE_API_KEY_PATH`). The workflow does not use them.

The workflow exports these to the job environment only if **all six** are non-empty. If any is missing it emits a warning, builds without signing, and names the artifact `...-UNSIGNED`. Secrets are not available to workflows triggered from fork PRs, which is why signing lives only in the release workflow.

## Running a release

1. Bump the version in `apps/open-stream-hub/package.json`, `src-tauri/Cargo.toml` and `src-tauri/tauri.conf.json` (they are kept in sync by hand; the app is currently `0.1.0`). Commit.
2. Tag `vX.Y.Z` and push the tag, or start the workflow from the Actions tab.
3. Write `docs/release-notes-X.Y.Z.md` and use it as the release notes. The README needs no edit: its release badge and Install link follow the latest published release.
4. Download the artifacts, verify them locally (below), and attach them to a release manually.

## Sidecar signing

`masterhub-service` is bundled via `bundle.externalBin` and lands at `OpenStreamHub.app/Contents/MacOS/masterhub-service`. It is a second executable and must be signed with Developer ID and the hardened runtime before notarization, or notarization rejects the app. Tauri's bundler signs `externalBin` files itself when signing is configured: it signs sidecars first and the `.app` last, with the hardened runtime on by default (per Tauri's bundler source and the [macOS signing guide](https://v2.tauri.app/distribute/sign/macos/); not yet exercised in this repository). The workflow still runs `codesign --verify` on the sidecar explicitly in the signed path and fails if it is unsigned. If it fails, sign it manually before bundling (`codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" src-tauri/binaries/masterhub-service-<triple>`) or add a signing step after `prepare-sidecar`. Also confirm the hardened runtime flag (`flags=0x10000(runtime)` in `codesign -dv`). If the app later needs entitlements, they must be configured in `tauri.conf.json` (`bundle.macOS.entitlements`); none are set today.

## Permissions and entitlements

`src-tauri/Info.plist` (merged into the bundle's `Info.plist` by Tauri) adds `NSAppleEventsUsageDescription`, and `src-tauri/Entitlements.plist` (set as `bundle.macOS.entitlements`, applied only when signing) grants `com.apple.security.automation.apple-events`. Nothing else is declared and the app is not sandboxed. No JIT or library-validation entitlements are needed: there is no JIT, and every dylib is a system one. What the bundled code does that macOS may gate:

- **Local networking.** `obs_scene` opens a plain TCP connection from the `masterhub-service` sidecar to a loopback address (`TcpStream` in `crates/cooltek-sdk-service/src/obs.rs`). The webview's CSP does not apply to it. An unsandboxed app needs no network entitlement for this; a future App Sandbox build would need `com.apple.security.network.client`. Not verified on a signed build.
- **Notifications and sounds.** A timer runs `/usr/bin/osascript` (`display notification`) and `/usr/bin/afplay`. Whether macOS shows the notification, and for which application it asks, is unverified on a real Mac; the service logs `timer_alert_warning` when it is refused.
- **Accessibility and Automation.** Shortcut, text and undo/redo actions drive System Events through `osascript`. Both the Automation entitlement and the usage string are declared (hardened-runtime apps need them for the prompt to appear), but whether the prompt names OpenStreamHub rather than osascript has not been checked on a signed build.
- **Input Monitoring.** Reading HID input may need it for the launching application (see the [first hardware session](masterhub-first-hardware-session.md), step 8).
- **Key recorder and posting keys.** The recorder's event tap needs Input Monitoring for OpenStreamHub itself, and posting keys or the media key needs Accessibility. Neither has a usage string; macOS asks on first use.
- **Updates and grants.** An ad-hoc build gets a new identity every build, so macOS forgets the Accessibility, Input Monitoring and Automation grants on each update. Only a stable Developer ID signature keeps them.
- **CPU usage.** The `cpu` dial face calls the Mach `host_processor_info` function in-process, which needs no permission or entitlement; it is drawn only when the dial screen is set to CPU usage (Settings > Module control, App).

Record the outcome of each in the first signed run and update this list.


## Local verification commands

Run against a downloaded and unzipped `OpenStreamHub.app`:

```sh
# Signature, including the nested sidecar
codesign --verify --deep --strict --verbose=2 OpenStreamHub.app
codesign --verify --strict --verbose=2 OpenStreamHub.app/Contents/MacOS/masterhub-service
codesign -dv --verbose=4 OpenStreamHub.app 2>&1 | grep -E "Authority|TeamIdentifier|flags"

# Gatekeeper assessment (passes only once notarized and stapled, or locally trusted)
spctl --assess --type execute --verbose=4 OpenStreamHub.app

# Notarization ticket
xcrun stapler validate OpenStreamHub.app

# Notarization history / failure logs
xcrun notarytool history --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD"
xcrun notarytool log <submission-id> --apple-id "$APPLE_ID" --team-id "$APPLE_TEAM_ID" --password "$APPLE_PASSWORD"
```

Zips downloaded from Actions carry a quarantine attribute, which is what Gatekeeper checks; use `ditto -x -k` to unzip and keep the signature intact.

## Untested

- No workflow has run in GitHub Actions; runner labels, cache settings and action versions (`actions/*@v4`, `Swatinem/rust-cache@v2`) are unconfirmed.
- Signing, notarization, stapling and the `spctl` check have never been exercised.
- Tauri's documented sidecar signing and hardened-runtime behaviour has not been exercised here; the workflow's explicit `codesign` check will show it on the first signed run.
- The Intel build has not run; `macos-15-intel` is the last Intel GitHub-hosted image, so watch GitHub's runner retirement notices. `macos-14` was avoided because GitHub retires it on 2026-11-02.
- `cargo test` for `src-tauri` on a headless runner is assumed to work.
- Stapling is not done separately; Tauri is expected to staple after notarizing. The workflow does not fail if `stapler validate` fails.
