# Now Playing

An OpenStreamHub plugin for Apple Music and Spotify. One M4315 key shows the current track's cover and drives the player:

| On a key | Does |
| --- | --- |
| Short press | Play / pause |
| Long press (held 500 ms) | Previous track, as soon as the time is reached |
| Double press (second within 300 ms) | Next track, at the second press |

The key shows the cover while playing, the cover dimmed with a pause sign while paused, and a dim play sign when nothing is playing or the player is not running. Without a cover it shows three bars while playing.

**Bind it to the key's own Press, and leave its Release unbound.** Short, long and double presses are told apart only on a key the app draws for this action, since only such a key is sure to send its release. If the key's Release runs something else, its first press goes back a track (no release comes, so it looks held) and later presses play or pause at once. Anywhere else a press plays or pauses at once: as a step of a multi-action or an if / else, while Settings > Let OpenStreamHub drive the display is **Built-in animation**, or on a knob's press.

The action also works on other controls: turning a knob or the dial right (a roller down) goes to the next track and left (a roller up) to the previous. Without dial functions, a dial click plays or pauses, a double click goes next, and a long click goes back. A key draws its picture on the M4315, and the dial on its screen while a dial function bound to this action is current (the whole round screen, at 240×240): bind it to the function's turn, for example, so turning skips tracks while the dial shows the cover. A knob or roller shows nothing.

The condition **Music is playing** lets an if / else test whether the chosen player, or either, is playing.

Requires OpenStreamHub 0.5.0 or later, macOS, and `/usr/bin/python3`, which macOS installs with the Xcode command-line tools (it offers to the first time something asks for it). Status: unit-tested with fakes; on 17 November 2026 it read the real Music app's state and cover and drew the key end to end through a fake app socket, by hand. Not yet checked on a real M4315 or M131E, or with Spotify.

## Install

1. In OpenStreamHub, open **Settings > Plugins** and choose **Install…**.
2. Pick this folder, `plugins/com.github.goniszewski.nowplaying`, in a copy of the repository (the release page's **Source code (zip)**, unzipped, has it). Review what it declares, then choose **Install and enable**.
3. Bind a key's **Press** to the **Plugin** action (Apps & web), with **Now Playing** as the plugin and **Play/pause, previous, next** as its action, and run the profile in **Live**, with Settings > Let OpenStreamHub drive the display set to **App** so the app drives the keys.

The first time it reads or drives a player, macOS asks whether OpenStreamHub may control Music (or Spotify). Choose **OK**. If you chose **Don't Allow**, the plugin's log in Settings > Plugins says so; allow it in System Settings > Privacy & Security > Automation.

## Settings on each key

| Setting | Default | |
| --- | --- | --- |
| Player | Whichever is playing | **Whichever is playing** picks the player that is playing, then one that is paused, then one that is running. Play/pause with neither running starts Apple Music. **Apple Music** or **Spotify** sticks to that one and starts it on play/pause. |
| Double press window (ms) | 300 | How long the plugin waits for a second press. A single press acts only once this has passed, so a shorter window makes play/pause quicker. 0 to 1000; the window starts at the first release. 0 turns double press off: play/pause then acts at the release. |
| Long press (ms) | 500 | How long a key is held to go back. 200 to 3000. |

## What it does on your Mac

- **Reads the player only while it is running**, with `pgrep` and then AppleScript (`music.applescript`, `spotify.applescript`), every 2 seconds while one of its keys is on screen, just after each press or turn, and when an if / else asks whether music is playing. It never starts a player except for play/pause.
- **Draws keys** with AppKit through `osascript -l JavaScript render.js`, as 72×72 PNGs. Covers are kept in its data folder (`OSH_PLUGIN_DATA/covers`): the current one for each player, and the last key drawn.
- **Network:** only for Spotify's covers, from `https://i.scdn.co/`; a redirect to any other host is refused. Apple Music's covers come from Music itself.
- **Presses that wait too long are dropped.** If a player takes more than 3 seconds to get to a press (a hung app, an Automation prompt), that press is dropped and logged rather than run late.
- **Two M4315 units:** the app names a key by module and index only, so the same key on two units counts as one: presses on both make one double press, and if the key is drawn for this action on one unit, a press of that key on the other is timed as if its release will come.
- A press is answered at once: the app sends the release only after that, and whether a press was single, double or long is known later. So a failure of the player (not allowed, not running) does not fail the press: it is written to the plugin's log in Settings > Plugins.

## Tests

From this folder: `/usr/bin/python3 -B -m unittest discover -s tests`. Players are faked, so the tests play, pause and skip nothing; one test draws the three key states with `render.js`.
