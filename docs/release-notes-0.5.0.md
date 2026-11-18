# OpenStreamHub 0.5.0

Plugins: add your own actions, if / else tests and pictures on keys and the dial, and install them from Settings. Two official plugins: Now Playing puts Apple Music or Spotify on one key with the cover as its picture, and Claude Code shows your Claude Code sessions on keys. A new demo, Twofold, is a sliding number game on all 15 keys. An unofficial macOS app for Cooler Master MasterHUB modules, independent of and not endorsed by Cooler Master.

> [!IMPORTANT]
> This build is **not signed or notarized**, like earlier releases. On first launch, use **System Settings → Privacy & Security → Open Anyway**. See [Install](https://github.com/goniszewski/OpenStreamHub#install). Unsigned builds count as a new app after each update, so macOS may ask for Accessibility, Automation and Input Monitoring again.

## What is new

- **Plugins.** A plugin is a separate program, in any language, that the app starts while a Live profile uses it and talks to over a local socket. It can:
  - add **actions** a key runs with the new **Plugin** action, with settings of its own (text, numbers, switches, choices) that may fill in variables;
  - hear the **rest of the input** on the controls bound to it: a key's release, a knob's turn direction, so it can hold a key (push to talk) or tell a long press from a short one;
  - add **conditions** an if / else asks it when the key is pressed ("the light is on");
  - **draw the M4315 keys** that run its actions, and the **M131E dial screen** while a dial function that runs one is current, and learn which of them are on screen.
- **Installing plugins.** **Settings → Plugins → Install…** picks a plugin's folder and shows what it declares before anything is copied. Installing again updates it; **Remove…** deletes it along with its approval, its secrets and its data. Plugins are grouped into Enabled and Disabled, one card each, with their log.
- **Nothing runs until you enable it.** **Enable…** lists the program a plugin starts, its actions, conditions, permissions and secrets, and approves exactly that `plugin.json`: any change to it needs a new review. Secrets such as access tokens are kept in the macOS Keychain, never in a profile. A profile that uses a plugin is rated high risk in the Execute confirmation, and a JSON import holding one is dry run only until allowed.
- **Now Playing, the first official plugin.** One key for Apple Music or Spotify: a short press plays or pauses, a long press goes to the previous track and a double press to the next. The key shows the cover, dimmed with a pause sign while paused (so does the dial, on a dial function bound to it), and an if / else can test whether music is playing. Bind it to a key's own Press, with nothing on its Release. Install it from the `plugins/com.github.goniszewski.nowplaying` folder of the source code ([how](https://github.com/goniszewski/OpenStreamHub/tree/master/plugins/com.github.goniszewski.nowplaying)).
- **Claude Code, the second official plugin.** Keys that show your [Claude Code](https://code.claude.com) sessions: which one works, waits for you (with the tool's name) or is done, and how full its context window is and what it has cost, also as a large ring on the dial. A press brings that session's terminal to the front, a long press shows the next session. With **Answer permission prompts** on, once the terminal is in front and you can read the prompt there, a press answers Yes, once, and a long press declines; it types nothing otherwise (iTerm2 and Terminal). Claude Code's hooks feed it: `install_hooks.py` adds them ([how](https://github.com/goniszewski/OpenStreamHub/tree/master/plugins/com.github.goniszewski.claudecode)).
- **Twofold, a new demo.** A sliding number game on the 15 keys (**Demos** page): press a key on an edge to slide the tiles toward it, merge equal ones, reach 2048 and keep going. The centre key shows the score and, pressed again, starts a new game. A connected roller or knob module makes moves too, and so do the Mac's arrow keys; the best score is kept.
- **Writing a plugin.** The [author guide](https://github.com/goniszewski/OpenStreamHub/blob/master/docs/plugins.md) covers the manifest (schema 1, stable from this release), the protocol and the limits. A small [example plugin](https://github.com/goniszewski/OpenStreamHub/tree/master/examples/plugins/com.example.notify) in Python, using only the standard library, shows every part.
- **Run program.** A new action starts a program directly, without a shell: its path, arguments (which may fill in variables) and start folder. It has 15 seconds; if the run stops or the time passes while it runs, it is stopped with the processes it started in its process group. Other presses on that module wait while it runs.
- **Editor.** The gesture picker is an icon grid that marks the gestures in use, unused keys and controls are muted, and built-in key tiles are drawn on black.
- **Preset packs folder.** Preset packs dropped into the `packs` folder in the app data folder are listed in the starter chooser beside the bundled ones, checked by the same rules. **Open packs folder** opens it.

## Fixes

- A module on any of the base's 27 connectors is now placed on the base layout. Before, some connectors showed "Position not detected" for some modules, and on one connector each, the roller and the dial were drawn about two cells away from it.
- On the two-column layout the workspace and the inspector scroll on their own, so scrolling one no longer moves the other.
- The Settings section that shows the current run is called **Run diagnostics** and is listed in the Settings contents.

## Checked on hardware

On 17 November 2026, on a real M4315, M131E and M003K: Claude Code's Session status key, its Context and cost dial face and answering real permission prompts from a key; Now Playing's cover on the dial; and the Twofold demo on the keys and with the knobs. Plugins were also tested with a fake app socket and fakes in the app's tests, the example plugin run for real by the system Python, and Now Playing against the real Music app (state and covers).

## Known limits

- **Now Playing** is not yet checked on a key of a real M4315 or with Spotify, **Claude Code** not yet in Terminal, and the Twofold demo not yet with a roller module.
- **A plugin runs as you.** It can do anything you can, including using the network; the permissions it lists are shown, not enforced, apart from the variables it may set. Install plugins only from authors you trust.
- **Plugins** run only while a Live profile uses them, need their folder unzipped before installing (a `.zip` is not taken), and have no registry or update checks. A plugin draws on the dial only while the profile's dial screen is not set to Off.
- **Intel build:** cross-compiled on an Apple Silicon Mac, ad-hoc signed, and not launched on Intel hardware or under Rosetta.
- Interface language is English only.

## Requirements

macOS 12.3 or later, Apple Silicon or Intel. Builds are per architecture; pick the matching `.dmg`. Plugins written in Python, such as Now Playing and Claude Code, use `/usr/bin/python3`, which comes with the Xcode command-line tools.
