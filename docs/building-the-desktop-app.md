# Building the desktop app

The desktop app (`apps/open-stream-hub`) runs profile actions through `masterhub-service`, which it ships as a Tauri **sidecar** (`bundle.externalBin: ["binaries/masterhub-service"]`). Tauri requires the file `src-tauri/binaries/masterhub-service-<target-triple>` to exist whenever `src-tauri` is compiled.

`scripts/prepare-sidecar.mjs` creates it. It reads the host triple from `rustc -vV`, runs `cargo build -p cooltek-sdk-cli --bin masterhub-service --locked` from the repository root, and copies the result into `src-tauri/binaries/`. It skips the copy when the content is identical. Rust (`rustc` and `cargo`) must be on `PATH`; otherwise the script fails with a message saying so.

| Command | Sidecar step |
| --- | --- |
| `npm run desktop` (`tauri dev`) | `beforeDevCommand` runs `npm run sidecar` (debug build) |
| `npm run desktop:build` (`tauri build`) | `beforeBuildCommand` runs `npm run sidecar:release` (release build) |
| `npm run sidecar` / `npm run sidecar:release` | Run the step alone |

For a cross build, the Tauri CLI sets `TAURI_ENV_TARGET_TRIPLE`; the script uses it (or `--target <triple>`) and passes `--target` to cargo. `CARGO_TARGET_DIR` is honoured.

`cargo test` and `cargo clippy --all-targets` in `apps/open-stream-hub/src-tauri` also need the file. Run `npm run sidecar` once before them on a fresh checkout. The files in `src-tauri/binaries/` are git-ignored.

In the app bundle, the sidecar lands at `OpenStreamHub.app/Contents/MacOS/masterhub-service`. The app finds it there. To use a different build, set `OPEN_STREAM_HUB_SERVICE` to its path; the variable takes priority.

Signing and notarization: the sidecar is a second executable inside the bundle and must be signed with the app (Developer ID and the hardened runtime) before notarization. Tauri signs externalBin files when signing is configured. This has not been verified here; only unsigned local builds were made.

The bundle declares no entitlements, usage-description strings or other `Info.plist` additions beyond `LSMinimumSystemVersion` 12.3. Loopback OBS networking, timer notifications and the CPU dial face are described in [release process](release-process.md#permissions-and-entitlements); none of them is verified on a signed bundle.
