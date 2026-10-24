# OpenStreamHub 0.3.0

Conditional keys (if / else, AND / OR, nesting, variables), GIFs on the dial, and a full set of OBS actions. An unofficial macOS app for Cooler Master MasterHUB modules, independent of and not endorsed by Cooler Master.

> [!IMPORTANT]
> This build is **not signed or notarized**, like earlier releases. On first launch, use **System Settings → Privacy & Security → Open Anyway**. See [Install](../README.md#install). Unsigned builds count as a new app after each update, so macOS may ask for Accessibility, Automation and Input Monitoring again.

## What is new

- **Conditional keys.** A new if / else action runs one list of steps or another, decided when you press the key, so it works with the app closed. Conditions can read OBS state (streaming, recording, virtual camera, replay buffer, scene, input mute), output mute and volume, whether a timer is running, and which app is in front. Combine them with AND / OR, nest groups and if / else blocks inside each other, and use your own variables. The editor supports drag and drop. See [conditional flows](masterhub-flows.md).
- **Profile schema 5.** Older profiles (schemas 1 to 4) are upgraded on open. A build older than 0.3.0 will refuse a profile that uses these features.
- **GIFs on the dial.** Import a GIF and it plays on the dial at up to 5 frames a second. The picker prepares Sharp, Balanced and Fast versions to compare, and shows which one is in use. The dial previews in the app play the GIF too. Small, simple GIFs play faster.
- **OBS actions.** Stream and record (start, stop, toggle), screenshot into Pictures, input mute, volume step and media source control. A press-twice guard protects the stream key, and a Streamer starter profile is included. OBS input volume follows OBS's own slider scale, so a fader tracks the slider. The OBS password is kept in the macOS Keychain.
- **Better artwork handling.** JPEG EXIF orientation is applied, BMP sources are accepted, key previews are drawn in the module's own 5x7 font exactly as on the keys, and import errors are explained in plain language.
- **All control modules start by themselves** (faders, rollers).
- **Profile keys open the profile in the editor** unless that would replace unsaved changes.

## Fixes

- Record macro no longer crashes the app, and Use recording closes the dialog.
- The dial picker no longer sticks on "Preparing" for an invalid background colour, and unsupported file types are named.
- Dial animation frames stay on time, retry after a failed frame, and a one-frame GIF is shown as a still.
- The key artwork editor says so when a picture still being prepared was discarded.
- Importing, duplicating or starting a profile says it is open in the editor and not yet running on the modules.

## Checked on hardware

Keys, the profile key, the timer countdown, media keys and the recorder on a real M4315; dial GIFs and function cycling on a real M131E; stream, record, scene, filter and source actions plus failure paths on a real OBS.

## Known limits

- **Conditional flows** are tested with fakes, unit and render tests, not on hardware. The dial function condition is not built.
- **Two-roller module (M032LE):** macOS still receives no reports from its rollers. Unresolved.
- A change of a module's turn does not reach a running module until it is started again.
- **Intel build:** cross-compiled on an Apple Silicon Mac, ad-hoc signed, and not launched on Intel hardware or under Rosetta.
- Interface language is English only.

## Requirements

macOS 12.3 or later, Apple Silicon or Intel. Builds are per architecture; pick the matching `.dmg`.

## License

MIT or Apache-2.0, at your option. Built on the CoolTEK SDK by projectzero7.
