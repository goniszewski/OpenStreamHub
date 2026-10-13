# OpenStreamHub 0.1.0

First public build. An unofficial macOS app for Cooler Master MasterHUB modules, independent of and not endorsed by Cooler Master.

> [!IMPORTANT]
> This build is **not signed or notarized** (that needs a paid Apple Developer account). On first launch, use **System Settings → Privacy & Security → Open Anyway**. See [Install](../README.md#install). macOS asks for Accessibility, Automation and Input Monitoring permission the first time an action needs it.

## What it does

- **Profiles and bindings** for the keypad, knobs, dial and faders, with nested keypad folders, profile switching, undo/redo, import and export (own JSON format), and preset packs.
- **Run modes.** A dry run shows planned actions without running them; Execute (Live) runs them after you confirm. Switching mode or saving an edit applies to a running module without restarting it, except display runs (keypad and dial screens), which restart for a profile change.
- **Displays.** Key images and animations on the 15-key module; dial faces (clock, CPU usage, custom picture, function labels). A key artwork editor with small, medium and large (edge to edge) icons; artwork reaches the key when you press Set key artwork. Per-profile **screen brightness** for the dial and the key screens (the keys are dimmed in software).
- **Lighting.** A Lighting page for the base, knobs and faders, and screen backlight and colour for the dial, usable while a profile runs.
- **Actions.** Volume and mute, microphone volume, shortcuts, text snippets, app launch (choose an app and see its name), URLs, media controls, system actions, multi-action sequences, a key macro recorder, countdown timers with a notification and sound, and OBS Studio actions.
- **Startup.** An option to start at login, and a permission check on every launch that walks you to the right System Settings pane when something is missing. Settings → Permissions shows the same status any time.
- **Updates.** A Check for updates button in Settings, and an optional daily check on launch (off by default). It only tells you; you download and replace the app yourself.
- **Focused-app rules** that switch a running profile when an app comes to the front.
- **Official `.mhz` profile import** with a compatibility report and review before anything runs.
- **Diagnostics.** On-demand diagnostic bundles with sensitive fields redacted, raw capture and offline replay. No telemetry, accounts or automatic updates.

## Tested on hardware

One rig: 15-key module (M4315), 3-knob module (M003K), IPS dial (M131E), base (M0064) and the five-fader module (M005S) for fader input.

- 15-key: press and release, key artwork, folder redraws, press feedback, in-place mode and profile switching, screen brightness.
- Knobs: rotation and press. Dial: rotation, press, faces, custom picture, backlight and colour while running, reconnect after replug.
- Fader input and the microphone volume fader.
- The service stops when the app dies, so modules are not left in software mode.

## Known limits

- **Two-roller module (M032LE):** both rollers send input on Windows, but macOS receives no reports. Unresolved.
- **Display runs** still restart on a profile change or an early mode switch; the screen redraws.
- **Not yet checked on hardware:** the macro recorder, focused-app rules from the workspace, preset packs, the base layout canvas, Lighting for knobs and faders, timer countdown shown on the key, and Mac sleep/wake recovery.
- **OBS:** tested against a fake server only; not yet against a real OBS install.
- **Not implemented:** `.mhz` export, firmware updates, and Twitch, Discord and Streamlabs integrations (shortcuts can be assigned by hand).
- **Intel build:** built by cross-compiling on an Apple Silicon Mac, ad-hoc signed, and not launched on Intel hardware or under Rosetta.
- Interface language is English only.
- Unsigned builds are treated by macOS as a new app after each update, so permissions may need to be allowed again.

## Requirements

macOS 12.3 or later, Apple Silicon or Intel. Builds are per architecture; pick the matching `.dmg`.

## License

MIT or Apache-2.0, at your option. Built on the CoolTEK SDK by projectzero7.
