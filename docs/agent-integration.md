# Answering AI agent prompts from the deck

With the **Terminal and AI agents** starter and a few agent hooks, the deck shows what a coding agent (Claude Code or Codex) is doing, and you can approve or deny its permission requests with a key press.

## What it does

The starter declares three variables:

- `agent_state`: `idle`, `working`, `waiting` or `done`, the summary of all your agent sessions (see below). The Accept key shows Working (amber), Needs you (red) or Done (green); idle shows its normal face.
- `agent_waiting`: a number, how many sessions wait for the deck. From 2 up the Accept key shows "2+ waiting" in dark red. The variable is optional: a profile without it works, without the count.
- `agent_decision`: `none`, `allow` or `deny`. Only the deck writes `allow` or `deny`: the app's socket, which `osh` uses, refuses to set it to anything but `none` ("only the deck can answer").

The hooks call the bundled `osh` tool:

- When you send a prompt, and after each tool the agent ran (`PostToolUse`), `osh agent state working`. The second one matters when you answered a prompt in the terminal yourself: the deck goes back to Working. It is not asynchronous, so it always finishes before the `Stop` hook of the same turn.
- When the agent asks for permission, `osh agent ask`. It sets `agent_state` to `waiting` and waits for the deck. While it waits, **Accept** sets `agent_decision` to `allow` and the **Interrupt** key (now showing a red Deny) sets it to `deny`. `osh` then answers the agent's prompt with that decision.
- When the agent finishes, `osh agent state done`. When a Claude Code session ends, `osh agent state end` (`SessionEnd`) forgets it. Codex has no such event: a finished session counts as Done for 30 minutes at most.

When nothing is waiting, Accept still sends Return and Interrupt still sends Escape, as before.

### Several sessions at once

Every hook tells `osh` its `session_id`, and `osh` keeps what each session is doing in `agents.json` next to the app's socket (guarded by `agents.lock`). The deck shows the summary: `waiting` if any session waits for you, else `working` if any works, else `done` if any finished, else `idle`. A session you did not name (a hand-typed `osh agent state working`) counts as one session called `default`.

The deck answers one permission request at a time. A second `osh agent ask` registers its session as waiting, then queues for the ask lock (`ask.lock`), oldest first, and shows its prompt on the deck when the first one is answered. Accept and Deny always answer the oldest waiting prompt. Every ask has its own timeout, which includes the time in the queue: one that waits too long prints nothing, so that agent shows its normal prompt. A state hook never hides a prompt that a live `osh agent ask` owns. The registry forgets sessions that were silent for 12 hours, finished ones after 30 minutes, and waiting ones whose `osh agent ask` is gone (killed, for instance) or that waited longer than the longest ask (11 minutes). A hook that cannot get the registry lock within 2 seconds changes nothing. The system releases the lock when the waiting `osh` ends, however it ends.

Between two prompts the deck shows Working for about a second and a half, then Needs you again for the next prompt. Press Accept or Deny once per prompt and wait for Needs you: the next ask clears every earlier press before it starts listening, so a repeated press for the first prompt cannot answer the second, which you had not seen. During that pause a press does what the key does outside a prompt (Accept sends Return to the app in front). Two asks of one session (a subagent, parallel tools) take separate places in line.

## Start a session

The starter's **Claude** and **Codex** keys use the **Open terminal and run** action (`terminal_command`): a press opens a new Terminal window and runs `claude` or `codex` there, as if you had typed it. You can add your own in the editor:

- **Terminal app:** Terminal or iTerm2. These two are the only choices, because they can be told to run a command in a new window.
- **Command:** one line of up to 512 bytes, run by your own login shell, so it can do anything you could type yourself. Only bind commands you trust.
- **Start in folder** (optional): a path starting with `/` or `~/`. The window runs `cd <folder> && <command>`, with the folder quoted so that nothing in it runs as a command.

