# Windows parity gaps

Tracker of feature gaps between the official Cooler Master MasterHUB 1.0.0.113 Windows software and OpenStreamHub plus the CoolTEK SDK. Compiled on 6 October 2026 from the analysis documents and a check of the code on branch `parity-docs-and-focus-rules`. No hardware was available and nothing here was run on a device.

## How to read this

- **Official capability** comes only from the static-analysis documents ([static analysis](research/masterhub-1.0.0.113-static-analysis.md), [additional modules](research/masterhub-additional-modules-1.0.0.113.md), [Windows software analysis](masterhub-windows-software-analysis.md), [keypad folders](masterhub-keypad-folders.md)). Anything else is marked **unverified**.
- **Our status** values: **Done** (implemented and tested offline; hardware acceptance still pending unless stated), **Partial**, **Missing**, **Hardware-blocked** (cannot be finished or accepted without a device), **Out of scope** (excluded by the roadmap).
- "Done" never means hardware-verified. [Protocol evidence](masterhub-protocol-evidence.md) lists every capability as hardware-pending.
- Some older docs are stale and were corrected against code. [Implementation status](masterhub-implementation-status.md) (lines 50 and 60) and [protocol evidence](masterhub-protocol-evidence.md) (row "Explicit dial lifecycle") still say there is no reconnect, no sleep/wake observer and an offline-only app. In code, `apps/open-stream-hub/src-tauri/src/session.rs`, `power.rs`, `service.rs` and `feed.rs` provide a supervised `masterhub-service` sidecar, sleep/wake handling, reconnect and a live read-only feed. See [session manager](masterhub-session-manager.md).

## Important limit of the evidence

The protocol analysis documents cover the device (discovery, input, display, mode and lighting commands, startup/suspend). The app-level features were settled on 7 October 2026 by a source-map pass over the official renderer: see [app features](research/masterhub-1.0.0.113-app-features.md). Focused-app rules, browser-tab cards, user scripts and a plugin SDK are **absent** from MasterHUB 1.0.0.113; the action catalogue, a flat multi-action list and a recorded key macro are **present**. The native `.node` addons, the SDK DLL and the separate Master CTRL app were not re-inspected in that pass.

## Summary table

