"""Adds the Claude Code plugin's hooks and status line to Claude Code's settings, or takes them out.

Run it from the installed plugin's folder, so the hooks point at the copy that stays put:

    /usr/bin/python3 "$HOME/Library/Application Support/dev.openstreamhub/plugins/com.github.goniszewski.claudecode/install_hooks.py"

Options:
    --remove           take the hooks and the status line out again (your own status line is put back)
    --no-statusline    leave the status line alone (the Context and cost key then shows nothing)
    --settings PATH    another settings file than ~/.claude/settings.json
    --dry-run          print the settings it would write, and write nothing

Before it adds the hooks to settings that do not have them, the settings file is copied next to itself
(settings.json.osh-backup); later runs leave that copy alone, so it keeps your settings from before the plugin. A settings file that is a symbolic link (a
dotfiles manager) is changed where it points, keeping its permissions. Hooks already there are kept: the plugin's are
added beside them, once, and run in the background ("async"), so they never slow Claude Code down. A status line you
already have keeps running: the plugin's runs it and prints what it prints. A status line that is not a command is
left alone.

Run --remove before removing the plugin in OpenStreamHub: its hooks would then point at a file that is gone. Any copy
of this script removes them, since it finds them by the plugin's id.
"""

import argparse
import json
import os
import shlex
import shutil
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
HOOK = os.path.join(HERE, "hook.py")
PYTHON = "/usr/bin/python3"
INSTALLED = os.path.expanduser("~/Library/Application Support/dev.openstreamhub/plugins/com.github.goniszewski.claudecode")
MARK = "com.github.goniszewski.claudecode/hook.py"  # how the plugin's commands are told apart from yours
EVENTS = ("SessionStart", "SessionEnd", "UserPromptSubmit", "PreToolUse", "PostToolUse",
          "PermissionRequest", "Notification", "Stop")


def hook_command():
    return "%s %s" % (PYTHON, shlex.quote(HOOK))


def statusline_command(wrapped):
    command = "%s %s statusline" % (PYTHON, shlex.quote(HOOK))
    return command + (" " + shlex.quote(wrapped) if wrapped else "")


def ours(entry):
    return isinstance(entry, dict) and MARK in str(entry.get("command", ""))


def wrapped_of(command):
    """The status line command the plugin's runs, or "" (None when `command` is not the plugin's)."""
    if MARK not in command:
        return None
    parts = shlex.split(command)
    at = parts.index("statusline") if "statusline" in parts else -1
    return " ".join(parts[at + 1:]) if at >= 0 else ""


class SettingsError(Exception):
    pass


def has_ours(settings):
    hooks = settings.get("hooks")
    if not isinstance(hooks, dict):
        return False
    return any(isinstance(group, dict) and isinstance(group.get("hooks"), list) and any(map(ours, group["hooks"]))
               for groups in hooks.values() if isinstance(groups, list) for group in groups)


def remove_hooks(settings):
    """Takes out the plugin's hooks, and only what held nothing else: a group left empty by that, then an event and
    the "hooks" object left empty by that."""
    hooks = settings.get("hooks")
    if not isinstance(hooks, dict):
        return
    emptied = False
    for event in list(hooks):
        groups = hooks[event]
        if not isinstance(groups, list):
            continue
        kept, touched = [], False
        for group in groups:
            if isinstance(group, dict) and isinstance(group.get("hooks"), list) and any(map(ours, group["hooks"])):
                touched = True
                group = dict(group, hooks=[entry for entry in group["hooks"] if not ours(entry)])
                if not group["hooks"]:
                    continue
            kept.append(group)
        if touched and not kept:
            del hooks[event]
            emptied = True
        else:
            hooks[event] = kept
    if not hooks and emptied:
        del settings["hooks"]


def install(settings, statusline=True):
    hooks = settings.get("hooks", {})
    if not isinstance(hooks, dict) or any(not isinstance(groups, list) for groups in hooks.values()):
        raise SettingsError('"hooks" is not an object of lists, as Claude Code expects; nothing was changed')
    remove_hooks(settings)
    hooks = settings.setdefault("hooks", {})
    for event in EVENTS:
        hooks.setdefault(event, []).append(
            {"hooks": [{"type": "command", "command": hook_command(), "async": True, "timeout": 10}]})
    current = settings.get("statusLine")
    if statusline and (current is None or isinstance(current, dict) and current.get("type") == "command"):
        wrapped = ""
        if isinstance(current, dict) and current.get("type") == "command" and current.get("command"):
            inner = wrapped_of(current["command"])
            wrapped = current["command"] if inner is None else inner
        line = dict(current) if current else {}
        line.update({"type": "command", "command": statusline_command(wrapped)})
        settings["statusLine"] = line
    return settings


def uninstall(settings):
    remove_hooks(settings)
    current = settings.get("statusLine")
    if isinstance(current, dict) and isinstance(current.get("command"), str):
        inner = wrapped_of(current["command"])
        if inner == "":
            del settings["statusLine"]
        elif inner is not None:
            settings["statusLine"] = dict(current, command=inner)
    return settings


def main(argv):
    parser = argparse.ArgumentParser(description="Add the Claude Code plugin's hooks to Claude Code's settings.")
    parser.add_argument("--remove", action="store_true")
    parser.add_argument("--no-statusline", action="store_true")
    parser.add_argument("--settings", default=os.path.expanduser("~/.claude/settings.json"))
    parser.add_argument("--dry-run", action="store_true")
    options = parser.parse_args(argv)

    path = os.path.realpath(options.settings)  # a symbolic link is changed where it points
    settings = {}
    if os.path.exists(path):
        with open(path, encoding="utf-8") as file:
            text = file.read()
        try:
            settings = json.loads(text) if text.strip() else {}
        except ValueError as error:
            print("%s is not valid JSON (%s); nothing was changed" % (path, error), file=sys.stderr)
            return 1
        if not isinstance(settings, dict):
            print("%s does not hold a JSON object; nothing was changed" % path, file=sys.stderr)
            return 1
    fresh = not has_ours(settings)  # the plugin's hooks are not there yet: these are the settings from before it
    try:
        if options.remove:
            uninstall(settings)
        else:
            install(settings, statusline=not options.no_statusline)
    except SettingsError as error:
        print("%s: %s" % (path, error), file=sys.stderr)
        return 1
    output = json.dumps(settings, indent=2) + "\n"
    if options.dry_run:
        print(output, end="")
        return 0
    os.makedirs(os.path.dirname(path), exist_ok=True)
    if os.path.exists(path) and (fresh or not os.path.exists(path + ".osh-backup")):
        shutil.copy2(path, path + ".osh-backup")  # the settings from before the plugin, not a later run's
    temporary = path + ".osh-tmp"
    with open(temporary, "w", encoding="utf-8") as file:
        file.write(output)
    if os.path.exists(path):
        shutil.copymode(path, temporary)
    else:
        os.chmod(temporary, 0o600)
    os.replace(temporary, path)
    if options.remove:
        print("Took the Claude Code plugin's hooks out of %s." % path)
    else:
        print("Added the Claude Code plugin's hooks to %s. Sessions started from now on show on the keys;" % path)
        print("restart Claude Code sessions that are already running.")
    if os.path.exists(path + ".osh-backup"):
        print("Your settings from before the plugin are in %s.osh-backup." % path)
    if not options.remove and not HOOK.startswith(INSTALLED + os.sep):
        print("Note: the hooks run %s, not the copy OpenStreamHub installed (%s); they keep working only while "
              "this file stays where it is." % (HOOK, INSTALLED), file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
