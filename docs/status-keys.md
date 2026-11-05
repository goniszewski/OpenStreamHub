# Status keys: show your work on the deck

A key can change its face when a profile variable changes (Inspector > Faces). `osh set` changes variables from outside the app, so any script can put a state on a key: a dirty git tree, a failing CI run, a test run in progress.

`scripts/deck-status/deck-status.sh` does the three most useful ones:

```sh
scripts/deck-status/deck-status.sh git ~/code/project --watch 5   # git_state: clean | dirty | unknown
scripts/deck-status/deck-status.sh ci ~/code/project --watch 30   # ci_state: passing | failing | running | unknown (needs gh)
scripts/deck-status/deck-status.sh run tests -- npm test           # tests: running, then passing | failing
```

`run` passes the command's exit code on, so it can sit in front of any command, in a shell alias or a git hook.

## Set up a key

1. Under **Variables**, add a text variable with the name the script sets (`git_state`) and the value `unknown`.
2. Select a key, and under **Faces** add one face per value: variable `git_state` is `dirty`, label "Dirty", an amber background; another for `clean`, label "Clean", green. The first face that matches is drawn, and with no match the key keeps its normal face.
3. Run the script. The key changes within a moment.

A key does not need an action of its own for this; pick any harmless one (for instance Open application, with your terminal).

The script calls `osh` from your PATH, else the one inside the app (`/Applications/OpenStreamHub.app/Contents/MacOS/osh`); set `OSH` to use another. It never fails when the app is closed or the variable is not declared: a status script must not break your build. Variables start again from their declared values when the app starts, and they are kept when you save the profile.

Writing your own is one line: `osh set --kind text build_state failing`. The value is the text a face tests.