| Area | Official software capability | Our status | Evidence | Next step |
| --- | --- | --- | --- | --- |
| Discovery: PID generations | Accepts legacy and newer PIDs for base, knobs, dial, keys, faders, rollers, VID `2516` | Done | `crates/cooltek-sdk-masterhub/src/modules/m0064.rs`, `discovery.rs` (`ModuleKind`); static analysis "Module discovery" | Confirm descriptors/interfaces on hardware |
| Discovery: serial identity | Connects by serial | Done | `session.rs` (module + serial identity); [session manager](masterhub-session-manager.md) | Hardware check of serial stability across replug |
| Discovery: hot-plug inventory | Not documented for the official app (unverified) | Done (ours) | `src-tauri/src/feed.rs` `watch_inventory`; README "Rig preview" | Hardware check |
| Input: M131E dial | Rotate, press, short/long press (host 600 ms) | Done | `crates/cooltek-sdk-masterhub/src/input/m131e.rs`, `dial.rs`; `Gesture` in `cooltek-sdk-service/src/profile.rs` | Capture real payloads and direction |
| Input: dial function cycling | Short click switches the dial's current function; long press runs the press action | Partial | Service crate: optional `dial_functions` in `profile.rs`, `ProfileSession::route` (`keypad.rs`), `dial.rs`; [dial functions](masterhub-dial-functions.md). Not yet in CLI event logging, app replay previews or the editor UI; no hardware check of the click sequence | Integration checklist in the dial-functions doc; hardware check of short click and the 600 ms threshold |
| Input: M003K knobs | Rotation and press | Done | `input.rs`, `modules/m003k.rs`; protocol evidence rows M003K | Hardware capture |
| Input: M4315 keys | 15-key press/release | Done | `input/m4315.rs`; additional-modules "Input framing" | Hardware key order |
| Input: M005S faders | Five absolute positions 0-100 | Done | `input/m005s.rs`; `Action::VolumeFromPosition` in `profile.rs` | Hardware range/jitter |
| Input: M032LE rollers | Up/Down per roller, `ff` idle | Done | `input/m032le.rs` | Hardware direction/order |
| Display: dial image | 240x240 JPEG, chunked `02 45` reports | Partial | SDK `dial::display_jpeg`, `display_jpeg_with_pacing` (`dial.rs`); typed `display_jpeg` in `hid/handle.rs`; CLI `dial jpeg`; no app/service caller | Wire into service/app once byte 6, pacing, orientation verified |
| Display: key images | 72x72 JPEG per key, `02 36` | Partial | `keypad.rs` `JpegPackets` is pure; no live write in any crate (grep of `display_jpeg`/`keypad::` shows none) | Add live `keypad::display_jpeg` behind explicit opt-in |
| Display: GIF/animation | GIF import split into JPEG frames sent one by one | Missing | static analysis "Dial display"; implementation status line 101 (animation unsupported) | Offline frame splitter first |
| Display: image prep | Center-crop square, scale, JPEG quality 1.0 | Done | CLI `image_prep.rs`; app `ArtworkPage.tsx`, `src-tauri/src/artwork.rs` | Compare crop/orientation with official on hardware |
| Display: keypad redraw on folder/page change | Per-key JPEG calls on redraw | Partial | Offline planner and coordinator against a fake sink: `display_sync.rs`, [display sync](masterhub-display-sync.md). No live key write, no wiring into `live_runtime.rs`; service still rejects live schema-2 keypad runs | Live `keypad` writer and sink, then the integration checklist; hardware acceptance |
| Lighting: dial backlight and color | Backlight 0-100, solid color (3 bytes) | Partial | `dial.rs` builders/setters; CLI `dial backlight/color`; not in app or service | Expose in app after color order verified |
| Lighting: base and M003K | Probe notes only (unverified for 1.0.0.113) | Missing | protocol evidence row "M003K/base lighting"; `modules/m0064.rs` has serial only | Analyse renderer/SDK for base lighting |
| Firmware: version query | Firmware info selectors 47/48/49 | Partial | `dial.rs` `read_firmware_info`; CLI `dial info`; M131E only | Extend to other modules once reports known |
| Firmware: update | Update flow exists (image writes blocked while "in progress"); flow itself not analysed | Out of scope | static analysis line 94; macOS plan "Firmware flashing/recovery ... later projects" | Keep excluded; metadata lookup is optional |
| Firmware: published metadata | UUID per module, stable version endpoint | Missing | static analysis "Module discovery"; windows analysis "Firmware metadata"; no network code in app | Optional; app has a no-network posture |
| Profiles: edit, save, folders | Profiles with one folder level (unverified beyond folders) | Done | `store.rs`, `BindingsPanel.tsx`; schema-2 nested folders to L8 | Compare official profile model |
| Profiles: import/export | Not documented for the official app (unverified) | Done (ours) | `ProfileTransfer.tsx`, `src-tauri/src/export.rs` | Hardware-independent |
| Actions | Catalogue now known ([app features](research/masterhub-1.0.0.113-app-features.md)): hotkey chords, launch/close app, open URL, text snippet, media transport, volume, system actions, timer, clock, multi-action, recorded key macro, lighting effect/brightness, profile next/previous, folders/pages, plus OBS, Twitch, Discord, Streamlabs, Adobe presets and Master CTRL bridges | Partial | `Action` enum in `profile.rs`: volume, set volume, mute, shortcut (ASCII), launch app | Open URL, full chords, media/system actions, text snippet and multi-action are the cheapest gaps; see the ranking in the app-features doc |
| App integration: focused-app rules | Absent in 1.0.0.113: profiles change only via a `profile` key action or the start-up chooser | Done (ours, extension) | `src-tauri/src/focus.rs`, [focus rules](masterhub-focus-rules.md); opt-in, Execute stays confirmation-gated | Not a parity gap; real focus-change behaviour needs a macOS check |
| App integration: browser cards | Absent in 1.0.0.113: only `web-url` and `shell.openExternal` | Not in official app | [app features](research/masterhub-1.0.0.113-app-features.md) | Optional extension only; threat model first |
| App integration: flows | Flat multi-action list (`multi-tasking`, fixed 280 ms spacing, not nestable) plus a recorded key macro; no conditional flows | Missing | [app features](research/masterhub-1.0.0.113-app-features.md); no list-valued action in `profile.rs` | Add a multi-action (non-nestable) and consider a macro recorder |
| App integration: user scripts | Absent in 1.0.0.113; only a hard-coded Photoshop automation | Not in official app | [app features](research/masterhub-1.0.0.113-app-features.md) | Optional extension only; sandbox and permission model first |
| Lifecycle: suspend/resume | Hardware mode on suspend; 2 s, then software mode, firmware query, display refresh | Partial | `session.rs`, `power.rs`; session doc "Comparison with the official app" (no firmware query or refresh) | Hardware timing; add refresh after display upload exists |
| Lifecycle: reconnect | Not documented beyond startup (unverified) | Partial | `session.rs` backoff 1/2/4/8/8 s, `needs_confirmation` for Execute runs | Hardware lifecycle gate |
| Lifecycle: multi-module live | Official handles all modules (unverified) | Partial | One service per interface in `session.rs`; feed reads many; README "multi-module live action routing" milestone | Route several modules in one session |
| Platform: Windows and Linux | Official is Windows-only | Out of scope | macOS plan scope; `macos.rs` non-macOS stub; power clock-gap fallback untested | None planned |
| Packaging: installer | WiX Burn installer, Electron | Partial | `tauri.conf.json` targets only `app`; `building-the-desktop-app.md` unsigned | Developer ID signing, notarization, DMG, CI |
| Diagnostics | Not documented for the official app (unverified) | Done (ours) | `diagnostics.rs`; [diagnostic bundle](masterhub-diagnostic-bundle.md); `masterhub-diag` CLI | Use in first hardware session |