It is not a general shell action: it only opens a terminal window with a command. It cannot be a step of a multi-action or an if / else, so no sequence can type into the window it opens. The command is passed to the terminal as data, never as part of the AppleScript that opens the window, and the activity log records only its length.

The first time a key opens a terminal, macOS asks whether OpenStreamHub may control Terminal (or iTerm2). Allow it in that prompt, or later in **System Settings > Privacy & Security > Automation**. Until you do, the key reports an error and opens nothing.

A profile you import from a JSON file that holds this action, or any action that writes `agent_decision`, anywhere in it, needs the usual review, like an imported MasterHUB or preset profile: it runs only as a dry run until you open it in the profile manager, review what it does and allow it to execute actions. The Execute confirmation lists every command line, folder and terminal app, and says when a key answers AI agent permission prompts. That review is about these two actions only: an imported profile can still open a terminal with **Open application** and type text into it without being flagged, although the Execute confirmation discloses both. Saving a running profile so that a key now answers agent prompts also counts as a new capability: a Live run pauses for one confirmation.

## Install

1. Create a profile from the **Terminal and AI agents** starter (**Manage profiles** in the header, then **New from a starter**) and run it. A profile made from an older version of the starter lacks `agent_decision`: create it again, or declare `agent_decision` (text, `none`) and set up the two keys yourself.
2. Open **Settings > AI agents (Claude Code, Codex)**. The panel shows the bundled `osh` it found and the hook configuration for each agent, already written with that `osh`'s absolute path, each with a **Copy** button.
3. **Claude Code:** merge the copied `hooks` object into `~/.claude/settings.json` (or a project's `.claude/settings.json`). If the file already has hooks for `UserPromptSubmit`, `PostToolUse`, `PermissionRequest`, `Stop` or `SessionEnd`, add the new entries to those arrays. A configuration you installed with an earlier version still works; copy the new one to get `PostToolUse` and `SessionEnd`.
4. **Codex:** add the copied lines to `~/.codex/config.toml`. Codex runs new hooks only after you review and trust them, so run `/hooks` in Codex once after adding them. Codex's `notify` setting only reports a finished turn, so the `Stop` hook is used instead.

The app never edits these files: you paste the configuration yourself. The panel's **First-run checklist** only reads them: it looks in `~/.claude/settings.json` and `~/.codex/config.toml` for a hook command that runs `osh agent ask` or `osh agent state` (the path before `osh` is not compared), and shows "Not found in …" when there is none, or "Could not read …" when the file exists but cannot be read (for Claude Code, JSON that does not parse; Codex's TOML is scanned line by line, not parsed). A missing file counts as not configured. Project-level `.claude/settings.json` files are not looked at. The last step turns done after the deck has answered a prompt: the app counts Accept and Deny answers in memory since it started, with no prompt text, and forgets them when it quits. **Check again** re-reads everything. The hooks call `osh` by its full path, so they work whether or not `osh` is on your PATH.

### From the command line

Without the panel (or to script it), `osh` prints the same configurations. They use the absolute path of the `osh` you run:

```sh
/Applications/OpenStreamHub.app/Contents/MacOS/osh agent setup claude
/Applications/OpenStreamHub.app/Contents/MacOS/osh agent setup codex
```

For Claude Code the merge instructions are printed on stderr, so `osh agent setup claude > hooks.json` gives plain JSON.

The permission hook waits up to 55 seconds (`--timeout`, at most 600), and the hook's own timeout is 65 seconds, so `osh` always gives up first.

## Dictation key

The starter's last key, **Dictate**, sends F19. macOS can start and stop dictation from a custom shortcut: open **System Settings > Keyboard > Dictation**, choose **Shortcut > Customize…** and press the Dictate key. Each press then starts or stops dictation in the app in front (keep the terminal focused). A key press cannot be held, so this is not push-to-talk. Until the shortcut is set, the key does nothing.

## When the deck is not there

`osh` fails open: the agent's normal permission prompt appears whenever the deck cannot answer.

