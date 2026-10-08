# MasterHUB 1.0.0.113: official OBS actions

Analysis date: 8 October 2026. Purpose: pin down exactly which OBS actions the official Windows MasterHUB 1.0.0.113 offers, which obs-websocket v5 requests each one sends, and which quirks of the official behavior we should copy, fix or drop, before OpenStreamHub adds more than `obs_scene`. Static, text-only reading of the extracted sources; nothing was executed and no OBS instance was contacted. Provenance and hashes are in [app features](masterhub-1.0.0.113-app-features.md#provenance).

Paths are relative to `sources/src/` in the extraction. `App.vue` line numbers are the action executor (`case "<action>":`); `store/module/masterHub.js` l.505-714 is the action catalogue; `plugins/OBSWebSocketSingleton.js` is the connection.

## Connection model of the official app

- One long-lived `obs-websocket-js` connection (`ws://host:port`, password), opened when OBS is enabled, kept open, with event subscriptions. IPv6 hosts are bracketed. The host is user-configurable, not limited to loopback.
- It subscribes to events and mirrors state in `obsState` to draw key state (record on/off, stream on/off, virtual camera, replay buffer, studio mode, mute, volume, filter, source visibility). On connect it reads `GetRecordStatus`, `GetStreamStatus`, `GetVirtualCamStatus`, `GetReplayBufferStatus` and `GetStudioModeEnabled`.
- Consequence for us: our client is deliberately one connection per action, no events (`eventSubscriptions` 0). That keeps every official action that is a plain toggle implementable with the obs-websocket `Toggle*` requests, and means **we have no live key state** (no "recording" glyph on the key). That is a documented gap, not a Phase 1 goal.
- All references to scenes, sources and inputs are by UUID in the 1.0.0.113 code (`sceneUuid`, `inputUuid`, `sourceUuid`), except scene, scene collection, profile and transition names, which are names. UUID addressing needs obs-websocket 5.4+ (OBS 29.1+). Our existing `obs_scene` uses `sceneName`, which works on every v5.

## The 17 actions

`obsList` (`App.vue` l.327-345) and the catalogue agree on 17 actions. "Display" is where the catalogue lets the action be placed; all 17 are keypad-capable, and only `audio-mixer` is also offered on roller, sliders5, dialEncoder and knob3 (`masterHub.js` l.623).

| # | Official action | Request(s) sent | Parameters (profile `func` fields) | Semantics and quirks | `App.vue` |
| --- | --- | --- | --- | --- | --- |
| 1 | `record-start-stop` | `ToggleRecord` | none | Pure toggle, awaited. | l.1818 |
| 2 | `record-continue-pause` | `ToggleRecordPause` | none | Toggles pause; OBS errors if not recording. | l.1823 |
| 3 | `stream` | `ToggleStream` | none | Pure toggle, **not awaited** (failures are silent). Goes live or ends a broadcast. | l.1828 |
| 4 | `virtual-camera` | `ToggleVirtualCam` | none | Toggle, not awaited. | l.1832 |
| 5 | `replay-buffer` | `ToggleReplayBuffer` | none | Toggle, not awaited. | l.1835 |
| 6 | `replay-buffer-save` | `SaveReplayBuffer` | none | Fails in OBS if the buffer is not running. | l.1838 |
| 7 | `scene-collection` | `SetCurrentSceneCollection` | `SceneCollectionTitle` → `sceneCollectionName` | Name match. Switching a collection reloads all scenes. | l.1841 |
| 8 | `scenes` | `SetCurrentProgramScene` | `selectSceneId` → `sceneName` | **Already implemented** as `obs_scene`. | l.1853 |
| 9 | `source-visibility` | `GetSceneItemEnabled`, then `SetSceneItemEnabled` | `visibilityScene` (or `obsVsbGroupItemParentUuid` for an item inside a group) → `sceneUuid`; `visibilitySceneSource` → `sceneItemId` | Always a toggle (read, then write the opposite). Two requests. Item id is OBS's numeric scene item id, not a name. | l.1861 |
| 10 | `obs-profile` | `SetCurrentProfile` | `profileTitle` → `profileName` | Name match. | l.1885 |
| 11 | `audio-mixer` | see below | `TypeID` (`Set`, `Adjust`, `Mute`), `sourceId` → `inputUuid`, `volumnDb`, `stepSize`, `MuteMode` | Three sub-modes; the only action on dials, rollers, faders and knobs. | l.1895 |
| 12 | `media-source` | `GetMediaInputStatus`, then `TriggerMediaInputAction` | `mediaSourceTitle` → `inputUuid`, `mediaActionTitle` | Sub-modes below; also starts a local progress timer that redraws the key. | l.1963 |
| 13 | `studio-mode` | `GetStudioModeEnabled`, then `SetStudioModeEnabled` | none | Toggle (read, write the opposite). | l.2111 |
| 14 | `transition` | `GetStudioModeEnabled`, `TriggerStudioModeTransition` | none | **Bug:** the studio-mode result is the response object, so `=== true` is never true; the final unconditional `TriggerStudioModeTransition` always runs. Net effect: always sends the trigger; OBS rejects it when studio mode is off. | l.2130 |
| 15 | `filter` | `GetSourceFilter`, then `SetSourceFilterEnabled` | `filterSource` → `sourceUuid`, `filterTitle` → `filterName` | Toggle (read, write the opposite). | l.2144 |
| 16 | `screenshot` | `GetCurrentProgramScene`, `SaveSourceScreenshot` | none | Saves a PNG of the current program scene to `%USERPROFILE%\Videos\Screenshot <date time>.png`. Windows path hard-coded. | l.2172 |
| 17 | `scene-transition` | `SetCurrentSceneTransition`, `SetCurrentSceneTransitionDuration` | `sceneTransitionTitle` → `transitionName`, `sceneTransitionDuration` (ms, `parseInt`) | Chooses the transition and its duration; does **not** trigger a transition. | l.2218 |

### `audio-mixer` sub-modes (l.1895-1960)

| `TypeID` | Request(s) | Notes |
| --- | --- | --- |
| `Set` | `SetInputVolume` with `inputVolumeDb = volumnDb` | Absolute dB. |
| `Adjust` | `GetInputVolume`, then `SetInputVolume` with `current + stepSize` | **Bug:** the two clamps are not chained. The second statement overwrites the first, so the lower bound (-100 dB) is lost and the result is only capped at 0. Stepping down below -100 sends a value OBS rejects or clamps. |
| `Mute` | `MuteMode` 1: `ToggleInputMute`; 2: `SetInputMute` true; 3: `SetInputMute` false | Toggle, force mute, force unmute. |

The analog controls use the same inputs differently (not a key action): knob3 (`App.vue` l.3698-3740), roller (l.3957-3985) and dial encoder (l.4281-4303) read `GetInputVolume`, add or subtract `obsAudioMixerAdjustmentUnit` (roller: fixed 1 dB), clamp to a min/max and send `SetInputVolume` in dB. The dial-encoder and knob press can mute (`SetInputMute`, l.3076-3084). These are separate from the key action and belong with our dial and knob function model, not with the key action list.

### `media-source` sub-modes (l.1963-2105)

State is read with `GetMediaInputStatus` (`OBS_MEDIA_STATE_PLAYING`, `PAUSED`, `STOPPED`, `ENDED`), then one `TriggerMediaInputAction` with `OBS_WEBSOCKET_MEDIA_INPUT_ACTION_{PLAY,PAUSE,RESTART,STOP}`:

| `mediaActionTitle` | Rule |
| --- | --- |
| `Play/Pause` | PLAYING → PAUSE; ENDED or STOPPED → RESTART; PAUSED → PLAY |
| `Play/Stop` | PLAYING → STOP; every other state (ended, stopped, paused) → RESTART |
| `Play/Restart` | PLAYING → RESTART; ENDED or STOPPED → RESTART; PAUSED → PLAY |
| `Stop` | always STOP |

The four titles are the `case` labels at l.1972, 2016 and 2046-2090 (read in full). `Play/Stop` has no PAUSED branch, so a paused source restarts instead of resuming.

## What we can match with one-shot connections

Everything above runs as request/response with no event subscription, so a one-connection-per-action client can do all of it. The differences:

| Official behavior | Our approach |
| --- | --- |
| Read-then-write toggles (#9, #13, #15, #11 `Mute` mode 1) | Use OBS's native toggle where one exists (`ToggleInputMute`). For scene item, studio mode and filter there is no `Toggle*` request, so do `Get*` then `Set*` on one connection. Racy only against a second controller, which the official app shares. |
| `ToggleStream` etc. not awaited | Await the response and report failures; the status code is the only signal we get. |
| UUID addressing (`inputUuid`, `sceneUuid`, `sourceUuid`) | Prefer names for portability to OBS 28 (the documented minimum), and for readable profiles. A profile that must survive a rename could use UUIDs, but that needs OBS 29.1+ and a way to pick them. Decide in the model phase. |
| Key state glyphs from events | Not planned. Needs a persistent subscribed connection, a display path and hardware-verified key images. |
| `screenshot` to a Windows path | If done, write under a fixed macOS directory chosen by the app, never a profile-supplied path. Candidate to defer: it writes a file, which none of the other 16 do. |
| Remote OBS host | Out of scope; loopback only stays (see the `ObsEndpoint` rules). |

## Official bugs and quirks we should not copy

1. `transition` ignores its own studio-mode check (always triggers).
2. `audio-mixer` `Adjust` loses the lower clamp.
3. `stream`, `virtual-camera`, `replay-buffer`, `replay-buffer-save` do not await, so errors vanish.
4. `screenshot` hard-codes `\Videos\` and Windows separators.
5. `scene-transition` is a "set" action; users may expect it to trigger one. Keep the two ideas separate in our model (`set transition` and `trigger transition`).

## Safety classification (input to the plan, not a decision)

| Risk | Actions | Why |
| --- | --- | --- |
| Changes what a live audience sees or hears, or ends/starts a broadcast | `stream`, `record-start-stop`, `scene-collection`, `obs-profile` | Going live or ending a stream cannot be undone from the controller; collection or profile switches reload everything. |
| Changes the live program output | `scenes` (done), `source-visibility`, `scene-transition`, `transition`, `filter`, `audio-mixer`, `studio-mode` | Same class as `obs_scene` today: reversible, but visible or audible immediately. |
| Local or auxiliary | `record-continue-pause`, `virtual-camera`, `replay-buffer`, `replay-buffer-save`, `media-source` | Output toggles that do not by themselves publish to an audience. |
| Writes a file | `screenshot` | Only OBS action that writes to disk. |

## Corrections to the first plan

- The official list has **17** actions (one already done), not about 18.
- **Missing from my first grouping:** `scene-collection`, `obs-profile`, `filter`, `media-source` and `screenshot` are official actions I had not listed.
- **Not official, so dropped from the plan:** `obs_preview_scene` and a dedicated `obs_volume` step action. Preview scene selection does not exist in 1.0.0.113. Volume is the `audio-mixer` `Set` and `Adjust` modes.
- Record, stream, replay buffer and virtual camera have **no start-only or stop-only variants** in the official app. Only toggles exist. Explicit start/stop would be an extension of ours (safer for multi-action sequences, because a toggle in a sequence is stateful).
- `studio-mode` and `transition` are toggle and trigger only; no explicit on/off.

## Open questions

1. Whether analog controls should drive OBS volume (the `obsSourceId`, `obsAudioMixerAdjustmentUnit` fields in imported `.mhz` files point at it). This is the dial and knob model, so it needs a separate decision.
2. How the progress timer (`startOBSMediaPlayTimer`) draws on the key. It is display-only and out of scope here.
3. Whether `sceneItemId` for groups (`obsVsbGroupItemParentUuid`) needs a separate field, or a nested scene reference, in our model.
4. Keypad-only versus multi-action: all 17 are allowed in the 15-key multi-task display (`15keys-multi-tasking`).

## Reproduction

```sh
cd ~/Documents/masterhub-analysis/1.0.0.113/app-analysis/sources/src
sed -n 505,714p store/module/masterHub.js          # catalogue
sed -n 327,345p App.vue                            # obsList
sed -n 1818,2240p App.vue                          # executor
cat plugins/OBSWebSocketSingleton.js               # connection and events
grep -n "obsAudioMixerAdjustmentUnit\|obsSourceId" App.vue   # analog controls
```
