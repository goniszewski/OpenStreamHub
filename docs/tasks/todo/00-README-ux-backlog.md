# UX/UI backlog (from the comparison with official MasterHUB 1.0.0.113)

Source: official app Vue sources (`~/Documents/masterhub-analysis/1.0.0.113/...`), Cooler Master pages and reviews, and our `apps/open-stream-hub/src`. Note: the one screenshot reviewed was stale (3 Oct); confirm findings against a live run before starting task 01.

## Suggested order
1. 01 Strip placeholders and jargon (P0)
2. 05 Demote simulation, header Run control (P0)
3. 02 Searchable action picker (P0), then 03 OBS grouping, 04 OBS live pickers
4. 14 Autosave (P1)
5. 06 Merge Devices and Profiles (P1, large, after 01 and 05)
6. 07 Key icon library, 08 Key copy/paste, 10 Dial inspector, 09 Lighting (P1)
7. 11 Templates, 12 Macro recorder, 13 Focus rules/undo (P2)
8. 15 Localisation (P3)

## Added from the press reviews (Noisy Pixel, APH Networks, Tom's Hardware)
Suggested order, after the tasks above:
1. 30 Automatic key icons (P1), 31 Profile backups (P1), key picker inside 12 (cheap, can ship before the recorder)
2. 28 Per-input lighting, 29 Fader action set, 36 OBS connection help (P2)
3. 34 App preset packs (after 11), 35 Quit/crash behaviour check (P2, hardware)
4. 32 State-aware keys (P2, after 18 and 27), 33 Live press feedback (P3, after 32), 37 Orientation check (P3, hardware)

## Conventions
One file per task: summary, why, scope, acceptance, hardware needed. Move the file to `../in-review` or `../done` as it progresses.
