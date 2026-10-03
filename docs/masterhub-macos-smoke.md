# Controlled macOS action checks

These examples exercise the service's native dispatcher without a MasterHUB or HID context. Run them from the repository root. Execution is explicit; normal workspace tests use mock processes and do not change host state.

## Audio

Read the current system output volume and mute state:

```sh
cargo run -p cooltek-sdk-service --example macos_audio_smoke
```

Run the controlled check:

```sh
cargo run -p cooltek-sdk-service --example macos_audio_smoke -- --execute
```

The check snapshots the original state, requests a five-percentage-point volume change (subtracting when the original exceeds 95), verifies a fresh query, toggles mute, verifies again, and restores both original fields. Volume verification allows at most one point of target deviation while requiring actual movement in the requested direction and unchanged mute. Mute verification requires the volume to stay at its observed value. Restoration is attempted after ordinary action/query failures and verified with another query. Failed volume verification stops the check before mute. An unchanged output, wider deviation, wrong direction, concurrent changes, or failed restoration produces a failing exit status. System-tool success alone is insufficient evidence of an effect.

Each query/setter uses the backend's bounded subprocess runner. `AudioSnapshot`, `audio_snapshot()`, and `restore_audio_snapshot()` are also public helpers for callers. Restore attempts volume and mute independently and combines failures; a cancelled dispatcher remains cancelled. Forced process termination cannot guarantee restoration. Avoid concurrent audio changes during the check.

## Application launch and scratch keystroke

Preview either fixed action without launching a system tool:

```sh
cargo run -p cooltek-sdk-service --example macos_action_smoke -- launch-textedit
cargo run -p cooltek-sdk-service --example macos_action_smoke -- type-marker
```

Request TextEdit through the native bundle-ID action:

```sh
cargo run -p cooltek-sdk-service --example macos_action_smoke -- --execute launch-textedit
```

Create a disposable document with a known marker and put its caret at the end. Start this command, then bring that document to the foreground during the five-second delay:

```sh
cargo run -p cooltek-sdk-service --example macos_action_smoke -- --execute type-marker
```

The action sends one plain `m` through System Events. Verify that exactly one `m` appears at the caret. It uses no clipboard commands. `action_completed` means the system tool returned successfully; it does not verify application focus or resulting text. Permission failures retain the native diagnostic. This example does not grant Accessibility or Automation permissions, focus applications automatically, or verify modified shortcuts such as Command-C/Command-V.

## Observations on 3 October 2026

On the Apple Silicon test host, querying system audio returned volume 75 and mute false. A one-point volume request returned successfully, but a fresh query remained at 75. A five-point request reported 79 rather than the requested 80. Both initial checks rejected their exact target mismatch and verified restoration. These observations do not establish the host's rounding or quantization mechanism.

The final check with the one-point verification tolerance passed: volume 75/unmuted became 79/unmuted, mute became true while volume stayed 79, then fresh queries verified restoration to 75/unmuted. Volume/mute effects are verified for this host and output configuration; other outputs, volume edges, and packaged-app execution still need runtime testing. The production dispatcher was not changed to compensate for the reported one-point deviation.

The TextEdit launch request returned successfully and a TextEdit window was observed. Plain-key dispatch also returned successfully, including a delayed attempt with the scratch document's caret selected, but the observed scratch text did not change. Keyboard effects and focus behavior remain unverified. No permission settings were changed. Hardware input, modified shortcuts, and action execution from a packaged application remain separate acceptance gates.