## Device discovery and modules

- **Official:** `startBaseDeviceCheck` accepts both PID generations; firmware UUIDs per module come from `TopNav.vue` (static analysis "Module discovery"; additional modules "Discovery").
- **Ours:** all six PID pairs are in `ModuleKind` and `accepts_product_id` (`modules/*.rs`). Hot-plug inventory and rig detection are in `feed.rs`. Diagnostic bundle enumerates descriptors without writing.
- **Gaps:** actual descriptors, interface/usage selection and topology are unverified (hardware-blocked). The official open path takes the first match with no usage filter; ours selects an exact interface, which is deliberately stricter.

## Input decoding per module

| Module | Official behavior (source) | Ours | Remaining |
| --- | --- | --- | --- |
| M131E dial | Queue 0, `01 44`, payload `04 xx 01/00` press/release, `03`/`00` rotation; host long press >= 600 ms | `parse_m131e_report`, `M131EDecoder`, `DialGestureTracker` (`input/m131e.rs`, `dial_session.rs`) | Real payload length, byte 1, physical direction |
| M003K | Opposite direction-bit convention to the dial | `input.rs` knobs with Clockwise/CounterClockwise | Not analysed from 1.0.0.113 source; probe notes only |
| M4315 | Class `34`, selectors `02` and legacy `00` | `input/m4315.rs` | Physical key order |
| M005S | M0 class `2a`, `02 05` | `input/m005s.rs` | Selector `00` conflict in probe notes stays rejected; jitter |
| M032LE | `01 02`, `00` Up, `01` Down, `ff` idle | `input/m032le.rs` | Roller placement, no magnitude established |

Dial function cycling (short click changes function) now exists in the service crate only ([dial functions](masterhub-dial-functions.md)); the CLI, app preview and editor do not use it yet, and a profile can carry it today only by hand. All decoder claims are hardware-pending.

## Display and image upload

- **Dial:** SDK upload exists and a pacing hook exists (`display_jpeg_with_pacing`, `dial.rs`), which the older docs do not mention. Only the CLI (`crates/cooltek-sdk-cli/src/main.rs`, `dial jpeg` with an explicit path) calls it. No service or app path uploads anything; README states sending images is unavailable until verified.
- **Keys:** packet planning only, plus an offline redraw planner and coordinator tested against a fake sink (`display_sync.rs`, [display sync](masterhub-display-sync.md)): diff after navigation, failed writes stay dirty, short writes never retried, full redraw after resume. No code writes key images to a device, and nothing outside that module calls it yet.
- **Open protocol questions (hardware-blocked):** header byte 6 on the dial, inter-chunk pacing/acknowledgement, orientation, accepted JPEG variants, key order.
- **Missing without hardware:** GIF frame splitting, progressive output, BMP, EXIF orientation handling (implementation status lines 34 and 101).

## Lighting and backlight

Dial backlight (0-100) and solid color are implemented as feature reports with CLI preview and explicit write; the color channel order is unverified. There is no app or service control. Base and knob lighting have no implementation and no 1.0.0.113 analysis behind them.

## Firmware

Only the M131E firmware info read exists (`dial.rs`, CLI `dial info`). Update flashing is excluded by roadmap. The official app's update flow was not analysed beyond the flag that blocks image writes during an update.

## Profiles and actions

Profile editing, validation, atomic store, drafts with restore, undo/redo, import/export, binding search and conflict warnings exist (`apps/open-stream-hub/src/edit-history.ts`, `profile-library.ts`, `binding-model.ts`, `draft-store.ts`). Keypad folders go to eight levels, deeper than the official software's one level (keypad folders doc, "Windows reference and hardware boundary"). The action set is small (six types in `Action`), shortcuts are limited to one ASCII key, and actions run only on macOS. The official action catalogue is not documented here (unverified).

## App-integration features

