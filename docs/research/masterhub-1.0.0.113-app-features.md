# MasterHUB 1.0.0.113: official app feature catalogue

Analysis date: 7 October 2026. Purpose: ground which app-level features the official Windows MasterHUB 1.0.0.113 actually has, so [the parity tracker](../windows-parity-gaps.md) can mark focused-app rules, browser cards, flows, user scripts and the action catalogue as real or not. Static, text-only reading; nothing from the package was executed.

## Provenance

Existing extraction from the 3 October 2026 analysis, reused (no re-download). The main-process file and renderer map hashes were recomputed on 7 October and match [the static analysis](masterhub-1.0.0.113-static-analysis.md). The ZIP and `app.asar` hashes are taken from that document and were not recomputed.

| Item | SHA-256 |
| --- | --- |
| Official ZIP `MasterHUB-Setup_1.0.0.113.zip` | `cf9aef839255b2144b4c6268b274429be054f33a37e12cbf7537542cdaa0d83f` |
| `resources/app.asar` (cab1) | `a80844ac26e9478023213095006bf88db178712fb7a2e1be7b244d8445202f3f` |
| Renderer map `js/app.c762092d.js.map` | `79da9c6d60fca74ce4eb7842257f15e92139a8acbc9b38c29e2e332f89658435` |
| Main process `background.js` | `d3139e7214cb8195c3026165bb1c4b22324c18fca6bedf3b10aa85eba941d0ef` |

Workspace (outside the repo, untracked): `~/Documents/masterhub-analysis/1.0.0.113/app-analysis/` with `sources/src/` (178 non-language `.vue`/`.js` files, 77k lines) and `background.js`. Paths below are relative to `sources/src/` unless noted. Method: text reads with grep/sed, `python3 -I` for inventory. Only identifiers and line refs are quoted.

Caveat: "Absent" means the identifiers and behaviors were searched for in the renderer sources and in `background.js` and not found. The absence of focused-window, foreground-app and browser-extension identifiers was re-checked independently across both. Native `.node` addons, the SDK DLL and the separate MasterCTRL app/worker were not re-inspected.

## Feature table

