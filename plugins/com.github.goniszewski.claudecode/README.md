# Claude Code

An OpenStreamHub plugin that puts your [Claude Code](https://code.claude.com) sessions on M4315 keys: which one is working, which one waits for you, which one is done. A press brings that session's terminal to the front, and, if you turn it on, answers its permission prompt once you can see it.

| Key | Shows |
| --- | --- |
| **Session status** | One session: its project, and **?** with the tool's name while it waits for you (amber), a check when it is done (green), dots while it works, a dash while idle. A red badge counts the other sessions that wait for you or are done. |
| **Context and cost** | A ring of how full the session's context window is (green, then amber from 60 %, red from 85 %), with what the session has cost below. |

On the M131E dial, an action drawn on a dial function shows while that function is current: **Context and cost** as a large ring around the screen with the percentage, the project and the cost inside, and **Session status** as on a key, enlarged. Bind it to the function's turn (turning then shows the next or previous session), and leave the dial screen on anything but Off in the profile's dial settings. The dial is one control, so a session chosen by turning it shows on every dial function of this plugin.

Which session a key shows: the one that most needs you (waiting for you, then done, then working, then idle, the latest first), of the key's **Project** when it is set (the project folder's name, such as `OpenStreamHub`). A done session you bring to the front from the key counts as seen and shows idle.

| On a key | Does |
| --- | --- |
| Short press | Brings the session's terminal to the front (the very tab in iTerm2 and Terminal; other terminal apps, and tmux or screen, are only brought forward). |
| Long press (held 600 ms) | Shows the next session, for 30 seconds. |

On a knob, the dial or a roller, turning shows the next or previous session, and a press works as on a key.

The condition **A Claude Code session is…** lets an if / else test whether a session (of a project, when set) is waiting for you, working, done or idle.

**Bind it to the key's own Press, and leave its Release unbound**: a long press is told apart only on a key the app draws for this action. Anywhere else a press acts at once.

Requires OpenStreamHub 0.5.0 or later, macOS, `/usr/bin/python3` (it comes with the Xcode command-line tools), and Claude Code with hooks. Status: unit-tested with fakes; on 17 November 2026, by hand, through a fake app socket, with a real Claude Code session in iTerm2 (behind a wrapper with a terminal of its own): its hooks, the status line, the keys, and answering real Bash permission prompts, Yes with a short press and No with a long one. Not yet checked on a real M4315 or M131E, or in Terminal.

## Answering permission prompts

With **Answer permission prompts** on (off by default, per key), a key whose session waits for a permission answers it, but only what you can see:

1. The first press brings the terminal to the front, so you read the prompt there, not on the key.
2. Once that terminal has been the front window for a moment and the prompt is the last thing in it, a **short press answers Yes, once** (it types Return, with **1. Yes** chosen) and a **long press declines** (it types Escape, as **No**).

Before it types anything, the plugin reads the terminal's text and checks that the prompt is Claude Code's permission prompt, last on screen. In the same step that types the key it checks again that the terminal is in front and its text has not changed since; otherwise it types nothing and logs why. It answers only a prompt you have had time to see: the key has shown the session waiting, the prompt has been on its screen and the terminal in front, each for at least 0.7 s (a press while the key still shows something else does what that showed). It also types nothing when another program than that Claude Code session (or the wrapper it runs in) is in the foreground of that terminal, when another option than **1. Yes** is chosen, or for a question Claude Code asks you (only permissions, "Do you want to …?", are answered). "Yes, and don't ask again" is never chosen from a key. A press that waited more than a second (behind a macOS permission dialog, say) is dropped, never run late.

This works in **iTerm2** and **Terminal**, whose text the plugin can read, and only in the tab or session whose terminal is the Claude Code process's (or one above it, as when a wrapper gives Claude Code a terminal of its own). Under tmux or screen, in other terminal apps (also one started from iTerm2 or Terminal), and in editors, a press only brings the app forward: there the plugin cannot tell one pane from another. In iTerm2 the keys go to that session only; in Terminal they are key presses, sent only while that tab is in front and Terminal is the frontmost app.

When a prompt is answered in the terminal itself, the key goes back to working within about two seconds (iTerm2 and Terminal), or at the next thing Claude Code does.

## Install

1. In OpenStreamHub, open **Settings > Plugins**, choose **Install…**, pick this folder (`plugins/com.github.goniszewski.claudecode` in a copy of the repository) and choose **Install and enable**.
2. Add the hooks to Claude Code, from the installed copy (so they keep working when you delete the download):

   ```sh
   /usr/bin/python3 "$HOME/Library/Application Support/dev.openstreamhub/plugins/com.github.goniszewski.claudecode/install_hooks.py"
   ```

   It adds its hooks beside any you have in `~/.claude/settings.json` (your settings from just before are kept in `settings.json.osh-backup`; a linked settings file is changed where it points), and a status line that feeds **Context and cost**; a status line you already have keeps running and showing. `--no-statusline` leaves the status line alone, `--dry-run` prints the result without writing it, and `--remove` takes everything out again. Restart running Claude Code sessions.

   **Before you remove the plugin** in Settings > Plugins, run the same command with `--remove`: afterwards the hooks and status line would point at a file that is gone (and your own status line, which the plugin's runs, would show nothing). Any copy of `install_hooks.py`, such as the one in the repository, can remove them.
3. Bind a key's **Press** to the **Plugin** action (Apps & web), with **Claude Code** as the plugin and **Session status** (or **Context and cost**) as its action, and run the profile in **Live**, with Settings > Let OpenStreamHub drive the display set to **App**.

The first time the plugin reads or brings forward iTerm2 or Terminal, macOS asks whether OpenStreamHub may control it. Choose **OK**; if you chose **Don't Allow**, allow it in System Settings > Privacy & Security > Automation. Answering in Terminal presses keys through System Events, so macOS may also ask for Accessibility (System Settings > Privacy & Security > Accessibility); answering in iTerm2 does not need it.

## What it does on your Mac

- **The hooks** (`hook.py`) run in the background ("async") at each Claude Code event, so they never slow it down or change what it does: they decide nothing and print nothing. Each writes a small file per session to the plugin's data folder (`~/Library/Application Support/dev.openstreamhub/plugin-data/com.github.goniszewski.claudecode/sessions`): its state, the tool's name, the project folder's name, the Claude Code process and its terminal (the app, iTerm2's session id, and the ttys of the process and those above it). Never your prompts, commands, paths or files. A session that ends leaves a short "ended" record for 10 minutes, so a hook that runs late cannot bring it back; a session whose Claude Code process is gone is removed.
- **The status line** (`hook.py statusline`) adds the context window's use, the cost and the model's name to that file.
- **The plugin** reads those files every second while one of its keys is on screen, and draws keys with AppKit (`render.js`). It reads the text of a waiting session's iTerm2 session or Terminal tab, only to see whether its prompt is still there, and keeps none of it. It uses no network.

## Tests

From this folder: `/usr/bin/python3 -B -m unittest discover -s tests`. Terminals are faked, so the tests bring nothing to the front and type nothing; one test draws every key face with `render.js`.
