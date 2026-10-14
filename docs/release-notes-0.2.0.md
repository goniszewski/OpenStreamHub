# OpenStreamHub 0.2.0

Menu bar mode, modules that stand up or turn on the base with their pictures turned to match, and one profile for all your modules. An unofficial macOS app for Cooler Master MasterHUB modules, independent of and not endorsed by Cooler Master.

> [!IMPORTANT]
> This build is **not signed or notarized**, like 0.1.0. On first launch, use **System Settings → Privacy & Security → Open Anyway**. See [Install](../README.md#install). Unsigned builds count as a new app after each update, so macOS may ask for Accessibility, Automation and Input Monitoring again.

## What is new

- **Menu bar mode** (on by default, Settings → Startup). Closing the window hides it and your modules keep running. The menu bar icon, the logo's square eye (also now the logo in the app header), offers Show OpenStreamHub, Workspace, Lighting, Demos, Settings, Check for new version and Quit. Quit still asks about unsaved changes. Turn the setting off to quit when the window closes.
- **Modules placed by where they are plugged in.** Each module sits on the base the way its connector turns it: the base layout stands it up or turns its picture, so the preview matches the real arrangement. Checked on the real base with several arrangements of the dial, knobs, faders and the 15-key module. About 80 of the connector entries have not been seen on a unit.
- **Pictures on a turned module read upright.** The key pictures and the dial face are turned the other way before they are sent, so a module standing on its side shows upright pictures, and the preview draws them the same way. Checked on the 15-key module at 90° and 270°. A module turned 180° does not connect to the base at all, so only 0°, 90° and 270° are real positions. A turn applies when a module's run next starts (Stop and Start, or a replug).
- **One profile for all modules.** A "Running profile" choice in the sidebar puts a profile on every running module and makes it the one modules start with; the header's profile picker is linked to it. The base layout names the profile each module runs under its picture. The result is shown as a notice.
- **Built-in profiles in the pickers.** Mac basics, Streaming (OBS) and Editing and media are listed in the profile pickers until you hide them in Settings → Built-in profiles. Choosing one saves it once as a profile of your own, and choosing it again opens that profile instead of making a numbered copy.
- **Icons on keys without artwork.** An action tile you have not given a picture now shows an icon for what it does over its label on the module (volume, mute, playback, timer, screenshot, undo, OBS and more), matching the preview. Your own artwork is untouched.
- **The profile you had open comes back** after a refresh or restart, instead of the first one.

## Fixes

- A module moved to another connector, and a module replugged, now start with the turn of the connector they are on now.
- Choosing a starter you already have no longer saves a "… 2" copy.

## Known limits

- **Not checked on hardware:** key order, roller direction and fader range on a turned module; the dial's picture on a turned module; the two-roller module on the base at all.
- **Two-roller module (M032LE):** macOS still receives no reports from its rollers. Unresolved.
- A change of a module's turn does not reach a running module until it is started again.
- The base's own lighting is stored apart from profiles, so a profile cannot be "run" on the base.
- **OBS:** tested against a fake server only.
- **Intel build:** cross-compiled on an Apple Silicon Mac, ad-hoc signed, and not launched on Intel hardware or under Rosetta.
- Interface language is English only.

## Requirements

macOS 12.3 or later, Apple Silicon or Intel. Builds are per architecture; pick the matching `.dmg`.

## License

MIT or Apache-2.0, at your option. Built on the CoolTEK SDK by projectzero7.