- On timeout, `osh agent ask` prints nothing, sets `agent_decision` back to `none` and `agent_state` to `idle`, and the agent shows its usual prompt.
- If the app is closed, no module runs a profile that declares both variables, or the app quits while `osh` waits, it prints nothing at once and the usual prompt appears.
- If `osh` is stopped (Ctrl+C or a termination signal), it resets the variables the same way.
- Before it waits, `osh agent ask` clears `agent_decision` in strict mode: every running module that declares it must apply the clear. If one does not (it fails or does not answer in time), the ask prints nothing.
- Saving the running profile restarts a run that draws on the deck. The app reads the variables just before it stops the run and gives them back to the new run (those the new profile still declares, with the same kind; within 20 seconds), so `agent_state` survives a save. Not carried: `agent_decision` (it starts at `none`, so an old Accept cannot answer a prompt in the next run), `agent_waiting`, and an `agent_state` of `waiting`, because the ask that showed it cannot survive the restart; the deck shows the next hook's state instead.
- `osh agent state` always exits 0 and prints nothing, whether or not the app runs, so it never blocks a prompt.

## Safety

- Only a deck press allows a request. The app's socket accepts no `agent_decision` value but `none`, so neither `osh set` nor any other program talking to the socket can write `allow` or `deny`; a deck key, an action inside the running profile, is the only writer. `osh agent ask` clears `agent_decision` before it starts waiting, so an old `allow` does not count, and the keys write a decision only while `agent_state` is `waiting`.
- `osh` never allows on an error: anything unexpected prints nothing, which leaves the decision to the agent's own prompt. A `deny` from any running module wins over an `allow`.
- The app's socket is in `~/Library/Application Support/dev.openstreamhub/ingress/` (a directory owned by you, 0700, files 0600; the app turns the socket off if that path is a symbolic link or belongs to someone else), and every request carries a token the app writes anew at each launch. Any program running as your user can read that token, so it can set `agent_state`, clear `agent_decision`, or start an ask of its own; it cannot answer one. Treat it like the agent's own terminal: anything running as you could already type into it.

## Where osh is

`osh` ships inside the app at `/Applications/OpenStreamHub.app/Contents/MacOS/osh`. To call it as `osh` in a terminal, press **Make `osh` available in the terminal** in the AI agents panel. It creates one symbolic link, `~/.local/bin/osh`, to the bundled `osh` (and `~/.local/bin` if it is missing). It never replaces a regular file there, or a link to another program; a link to the `osh` of another copy of OpenStreamHub is updated. **Remove link** deletes only that link, and only while it points at an OpenStreamHub `osh`.

macOS does not put `~/.local/bin` on the PATH by itself. The app sees only its own PATH, not your shell's, so when it cannot confirm the folder is there it shows the line to add to `~/.zshrc` (or `~/.bash_profile`):

```sh
export PATH="$HOME/.local/bin:$PATH"
```

Without the panel, link it yourself, for example:

```sh
ln -s /Applications/OpenStreamHub.app/Contents/MacOS/osh ~/.local/bin/osh
```

`osh agent setup` writes the path of the `osh` you ran, so run it from where the hooks should find it.

If macOS runs the app from where it was downloaded (App Translocation, a temporary copy whose path changes at every launch), the panel offers no link or configuration and `osh agent setup` exits with an error. Move OpenStreamHub to Applications and open it from there.

## Not yet verified

- Whether `PermissionRequest` fires in Claude Code's auto-accept or bypass-permissions modes, or with Codex's approval policies that skip prompts. When it does not fire, the deck is not asked.
- Whether Codex accepts the `PostToolUse` hook name used in its snippet (it is in Codex's hook documentation; if `/hooks` rejects it, delete that table), and whether Claude Code and Codex send `session_id` in every hook (without it all hooks count as the one `default` session, as before).
- The hook configurations were written from the Claude Code and Codex documentation. `osh` itself is tested against a fake app, but neither agent has been run end to end with the deck yet.