| Feature | Verdict | Evidence |
| --- | --- | --- |
| Profiles: create, rename, duplicate, delete, import/export `.mhz` | Present | `lang/en_us.js` `profile.*` (l.119-152); `components/Common/TopNav.vue` `.mhz` accept (l.285) and check (l.2352); `lib/profileHelper.js` (`readProfileData`, per-serial files) |
| Profile switching by a bound key | Present | action `profile` next/previous (`store/module/masterHub.js` l.235-246); run at `App.vue` l.1376 |
| Per-app / focused-window profile rules | Absent | Zero hits for active-win, foreground, GetForegroundWindow, focusedWindow, autoSwitch, processName, exeName, bundleId, app-bind in renderer and `background.js`. Profiles change only via the `profile` key action or the start-up chooser (`profile.description`). Bundled `masterHubConfig_profile_Photoshop.mhz` is a template, not a rule |
| Browser integration / browser-tab cards | Absent | No extension, native-messaging or `chrome.tabs` code. "browser" appears only in `ipc-open-browser` (`shell.openExternal`, `App.vue` l.2456), the `web-url` action, and a Discord RPC field (`plugins/DiscordSingleton.js` l.57) |
| Multi-action ("Multi-Task") | Present | action `multi-tasking` (`masterHub.js` l.198-210); `App.vue` l.1270 and `executeMutitaskingAction` l.4472 runs `actionList` sequentially with a fixed 280 ms sleep. Not nestable: `unsupportMutitaskingList` in `masterHub.js` (multi-tasking, timer, clock, new-folder, page actions, media-fast-back-forward) |
| Conditional / triggered flows | Absent | No workflow/flow identifiers; multi-task is a flat list |
| Recorded key macro | Present | `components/modal/RecordMacroModal.vue`; `hotKeySettingMode` `'hotkey'` vs `'macro'` (`App.vue` l.1717-1735, l.4480); `system.hotkey_macro_*` (en_us l.292-298) |
| User scripts (user-authored) | Absent | No scripting or sandbox identifiers. Only fixed internal Adobe automation: `run-photoshop-script` (`App.vue` l.3017-3041) writes a temp `.jsx` for a hard-coded Photoshop 2024 path (`background.js`). `child_process` is used to launch URLs/apps |
| Action: hotkey / key combo | Present | `hot-key` "Key Actions" (`masterHub.js` l.494-503; `App.vue` l.1717), nut-js (`mixins/nutMixin.js`) |
| Action: launch / close app or file | Present | `app-files`, `actionAppMode` Open/Close (`App.vue` l.1405-1412; `closeApp` IPC) |
| Action: open URL | Present | `web-url` (`App.vue` l.1394, prefixes `https://`, opens via `explorer`) |
| Action: text snippet | Present | `text` (`App.vue` l.1778-1790): `copyText` IPC, Ctrl+V, optional Enter (`afterEnter`) |
| Action: media transport | Present | `media-track-control` (play/pause, stop, prev, next), `media-fast-back-forward`, `play-pause` (plays a chosen audio file) (`masterHub.js` l.362-428) |
| Action: volume | Present | `volume-control` up/down/mute (`masterHub.js` l.402-414) |
| Action: system | Present | undo/redo, switch desktop, `screen-capture`, zoom, custom scaling, scroll (`masterHub.js` l.334-465) |
| Action: timer | Present | `timer` with end sound (`mixins/timerMixin.js`; `App.vue` l.1791 `countDownWorker`) |
| Action: clock | Present | dial-encoder `clock` with date/analog modes (`FeatureSetting.vue` l.377; `customs/Clock.vue`; en_us l.310-312). Keypad support Unverified |
| Lighting effect / brightness actions | Present | `lighting-effect` (9 effects plus next/previous, `masterHub.js` l.212-231); `brightness` (l.249-261) |
| Folders | Present | `new-folder` (`masterHub.js` l.293-300); `item.folder` in `lib/profileHelper.js`; back tile at `App.vue` l.1286. Depth limit Unverified |
| Pages | Present | `create-new-page`, `previous-page`, `next-page` (`masterHub.js` l.302-326); `components/modal/PageEditModal.vue` |
| Per-module pages | Present | `SettingContents/{Keypad,Knobs,DialEncoder,Roller,Sliders5,MasterCtrl}Default.vue`; `components/Widget/*`; `types/constant.js` l.1-12 |
| Lighting (single module / whole kit) | Present | `components/Menu/LightingSetting.vue`; en_us l.154-173; `mixins/lightingEffectMixin.js` |
| Display: per-key icon/image/GIF/title | Present | `views/stack/base15key/editicon.vue`, `EditIconModal.vue`; GIF pool (`lib/profileHelper.js`); dial image generator `store/module/knobs.js` l.807-1082 |
| Display: system-stat widgets | Present | `types/constants/lcdDisplayMode.js` (CPU/GPU temp and usage, Image); `sysInfo` IPC (`App.vue` l.7170-7195); `masterHub.js` `displyItemMap` |
| Integrations: OBS | Present | ~18 actions over obs-websocket (`masterHub.js` l.505-714; `plugins/OBSWebSocketSingleton.js`; `App.vue` l.1819+) |
| Integrations: Twitch, Discord, Streamlabs | Present | `plugins/*`, `SettingContents/{twitch,discord,streamlabs}/*`; `masterHub.js` l.715, 907, 1007 |
| Integrations: Photoshop, Illustrator, Premiere | Present | preset groups `masterHub.js` l.1128, 1962, 2632 |
| Master CTRL bridge | Present | `ctrl-scenes`, `ctrl-lighting-profile`, `ctrl-cooling-profile` (`masterHub.js` l.864+; `App.vue` l.2505-2525); `connectCtrl` IPC. Needs the separate Master CTRL app; end to end Unverified |
| Firmware update | Present | `TopNav.vue` dialog (l.661+); `/api/devices/<uuid>/firmwares/stable` lookups (l.2169-2189); `downloadDeviceFirmware` IPC |
| Software self-update | Present | `settings.software_notice_new_version`; release lookup `.../api/softwares/.../releases/stable` (`TopNav.vue` l.1869); `updateSoftware` IPC |
| Cloud / login (Master ID) | Present (UI) | `settings.login/logout`, `backup_to_masterid` `.mhz` upload (`TopNav.vue` l.46-57, 268-285); auth window via `ipc-open-verify`. Backend Unverified |
| Autostart, languages, log export | Present | `isAutomaticallyStart` IPC, `lang/*`, `zip-log-file` IPC |
| Plugin / SDK / extension API | Absent | "plugin" means built-in integrations (`type: 'plugin'` in `masterHub.js`) |

## Implications for OpenStreamHub parity

Focused-app rules, browser-tab cards, user scripts and a plugin SDK do not exist in 1.0.0.113. They are not parity gaps, only possible OpenStreamHub extensions. Flows exist only as a flat multi-action list plus a recorded key macro.

| Rank | Feature | Value | Effort | Note |
| --- | --- | --- | --- | --- |
| 1 | Open URL | High | Low | OS opener |
| 2 | Full hotkey chords | High | Medium | Our `shortcut` is ASCII-only |
| 3 | Media and system actions | High | Low-medium | OS key synthesis |
| 4 | Multi-action (~280 ms spacing) | High | Medium | List-valued action in `profile.rs`; forbid nesting |
| 5 | Text snippet | Medium-high | Low | Clipboard plus paste; macOS permissions |
| 6 | Profile next/previous action | Medium-high | Low | |
| 7 | Folders and pages | High | Medium | Biggest model change |
| 8 | Per-key icon/GIF/title | High | Medium-high | Hardware-blocked verification |
| 9 | Stat widgets and clock | Medium | Medium | Sensors needed for temps |
| 10 | Recorded key macro | Medium | Medium-high | |
| 11 | Timer | Low-medium | Low-medium | |
| 12 | `.mhz` import/export | Medium | High | Format not analysed |
| 13 | Lighting | Medium | Medium | Protocol pending hardware |
| 14 | OBS/Twitch/Discord/etc. | Medium | High each | OBS first |
| 15 | Master ID backup | Low | High | Proprietary backend; skip |

## Open items

- `.mhz` schema.
- Folder depth and page limits (`store/module/keypad.js`).
- Whether `clock` exists on keypad tiles.
- Native addon and `worker/Ctrl.js` contents.
