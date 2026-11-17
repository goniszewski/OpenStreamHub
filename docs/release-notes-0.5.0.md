# OpenStreamHub 0.5.0

Plugins: add your own actions, if / else tests and key pictures, and install them from Settings. Two official plugins: Now Playing puts Apple Music or Spotify on one key with the cover as its picture, and Claude Code shows your Claude Code sessions on keys. An unofficial macOS app for Cooler Master MasterHUB modules, independent of and not endorsed by Cooler Master.

> [!IMPORTANT]
> This build is **not signed or notarized**, like earlier releases. On first launch, use **System Settings → Privacy & Security → Open Anyway**. See [Install](https://github.com/goniszewski/OpenStreamHub#install). Unsigned builds count as a new app after each update, so macOS may ask for Accessibility, Automation and Input Monitoring again.

## What is new

- **Plugins.** A plugin is a separate program, in any language, that the app starts while a Live profile uses it and talks to over a local socket. It can:
  - add **actions** a key runs with the new **Plugin** action, with settings of its own (text, numbers, switches, choices) that may fill in variables;
  - hear the **rest of the input** on the controls bound to it: a key's release, a knob's turn direction, so it can hold a key (push to talk) or tell a long press from a short one;
  - add **conditions** an if / else asks it when the key is pressed ("the light is on");
  - **draw the M4315 keys** that run its actions, and learn which of them are on screen.
- **Installing plugins.** **Settings → Plugins → Install…** picks a plugin's folder and shows what it declares before anything is copied. Installing again updates it; **Remove…** deletes it along with its approval, its secrets and its data. Plugins are grouped into Enabled and Disabled, one card each, with their log.
- **Nothing runs until you enable it.** **Enable…** lists the program a plugin starts, its actions, conditions, permissions and secrets, and approves exactly that `plugin.json`: any change to it needs a new review. Secrets such as access tokens are kept in the macOS Keychain, never in a profile. A profile that uses a plugin is rated high risk in the Execute confirmation, and a JSON import holding one is dry run only until allowed.
- **Now Playing, the first official plugin.** One key for Apple Music or Spotify: a short press plays or pauses, a long press goes to the previous track and a double press to the next. The key shows the cover, dimmed with a pause sign while paused, and an if / else can test whether music is playing. Bind it to a key's own Press, with nothing on its Release. Install it from the `plugins/com.github.goniszewski.nowplaying` folder of the source code ([how](https://github.com/goniszewski/OpenStreamHub/tree/master/plugins/com.github.goniszewski.nowplaying)).
- **Claude Code, the second official plugin.** Keys that show your [Claude Code](https://code.claude.com) sessions: which one works, waits for you (with the tool's name) or is done, and how full its context window is and what it has cost. A press brings that session's terminal to the front, a long press shows the next session. With **Answer permission prompts** on, once the terminal is in front and you can read the prompt there, a press answers Yes, once, and a long press declines; it types nothing otherwise (iTerm2 and Terminal). Claude Code's hooks feed it: `install_hooks.py` adds them ([how](https://github.com/goniszewski/OpenStreamHub/tree/master/plugins/com.github.goniszewski.claudecode)).
- **Writing a plugin.** The [author guide](https://github.com/goniszewski/OpenStreamHub/blob/master/docs/plugins.md) covers the manifest (schema 1, stable from this release), the protocol and the limits. A small [example plugin](https://github.com/goniszewski/OpenStreamHub/tree/master/examples/plugins/com.example.notify) in Python, using only the standard library, shows every part.
- **Run program.** A new action starts a program directly, without a shell: its path, arguments (which may fill in variables) and start folder. It has 15 seconds; if the run stops or the time passes while it runs, it is stopped with the processes it started in its process group. Other presses on that module wait while it runs.
- **Editor.** The gesture picker is an icon grid that marks the gestures in use, unused keys and controls are muted, and built-in key tiles are drawn on black.
- **Preset packs folder.** Preset packs dropped into the `packs` folder in the app data folder are listed in the starter chooser beside the bundled ones, checked by the same rules. **Open packs folder** opens it.

## Fixes

- On the two-column layout the workspace and the inspector scroll on their own, so scrolling one no longer moves the other.
- The Settings section that shows the current run is called **Run diagnostics** and is listed in the Settings contents.

## Checked on hardware

Nothing in this release was newly checked on hardware. Plugins were tested with a fake app socket and fakes in the app's tests, the example plugin run for real by the system Python, Now Playing against the real Music app by hand on 17 November 2026 (state and covers), and Claude Code with a real Claude Code session in iTerm2 the same day, answering real permission prompts from a key.

## Known limits

- **Plugin key images** are not yet checked on a real M4315, **Now Playing** not yet with Spotify, and **Claude Code** not yet in Terminal.
- **A plugin runs as you.** It can do anything you can, including using the network; the permissions it lists are shown, not enforced, apart from the variables it may set. Install plugins only from authors you trust.
- **Plugins** run only while a Live profile uses them, need their folder unzipped before installing (a `.zip` is not taken), and have no registry or update checks. Pictures are drawn only on M4315 keys, not on the dial.
- **Intel build:** cross-compiled on an Apple Silicon Mac, ad-hoc signed, and not launched on Intel hardware or under Rosetta.
- Interface language is English only.

## Requirements

macOS 12.3 or later, Apple Silicon or Intel. Builds are per architecture; pick the matching `.dmg`. Plugins written in Python, such as Now Playing and Claude Code, use `/usr/bin/python3`, which comes with the Xcode command-line tools.