Settled by the [app features](research/masterhub-1.0.0.113-app-features.md) source-map pass. None of the four exists in the official app, so none is a parity target; they are optional OpenStreamHub extensions.

| Feature | Evidence in our repo | Notes |
| --- | --- | --- |
| Focused-app rules | Implemented as an opt-in extension: `src-tauri/src/focus.rs`, [focus rules](masterhub-focus-rules.md); executing sessions still need confirmation for the exact profile shown | Real macOS focus-change behaviour (Cmd-Tab, Spaces, Stage Manager, helper bundle ids) is not verified |
| Browser cards | None; app CSP allows only `self` and IPC (`tauri.conf.json`) | Would need a browser extension or adapter and a display upload path |
| Flows | None; the official app only has a flat multi-action list and a recorded key macro | A non-nestable multi-action in `profile.rs` would match it |
| User scripts | None; README states no script integration | Needs a sandbox and permission model |

## Lifecycle and reconnect

Implemented: sleep/wake observers on macOS, clock-gap fallback elsewhere, reconnect with bounded backoff, device identity by serial, Execute never auto-resumes (`session.rs`, `power.rs`, [session manager](masterhub-session-manager.md)). Gaps against the official sequence: no firmware query and no display refresh after resume; the 2 s delay is copied, not verified. Dock Quit still loses unsaved drafts to disk, mitigated by webview draft restore (README).

## Packaging and signing

`tauri.conf.json` builds only an unsigned `.app` (`targets: ["app"]`), with the sidecar as `externalBin`. `.github/workflows/ci.yml` (macOS arm64 and Intel) and `release.yml` (signs and notarizes only when all six Apple secrets exist, otherwise an artifact labeled UNSIGNED) are on `master` with [release process](release-process.md) notes. No workflow has run on GitHub yet, and signing, notarization and sidecar signing have never been exercised. Intel Mac, Windows and Linux builds are untested.

## Diagnostics

Ahead of the documented official software: passive descriptor/input diagnostic bundle with redaction (`diagnostics.rs`, [diagnostic bundle](masterhub-diagnostic-bundle.md)), `masterhub-diag` record/replay, fixtures. No gap beyond hardware acceptance.

## Prioritized work

### Can be done without hardware

1. ~~N1: Source-map pass for app-level features.~~ Done 7 October 2026: [app features](research/masterhub-1.0.0.113-app-features.md). Still open from it: the `.mhz` profile format, official folder/page limits, whether `clock` exists on keypad tiles, and the native addon and `worker/Ctrl.js`.
2. ~~Update stale docs.~~ Done: implementation status, macOS plan and protocol evidence lifecycle rows.
3. ~~Dial function cycling.~~ Service-crate design and routing done and reviewed; CLI logging, app preview, editor UI and the dial screen remain (checklist in [dial functions](masterhub-dial-functions.md)).
4. ~~Focused-app rules.~~ Implemented as an opt-in extension (not in the official app); needs a real macOS focus check.
5. Multi-action (non-nestable, ~280 ms spacing) and a richer action catalogue (open URL, full chords, media/system, text snippet, profile next/previous), replay-tested. Ranked in the app-features doc.
6. GIF frame splitting in offline artwork prep; EXIF orientation handling.
7. Offline half done: sink trait, fake sink, redraw planner and coordinator ([display sync](masterhub-display-sync.md)). Remaining without hardware: a typed short-write error in the masterhub dial helper, wiring into `live_runtime.rs`, an artwork provider with Back-key depth badges, and the Tauri resume hook.
8. Lighting controls in the app against a fake transport, flagged unverified until color order is known.
9. Release engineering: CI and release workflows exist but have never run; remaining are a DMG target, a real signed dry run with Apple secrets, and the sidecar signing check.
10. Browser cards and user scripts: not in the official app; optional extensions, threat model first, no runtime until needed.
11. Multi-module single-session routing, tested with simulated inventories.

### Needs hardware

1. First hardware session: descriptors, topology, interface selection, serials, physical control order and direction for all modules (hardware-blocked for every Done row).
2. Dial display: byte 6, pacing/acknowledgement, orientation, JPEG acceptance; then key image upload and key order.
3. Color channel order, backlight effect, base/knob lighting commands.
4. Feature-report semantics: reply byte 1 and firmware bytes; firmware query for non-dial modules.
5. Sleep/wake and replug timing: 2 s delay, re-enumeration, serial/path stability, SIGINT-before-sleep.
6. Complete input payload lengths and dial long-press sequences; fader range and jitter.
7. Real macOS action acceptance, permissions and sustained throughput.
8. Official-app Windows captures (USBPcap) for startup, display and profile changes, per the [macOS plan](masterhub-macos-plan.md), "Windows capture procedure".
