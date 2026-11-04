# Answering AI agent prompts from the deck

With the **Terminal and AI agents** starter and a few agent hooks, the deck shows what a coding agent (Claude Code or Codex) is doing, and you can approve or deny its permission requests with a key press.

## What it does

The starter declares two text variables:

- `agent_state`: `idle`, `working`, `waiting` or `done`. The Accept key shows Working (amber), Needs you (red) or Done (green); idle shows its normal face.
- `agent_decision`: `none`, `allow` or `deny`. Only the deck writes `allow` or `deny`.

The hooks call the bundled `osh` tool:

- When you send a prompt, `osh agent state working`.
- When the agent asks for permission, `osh agent ask`. It sets `agent_state` to `waiting` and waits for the deck. While it waits, **Accept** sets `agent_decision` to `allow` and the **Interrupt** key (now showing a red Deny) sets it to `deny`. `osh` then answers the agent's prompt with that decision.
- When the agent finishes, `osh agent state done`.

When nothing is waiting, Accept still sends Return and Interrupt still sends Escape, as before.

There is one `agent_state` and one `agent_decision` for all sessions. With several agents running at once, the deck shows whichever set the state last, and a key press answers whichever is asking. If two ask at the same time, one press can answer both. Use it with one agent at a time if that matters to you.

## Install

1. Create a profile from the **Terminal and AI agents** starter and run it. A profile made from an older version of the starter lacks `agent_decision`: create it again, or declare `agent_decision` (text, `none`) and set up the two keys yourself.
2. Print the hook configuration. It uses the absolute path of the `osh` you run:

   ```sh
   /Applications/OpenStreamHub.app/Contents/MacOS/osh agent setup claude
   /Applications/OpenStreamHub.app/Contents/MacOS/osh agent setup codex
   ```

3. **Claude Code:** merge the printed `hooks` object into `~/.claude/settings.json` (or a project's `.claude/settings.json`). If the file already has hooks for `UserPromptSubmit`, `PermissionRequest` or `Stop`, add the new entries to those arrays. The merge instructions are printed on stderr, so `osh agent setup claude > hooks.json` gives plain JSON.
4. **Codex:** add the printed lines to `~/.codex/config.toml`. Codex runs new hooks only after you review and trust them, so run `/hooks` in Codex once after adding them. Codex's `notify` setting only reports a finished turn, so the `Stop` hook is used instead.

The permission hook waits up to 55 seconds (`--timeout`, at most 600), and the hook's own timeout is 65 seconds, so `osh` always gives up first.

## When the deck is not there

`osh` fails open: the agent's normal permission prompt appears whenever the deck cannot answer.

- On timeout, `osh agent ask` prints nothing, sets `agent_decision` back to `none` and `agent_state` to `idle`, and the agent shows its usual prompt.
- If the app is closed, no module runs a profile that declares both variables, or the app quits while `osh` waits, it prints nothing at once and the usual prompt appears.
- If `osh` is stopped (Ctrl+C or a termination signal), it resets the variables the same way.
- `osh agent state` always exits 0 and prints nothing, whether or not the app runs, so it never blocks a prompt.

## Safety

- Only a deck press allows a request. `osh agent ask` clears `agent_decision` before it starts waiting, so an old `allow` does not count, and the keys write a decision only while `agent_state` is `waiting`.
- `osh` never allows on an error: anything unexpected prints nothing, which leaves the decision to the agent's own prompt. A `deny` from any running module wins over an `allow`.
- The app's socket is in `~/Library/Application Support/dev.openstreamhub/ingress/` (directory 0700, files 0600), and every request carries a token the app writes anew at each launch. Any program running as your user can read that token, so any of them could set `agent_decision` too. Treat it like the agent's own terminal: anything running as you could already type into it.

## Where osh is

`osh` ships inside the app at `/Applications/OpenStreamHub.app/Contents/MacOS/osh`. To call it as `osh`, link it onto your PATH:

```sh
ln -s /Applications/OpenStreamHub.app/Contents/MacOS/osh /usr/local/bin/osh
```

`osh agent setup` writes the path of the `osh` you ran, so run it from where the hooks should find it.

## Not yet verified

- Whether `PermissionRequest` fires in Claude Code's auto-accept or bypass-permissions modes, or with Codex's approval policies that skip prompts. When it does not fire, the deck is not asked.
- The hook configurations were written from the Claude Code and Codex documentation. `osh` itself is tested against a fake app, but neither agent has been run end to end with the deck yet.
