# Importing official MasterHUB profiles

OpenStreamHub can import a profile exported by the official MasterHUB app for Windows (a `.mhz` file), or the `.zip` of several `.mhz` files that the official app writes for a multi-profile export. The format and the mapping rules are in [the `.mhz` format analysis](research/masterhub-mhz-format.md). The importer has been tested with synthetic files only, not with real exports.

## How to import

1. Open **Manage profiles** (next to the profile switcher in the app header) and choose **Import official .mhz**, or drop the file onto the workspace.
2. Read the report. While the profile is still dry run only, its banner can reopen the report of the latest import of this app session; after a restart or another import it is gone. It lists every control in the file: imported, imported with changes, or left unbound, and why. Values are shortened: a program path shows only its file name, a web address only its `scheme://host`, and text only its length.
3. The new profile opens. It is always a new profile named `<name> (imported)` (or `(imported 2)`, and so on). An existing profile is never changed or overwritten.

## Dry run only until you allow it

An imported profile runs only as a dry run. The **Execute actions** mode is unavailable for it in the Run panel, and the session manager refuses to start, resume or switch an executing run into it, whether by Start, by a focused-app rule or by a profile key. Anything derived from it in this app stays dry run only too: an edited draft (also one recovered after a crash), a duplicate, a copy saved over another profile, a JSON export made here (also of unsaved edits) and imported again, and the open draft after you delete it.

If the list of imported profiles is ever missing or damaged, no profile can run with actions and the profile manager says so. Choose **Mark all profiles as needing review** to continue; every profile then runs only as a dry run until you allow it again. If the app also cannot read its marker file, it says so; to recover by hand, quit OpenStreamHub, replace `profiles/.imported-profiles` in the app data folder with an empty file, start the app again and choose **Mark all profiles as needing review** (imported profiles you deleted earlier are then no longer tracked). Never delete `imported-profiles.json` or the marker to recover: that would make every profile runnable. Content of imported profiles you deleted earlier stays dry run only too: a second copy of those fingerprints is kept beside the profiles.

To let it run actions:

1. Open it with **Manage profiles** and review its bindings. Save or discard any changes first. While the list of imported profiles cannot be read, saving an existing profile or a copy of one is refused; only brand-new profiles can be saved until you recover.
2. Choose **Allow this imported profile to execute actions…**. The confirmation names the profile and lists everything it can do: pasted text (shown in full), web pages, screenshots, shortcuts and multi-step sequences.
3. Tick the acknowledgement and choose **Allow actions**. Starting a run with actions still asks for confirmation every time.

## What is imported

- Keyboard shortcuts, when every key has a macOS equivalent. Windows shortcuts are kept as they were (Ctrl stays Control, Win becomes Command), so review them: on macOS the same command usually uses Cmd.
- Web addresses that pass the URL rules, text snippets, volume and mute, media keys, undo/redo, screenshot, profile next/previous, multi-step actions whose every step maps, folders and pages (as keypad folders), key labels, knob and roller volume or shortcut sides, dial functions and fader volume.

## What is never imported

Program launches and closes (`app-files`), recorded macros, timers, clocks, lighting and brightness, OBS, Twitch, Discord, Streamlabs and Master CTRL actions, application scripts, images and GIFs, colours and fonts. These controls stay unbound and are listed in the report. A multi-step action with any such step is left unbound as a whole. Device serial numbers and module identifiers from the file (wherever they appear in it, including on individual keys) are never stored, nor are the paths of program launches. Text snippets, web addresses and key labels are imported as they are (labels and names lose invisible characters, and anything containing a device serial is left out).

## Limits

A file larger than 32 MiB, an archive with unexpected or unsafe entries, encryption, more than 256 entries, a decompression bomb, a profile JSON over 16 MiB, invalid or deeply nested JSON, a file naming more than 64 device serial numbers, or more than 8 profiles in a multi-profile `.zip` is refused as a whole, and no profile is created. Images inside the archive are skipped without being read. The report lists at most 1,000 controls and stays under 512 KiB; controls past that are counted but not listed. Exporting to `.mhz` is not supported.

## Residual risks

The dry-run rule follows profiles by a fingerprint of their contents and by where a draft came from. It cannot follow content that leaves this app's control:

- A JSON file edited by hand (or by another tool) after export, or a profile written outside this app, has a new fingerprint and imports as an ordinary profile.
- Copying the profile folder elsewhere without its hidden files (the `.imported-profiles` marker) and without `imported-profiles.json` loses the record of which profiles were imported.
- Twins (for example the same file imported twice) share a fingerprint. Allowing one of them clears that shared fingerprint: the other twin still shows as dry run only and stays blocked by its own record, but a copy of it saved without its source (for example a re-imported export) can then run. The reverse also happens: exporting the twin that is still dry run only flags the shared fingerprint again, so the allowed twin becomes dry run only again until you allow it once more.
- The list keeps at most 1,024 records and 4,096 fingerprints. Past that, only records of deleted profiles are dropped (oldest first), and a fingerprint is never dropped while any saved profile has that content (with or without a record): first fingerprints nothing names, then those of deleted profiles, and fingerprints of exported files last. If the caps still cannot be met, the save, import or export is refused instead.
- Profiles deleted while the list could not be read are remembered for the recovery action (up to 1,024).
- An export only stays flagged when this app knows where the draft came from. A request that omits or spoofs the draft's source (`sourceProfileId`) is trusted for what it says; content that matches a flagged fingerprint is still caught.
- Exporting edited content whose source is flagged flags that content wherever it appears, so an ordinary profile with exactly the same bindings becomes dry run only too (an expected false positive).
- A diagnostic bundle can include the open profile with its text and web addresses redacted. That profile is different content, so importing it from the bundle creates an ordinary profile that is not dry run only (its shortcuts and structure can then run like any profile you made, after the usual Execute confirmation).
- The app cannot prove that a person clicked **Allow actions**; the confirmation is that step.
- An imported profile with no binding at all is tracked only by its id and by drafts opened from it, never by its contents, so a blank new profile is not dry run only.
- When more than 128 profile files exist, deleting an imported profile leaves its record as if the profile still existed (the list cannot be updated safely); it stays dry run only and takes one of the 1,024 records until the extra files are removed.
- The profile list shown in the app trusts each file's modification time and size: a file rewritten outside the app with exactly the same size and time keeps its old summary until it changes again. Flags by id still apply, and allowing a profile always re-reads and re-hashes the file.
- Two different profiles that happen to have identical bindings share a fingerprint: if one is dry run only, the other is too (an expected coincidence, not a leak).
