# MasterHUB 1.0.0.113: the `.mhz` profile format

Analysis date: 7 October 2026. This answers the open item "`.mhz` schema" in [app features](masterhub-1.0.0.113-app-features.md): what an official MasterHUB profile file contains, how the official app reads and writes it, and how much of it OpenStreamHub could import into profile schema 2 without putting the user at risk. The analysis was static and text-only. Nothing from the package was executed, and no `.mhz` file was opened by the official app or by our code.

**Implementation status (7 October 2026):** import is implemented in `apps/open-stream-hub/src-tauri/src/mhz/` (archive reader, bounded JSON, mapper) with the dry-run-only flag in `import_flags.rs`; see [Implementation notes](#implementation-notes) and the [user guide](../importing-masterhub-profiles.md). It is tested with synthetic files only. Export is not implemented.

Verdicts use **Present** (seen in the sources or the bundled files), **Absent** (searched for and not found), and **Unverified** (inferred, not confirmed).

## Provenance and method

- Sources: the extraction reused from the earlier analysis, at `~/Documents/masterhub-analysis/1.0.0.113/app-analysis/` (`sources/src/` plus the main-process `background.js`). Its hashes are listed in [app features](masterhub-1.0.0.113-app-features.md#provenance). Paths below are relative to `sources/src/` unless they say otherwise.
- Bundled templates, found in MSI cab 4 at `payload/raw/cab4/`. They are loaded by `components/modal/CreateProfileModal.vue` l.114-129 through `getProfileTemplatePath`:

  | File | Size | SHA-256 |
  | --- | --- | --- |
  | `masterHubConfig_profile_Default.mhz` | 1,524,940 B | `6416bb2e0c4111ddc06ef9320d9626877b2ea5ceaba5561aeae5b8f5370e8502` |
  | `masterHubConfig_profile_Photoshop.mhz` | 1,010,076 B | `91ce3df75b0a259de5d8b551cf7182f10ed062536a445c1bcd04463f9da26b02` |

- Method: `file`, `xxd` and `unzip -lv` on the templates. The single entry of each was copied out with `unzip -p` into a scratch directory outside the extraction. Small `python3 -I` scripts, also kept outside the extraction, walked the JSON and read the JPEG headers. The sources were read with grep and sed. Only identifiers and line references are quoted here.

## Verdict summary

| Question | Verdict | Evidence |
| --- | --- | --- |
| Container is a standard ZIP | Present | Magic `PK\x03\x04`, version 2.0, deflate, general-purpose flags `0x0008` (data descriptor, streamed writer). Written by an archiver-style `zip` writer at zlib level 9 (`background.js` `compressProfiles`) |
| Payload is plain JSON | Present | Single entry, 7-bit ASCII JSON (Default 4,388,568 B, Photoshop 2,446,870 B). Read with `JSON.parse` (`lib/profileHelper.js` l.28-40; `store/index.js` l.1557-1566) |
| Encryption or obfuscation | Absent | ZIP flag bit 0 (encryption) is clear and there is no AES extra field. The JSON is cleartext. No cipher or decode step on the import path |
| Signature, MAC, or embedded checksum | Absent | Nothing beyond the ZIP's own per-entry CRC-32. The Master ID upload computes a CRC-32 of the whole file and sends it as a `checksum` HTTP header (`components/Common/TopNav.vue` l.2365-2378). That is transport integrity, not a signature |
| Schema or format version field | Absent | No `version` key in either template, and none read during import |
| Embedded images | Present | Node `Buffer` objects serialized as JSON byte arrays (`{"type":"Buffer","data":[…]}`) plus one base64 JPEG string per key. Optional `uploadImages/` and `gifPools/` ZIP folders |
| Profiles are tied to a device serial | Present, remapped on import | The file names the base serial and keys every module section by module serial. A file import remaps the sections by position onto the connected modules of the same type (see [Device serials](#device-serials)) |
| Import executes anything | Absent at import time, Present at key press | Import only stores the data. The `web-url` and `app-files` actions later run `child_process.exec` with interpolated strings (`App.vue` l.1394-1412) |

## Container

A single-profile export (`saveConfigFile`, `store/index.js` l.1331-1381) is a ZIP with:

| Entry | When present | Content |
| --- | --- | --- |
| `profile<id>_<baseSerial>` (no extension) | Always | The profile JSON. Its name is the basename of the per-serial profile file (`getProfilePath`, `lib/profileHelper.js` l.19-21) |
| `uploadImages/<basename>` | When keypad tiles reference uploaded images through `gifPath` | Image files copied from `%APPDATA%/MasterHub/uploadImage` (`getUploadImageFromProfile`, l.47-92) |
| `gifPools/<hash>` | When keypad tiles carry `gifFileHash` | GIF files from `%APPDATA%/MasterHub/gifPool` (`getGifPoolFromProfile`, l.93-129) |

Both templates contain only the JSON entry: `profile2_<serial A>` (Default) and `profile2_<serial B>` (Photoshop). There is no ZIP comment and no extra entry.

A multi-profile export writes one `.mhz` per profile and then zips those into `masterHubConfig_profiles.zip` (`store/index.js` l.47 and l.1378). That outer file is a ZIP of ZIPs and is not itself a `.mhz`.

Export file name: `%APPDATA%/MasterHub/masterHubConfig_profile_<label>.mhz`, where `<label>` is the profile label, not sanitized (`store/index.js` l.46, l.1346).

### How the official app opens a `.mhz` (`background.js` `extractProfiles`)

1. Runs `7z.exe x -y "<path>" -o"<APPDATA>/MasterHub/profileExtract"` through `child_process.exec`. `7z.exe` sits next to the app in `resources\`. The whole archive is extracted, with no size, entry-count or entry-type checks.
2. Lists the extract directory. An entry matching `/^profile[0-9]+_(.*)/` becomes the profile to read; if several match, the last one in `readdir` order wins. Every file in `uploadImages/` is copied into `uploadImage/`, and every file in `gifPools/` into `gifPool/`. Existing files are skipped and nothing is overwritten. Contents are not validated.
3. The renderer reads the profile with `readFileSync` and `JSON.parse`, then sends `cleanExtractProfiles`, which deletes and recreates the extract directory.

Zip-slip protection is whatever this build of 7-Zip does by default (Unverified for this `7z.exe` version). There is no decompression-bomb limit (Absent).

## Top-level schema

The order and set of keys match what `syncConfig` writes (`store/index.js` l.1446-1475):

| Key | Type | Meaning |
| --- | --- | --- |
| `serialNumber` | string | Serial of the base (`MHBS…`) the profile belongs to |
| `profileId` | number | The profile's local numeric id. Import requires it to be numeric but does not reuse it |
| `profileName` | string | Display name. Import also accepts the key `profilename` |
| `knobsList` | object: serial → knob set | M003 three-knob modules |
| `keypad` | object: serial → pages → tiles | M4315 15-key modules |
| `keypadBtnConfig` | object: serial → UI state | `currentFolderIndex`, `currentPage`, `isNew`, `pageList`, `showContextMenu`, `totalPage`, `viewMode` |
| `colorPickerFavoriteColors` | array | Colour-picker favourites (UI) |
| `rollerBlock` | object: serial → rollers | M002/M032 two-roller modules |
| `dialEncoder` | object: serial → dial | M131 IPS dial |
| `sliders5List` | object: serial → index → fader | Five-fader module |
| `touch46s`, `touch50s` | object | M4600 and M5000 touch modules (empty in both templates) |
| `deviceLightingEffect` | object | Lighting by module type (`basex64`, `m131`, `m002`, `m003`, `m4300`, `m4315`, `slider`, `wholeDevice`), then by serial: `effect`, `color` (hex), `brightness`. Both templates also contain a stray `undefined` serial key |
| `devices` | object | Snapshot of the module inventory (`basex64`, `m003`, `slider` lists with `serialNumber`, `parentSerialNumber`, `moduleName`, `portNumber`, `portNumbers`, `rootAddress`, lighting) |
| `filtered` | boolean | Set to `true` by `syncConfig`. When loading a stored profile (not a file import), it skips the "is this module connected" filter |
| `iconLibrarys` | optional | Written by `saveProfileFileConfig` (l.1315-1328). Absent from both templates |

## Per-module sections

### Keypad (`keypad[serial][page][slot]`)

- `keypad[serial]` is an array of pages. Each page is an array of 15 tile objects, and the tile's `id` (0-14) is the slot. `keypadBtnConfig[serial].totalPage` and `pageList` describe the linear pages.
- Default tile fields come from `defaultKeypadObj` in `store/module/keypad.js` l.20-70. The fields that matter:
  - `btnFunc`: the action type id (see [Action catalogue](#action-catalogue-and-how-actions-map)).
  - Action parameters: `hotkey {key[], code[]}`, `hotKeySettingMode` (`hotkey` or `macro`), `hotkeyMacro {keys[]}`, `url`, `app`, `appFilePath`, `actionAppMode` (`Open` or `Close`), `content` (text), `dropdownOption` (option key), `profileId`, `volumeAction`, `direction`, `countDownSecond*` (timer), `lightingEffectColor`, `actionList[]` (multi-task steps), `folder[]` (child tiles), `folderIndex`, `btnType`, `scene`, `psBrushOpacity`, `psBrushFlow`.
  - Appearance: `title`, `enableTitle`, `displayType` (`image` or `color`), `btnColor`, `btnIconColor`, `btnTextColor` (`{r,g,b}`), `btnFontStyle`, `btnFontSizeStep`, `btnAlignText`, `fontWeight`, `imageType`, `iconPath`, `logo`.
  - Images: `imageBuffer`, `bigImageBuffer`, `originalImageBuffer`, `imageIcon`, `reverseImageIcon`, `frames[]`, `smallFrames[]`, `gifPath`, `gifFileHash`, `onGifFileHash`.
  - Runtime state that gets saved anyway: `intervalId`, `count`, `countDownInterval`, `endTime`, `currentPage`, `currentSettingMode`, `viewMode`.
- **Placeholder fields:** every tile carries `volumeAction: "volume-up"` and `direction: "up"` whatever its `btnFunc`. An importer must read `volumeAction` only when `btnFunc` is `volume-control`.
- **Folders:** a `new-folder` tile holds its children in `folder[]` (`App.vue` l.1286-1310). The import code (`store/index.js` l.1683-1720) and both image collectors walk exactly one level of `folder[]`. That matches the one-level depth recorded in [keypad folders](../masterhub-keypad-folders.md). Whether `folder[0]` is the generated Back tile (`back-to-master.png`) is Unverified.
- **Multi-task steps:** `actionList[]` rows have `action`, `hotkey`, `hotKeySettingMode`, `hotkeyMacro`, `app`, `file`, `url`, `content`, `profileId`, `direction`, `dropdownOption`, `volumeAction`, plus UI fields (`id` UUID, `name`, `label`, `taskName`, `icon`, `display[]`, `onImage`, `doneImage`). They run through the same `executeKeypadFunction` switch, 280 ms apart (`App.vue` l.4472-4480). Which field an `app-files` step reads its path from (`file` or `appFilePath`) is Unverified.
- **Recorded macros:** `hotkeyMacro.keys[]` rows are `{uid, action: "Press"|"Release", name, code, seconds}`, where `seconds` is the delay in milliseconds, clamped to 0-9999 by the recorder (`components/modal/RecordMacroModal.vue` l.136-193). The recorder stops at 998 rows. Replay (`App.vue` l.1729-1741) enforces no count or pairing, so a crafted file can leave keys held down.
- **Hotkeys:** `hotkey.key[]` holds key names (`Control`, `Shift`, `Alt`, `Win`, `LeftSuper`, `Meta`, letters, `Enter`, `PageUp`, `Left`, `Minus`, `Equal`, `Add`, `-`, `+`, `[`, `]`, …). These are what actually gets executed: `nutKeyCode` maps the names to nut-js codes (`mixins/nutMixin.js` l.71-102). `hotkey.code[]` is unreliable. It mixes Windows virtual-key codes (`Win`=91, `I`=73) with nut-js enum values (`Shift`=56, `S`=31, `PageUp`=95 in the Default template), so an importer must use `key[]` and ignore `code[]`.
- The Default template also has a second keypad keyed `CT99999999999` with 15 empty tiles. No source file references that serial (Unverified meaning, probably a development placeholder).

### Knobs (`knobsList[serial]`)

`rotateR`, `rotateG`, `rotateB` are the three knobs, with `id` 0, 1 and 2. Each has `btnFunc`, `btnName`, `left` and `right` (`{dropdownOption, hotkey, volumeAction}`), and `functionList[]` + `functionIndex`. `functionList[]` repeats the same fields plus `langkey`, `knobEnableTitle` and `knobStreamlabsAudioSourceId`. The top-level knob fields mirror `functionList[functionIndex]`. Knob press bindings are not in this shape (Absent from both templates).

### Dial (`dialEncoder[serial]`)

- Function cycling: `functionList[]` + `functionIndex`. Each function has `btnFunc`, `btnName`, `langkey`, `left` and `right` sides, and an `imageBuffer`.
- Display: `lcdDisplayMode` (`Default`, `Image`, `Clock`, plus the stat modes in `types/constants/lcdDisplayMode.js`), `imageBuffer`, `originalImageBuffer`, `bgImageBuffer`, `bgColorHex`, `textColorHex`, `displayImage`, `label`, `dialEncoderEnableTitle`.
- GIF: `frames[]` (240x240 JPEG Buffers; 63 frames in Default) with `framesDelay[]` (ms, 80 each), `imageName`, `imageType: "gif"`, `gifPath`, `filepath`.
- Integration fields: `obsSourceId`, `obsAudioMixerAdjustmentUnit`, `dialEncoderStreamlabsAudioSourceId`, `dialEncoderStreamlabsAudioAdjustmentUnit`, `volumeControlProcessId`, `dialEncoderIsTimerSoundOn`.

### Rollers (`rollerBlock[serial]`)

`left` and `right` rollers, each with `btnFunc`, `btnName`, `up` and `down` (`{dropdownOption, hotkey, volumeAction}`), `enableTitle`, `sourceId`, `rollerStreamlabsAudioSourceId` and `volumeControlProcessId`.

### Faders (`sliders5List[serial]["0".."4"]`)

`btnFunc`, `btnName`, `value`, `sourceId`, `streamlabsAudioSourceId` and `volumeControlProcessId` (per-application volume), plus `slider {dropdownOption, hotkey, volumeAction, volumeRange[2], streamlabsVolumeRange[2]}`. The Default template has five unbound faders with `volumeRange` `[-96, …]`, so the dB semantics are Unverified.

## Embedded images and GIFs

| Field | Encoding | Size seen |
| --- | --- | --- |
| Keypad `imageBuffer` | JPEG in a JSON `Buffer` array | 72x72 (the key display size) |
| Keypad `bigImageBuffer`, `originalImageBuffer` | JPEG in a `Buffer` array | 200x200 |
| Keypad `imageIcon` | base64 JPEG string, no `data:` prefix | 200x200 |
| Keypad `frames[]`, `smallFrames[]` | JPEG `Buffer`s per GIF frame | Empty in both templates |
| Dial `imageBuffer`, `functionList[].imageBuffer`, `frames[]` | JPEG `Buffer`s | 240x240 |
| `uploadImages/`, `gifPools/` ZIP folders | Raw files, referenced by `gifPath` and `gifFileHash` | Absent from both templates |

- Import turns each `{type:"Buffer",data}` back into a `Buffer` with `Buffer.from` (`store/index.js` l.1640-1720 for keypad and folder tiles, about l.1886-1914 for the dial).
- Size cost: a byte stored as a JSON decimal array takes about 3.5 bytes. Images make up most of the 4.4 MB Default JSON.
- **Privacy leak in the official template:** the Default dial carries `gifPath` `C:\Users\<redacted user>\AppData\Roaming\MasterHub\uploadImage/<n>.gif`, the template author's Windows user name (redacted here). Its `devices` list carries real serials of a different base (`<serial B>`) from the one named in `serialNumber`. User exports leak the same kinds of data: serials, local paths and app paths in `appFilePath`.
- `getUploadImageFromProfile` builds `uploadImage/<gifPath>` and takes only the part after the last `/`. A `gifPath` containing `..\` therefore points outside the folder on Windows, and a re-export could pick up an arbitrary file (plausible, Unverified). Dial GIFs are never collected for export (`getUploadImageFromProfile` walks only `keypad`). Only their decoded frames travel, inside the JSON.

## Device serials

- On disk, profiles are stored one file per base and profile: `<DATA_DIR>/profile<id>_<baseSerial>` (`lib/profileHelper.js` l.19-21, l.28-46).
- Inside a file, every module section is keyed by module serial (`MHKD15…` keypad, `MHKB03…` knobs, `MHDD01…` dial, `MHRR02…` rollers, `MHFD05…` faders).
- **File import does not require matching serials.** `importConfig` (`store/index.js` l.1537-2027) handles each section as follows:
  - Keypad: kept for every connected keypad whose serial matches. If none matches, the file's keypads are assigned to the connected keypads by position (l.1733-1770).
  - Knobs, rollers, dial, faders and touch modules: always assigned by position. The i-th entry in the file goes to the i-th connected module of that type under the current base (l.1810, 1835, 1861, 1883, 1938, 1985).
  - The base serial in the file is only passed to `filterConfigdataByParentSerialNumber` (l.2147).
- The result: an exported profile works on another person's hardware, and the serials in it are identifying data rather than real constraints.

## Versioning and checks on import

| Check | Verdict | Where |
| --- | --- | --- |
| File extension | Present, UI only | Open dialog filter `extensions: ['mhz']` (`components/modal/EditProfileModal.vue` l.383-392). `.mhz` suffix check only for the Master ID upload (`TopNav.vue` l.2352) |
| ZIP well-formedness | Delegated to `7z.exe`; a failed exec returns `false` | `background.js` `extractProfiles` |
| JSON parses | Present | `store/index.js` l.1562-1566 |
| `profileId` numeric, name non-empty | Present | l.1569-1573 (rejects when `isNaN(profileId)`, or the name is `''` or missing) |
| Name collision | Present: appends `_copy` or `_copyN` | l.1577-1586 |
| Overwrite an existing profile | Absent: always creates a new profile; the new local id is max+1 | `createProfile` l.982-997 |
| Schema version, field types, action allow-list, size limits | Absent | No validation before the data is committed to the store |
| Signature or authenticity | Absent | — |

## Action catalogue and how actions map

Action type ids come from `featureClassList` in `store/module/masterHub.js` (l.193-2632+). The option key the user picks is stored in `dropdownOption`. The option tables are in `masterHub.js`; that `dropdownOption` holds the key is inferred from the templates (knob and dial `left` hold 1, `right` hold 0).

| Group | `btnFunc` / `action` ids (line in `masterHub.js`) | Options |
| --- | --- | --- |
| Master HUB | `multi-tasking` 201, `lighting-effect` 215, `profile` 238, `brightness` 252, `timer` 266, `mh-zoom-in-out` 275 | lighting 0-9 effects, 10 next, 11 previous; profile 0 next, 1 previous |
| Navigation | `new-folder` 296, `create-new-page` 305, `previous-page` 314, `next-page` 321 | |
| System | `undo-redo` 337, `switch-desktop` 351, `media-fast-back-forward` 365, `media-track-control` 387, `volume-control` 404, `play-pause` 420, `screen-capture` 433, `zoom-in-out` 442, `scroll-up-down` 456, `web-url` 470, `app-files` 479, `text` 488, `hot-key` 497 | undo 0, redo 1; media 4 play/pause, 5 stop, 2 previous, 3 next; volume 0 up, 1 down, 2/3 mute toggle |
| OBS | `record-start-stop`, `record-continue-pause`, `stream`, `virtual-camera`, `replay-buffer`, `replay-buffer-save`, `scene-collection`, `scenes`, `source-visibility`, `obs-profile`, `audio-mixer`, `media-source`, `studio-mode`, `transition`, `filter`, `screenshot`, `scene-transition` (522-705) | |
| Twitch | `chat-message`, `clear-chat`, `followers-chat`, `sub-chat`, `emote-chat`, `slow-chat`, `shield-mode`, `play-ad`, `stream-game-title`, `create-stream-marker`, `create-clip`, `open-last-clip` (733-854) | |
| Master CTRL | `ctrl-scenes`, `ctrl-lighting-profile`, `ctrl-cooling-profile` (872-896) | |
| Streamlabs | `streamlabs-record`, `-stream`, `-scenes`, `-source-visibility`, `-audio-mixer`, `-studio-mode`, `-transition` (925-997) | |
| Discord | `discord-mute`, `-deafen`, `-voice-channel`, `-text-channel`, `-set-audio-device`, `-voice-input-mode`, `-user-volume-control`, `-output-volume-control`, `-input-volume-control` (1025-1118) | |
| Adobe presets | `ps-*` (1145-1951), `ai-*` (1979+), Premiere group (2632+) | Each preset stores a `hotkey`; knob-type presets store `left` and `right` hotkeys |

There is no separate `clock` key action. `clock` appears only in `unsupportMutitaskingList` (l.181-190), and the dial's clock is `lcdDisplayMode: "Clock"`. Keypad clock stays Unverified.

How the official app executes the actions that matter for safety:

- `web-url`: `exec('explorer "<url>"')`, adding `https://` when the string does not contain `https` anywhere (`App.vue` l.1394-1403). The URL is interpolated into a shell command line, so an imported URL containing `"` can inject commands (plausible, Unverified on Windows).
- `app-files` Open: `exec('"<path.normalize(appFilePath)>"')` (l.1405-1408), which runs any executable path, with the same quoting weakness. Close: `closeApp` IPC to `killApp` (`background.js`; internals Unverified).
- `play-pause`: plays a local audio file `file:///<appFilePath>` (l.1413+).
- `text`: `copyText` IPC, Ctrl+V, optional Enter (`afterEnter`) (l.1778-1790).
- `hot-key`: nut-js key press, or replay of a recorded macro (l.1717-1745).
- Hotkey-backed presets: on a key, `undo-redo`, `switch-desktop`, `media-fast-back-forward`, `media-track-control`, `zoom-in-out`, `mh-zoom-in-out` and most `ps-*` presets press whatever `hotkey.key[]` is stored on the tile (`App.vue` l.1596-1634). `dropdownOption` only decides which hotkey the editor fills in. An importer can map by `dropdownOption` (the intent) or by the stored hotkey (what Windows actually pressed). The mapping below uses the intent where we have a matching action.
- `scroll-up-down` dispatches `executeScroll` up or down by `dropdownOption` (l.1765-1769). The stored `PageUp`/`PageDown` hotkey is not used.
- Keypad `volume-control` passes `volumeAction` and `volumeControlProcessId` to `adjustVolume` (l.1770-1776). A non-zero process id means per-application volume.
- Adobe actions that call `run-photoshop-script` (for example `ps-flatten-image`, `ps-tool-auto-masking-tool`) send fixed script names from the code (`App.vue` l.3017-3041). No script text comes from the profile, so profile-borne scripts are Absent.

## Mapping to OpenStreamHub profile schema 2

Target: `Profile` in `crates/cooltek-sdk-service/src/profile.rs` (`schema_version` 2, `name`, flat `bindings`, `keypad` pages, optional `dial_functions`), with actions as documented in [the action catalogue](../masterhub-actions.md). Legend: **Lossless** (same behaviour), **Lossy** (imported with a documented change, flagged for review), **Unsupported** (imported as an unbound slot and listed in the report). **Dangerous** marks inputs that must never be imported as executable bindings.

### Structure

| `.mhz` field | Schema 2 target | Verdict |
| --- | --- | --- |
| `profileName` | `name` (trimmed, control characters removed, at most 128 bytes, with an "(imported)" suffix) | Lossy (renamed) |
| `profileId`, `serialNumber`, `devices`, `keypadBtnConfig`, `filtered`, `colorPickerFavoriteColors`, `iconLibrarys` | Dropped | Unsupported (UI state, identifiers) |
| Module serial keys | No `serial_number` selector (matches any serial). This follows the official positional remap and avoids storing foreign serials | Lossy |
| More than one module of a type in the file | First entry only. The rest are reported | Lossy |
| `keypad[s][0]` (page 0) slots 0-14 | `keypad.pages[root].slots[id]` | Lossless (layout) |
| `new-folder` tile and `folder[]` | `KeypadTile::Folder` plus a child page. The child's slot 0 stays null for Back, and a non-empty `folder[0]` other than the Back tile is reported | Lossy |
| Pages 1..n (`next-page`, `previous-page`, `create-new-page`) | Child pages chained from a `next-page` tile as Folder tiles, if the tree rules allow it (one parent, depth at most 8, at most 64 pages). Otherwise the page is reported. `previous-page` is dropped, because Back covers it | Lossy |
| `title` | `KeypadTile::Action.label` (at most 128 bytes; empty becomes "Key N") | Lossless |
| Colours, fonts, `displayType`, `enableTitle` | Dropped | Unsupported (no appearance in schema 2) |
| Images (`imageBuffer`, `imageIcon`, `frames`, dial frames, `uploadImages/`, `gifPools/`) | Dropped. A later asset importer would need its own JPEG/GIF decoder limits | Unsupported |
| Knobs `rotateR/G/B` (`id` 0-2) `right` and `left` | `bindings` on `m003k` control `id`, `right` → `clockwise`, `left` → `counter_clockwise` | Lossy (direction mapping Unverified) |
| Knob `functionList` with more than one entry | First (current) function only | Lossy |
| Dial `functionList[]` | `dial_functions[]` (name from `btnName`, at most 64 bytes, at most 16 functions), `right` → `right`, `left` → `left` | Lossy (no images; direction labels as in `Gesture`) |
| Dial `lcdDisplayMode`, display fields | Dropped | Unsupported |
| Rollers `left`/`right` `up`/`down` | `m032le` control 0/1 with `up`/`down` | Lossy (roller order Unverified) |
| Faders `sliders5List` `volume-control` | `m005s` control i with `volume_from_position` | Lossy (`volumeRange` and per-app `volumeControlProcessId` are ignored) |
| Faders with any other function | Unbound | Unsupported |
| `deviceLightingEffect` | Dropped | Unsupported (lighting is pending hardware) |

### Actions

| Official action | Schema 2 action | Verdict and condition |
| --- | --- | --- |
| `hot-key` (`hotKeySettingMode` `hotkey`) | `shortcut` | Lossy. Only when `key[]` has exactly one non-modifier key that is ASCII alphanumeric, space, or one of our named keys (`Enter`→`return`, `Left`→`arrow_left`, `PageUp`→`page_up`, `F5`→`f5`, `Escape`, `Tab`, `Delete`, `Home`, `End`, …). Modifiers: `Control`/`Ctrl`→`control`, `Alt`→`option`, `Shift`→`shift`, `Win`/`LeftSuper`/`Meta`→`command`. Always flagged "Windows shortcut, review", because Ctrl-based Windows shortcuts are usually Cmd on macOS. The importer must not silently swap Control for Command. Punctuation keys (`-`, `+`, `[`, and named `Minus`, `Equal`, `Add`) or several non-modifier keys: Unsupported. `code[]` is ignored |
| `hot-key` (`macro`) | none | Unsupported (no macro recording; replay can leave keys held) |
| `web-url` | `open_url` | Lossy. Only if the URL passes `validate_url` unchanged. The official `https://` prefixing is not reproduced, and a bare host is rejected and reported. Flagged in the risk summary |
| `text` | `text` (`content` → `text`, `afterEnter` → `press_enter`) | Lossless when it passes our text rules (at most 2048 bytes, no invisible characters), otherwise Unsupported. **Dangerous** when `press_enter` is set or the text contains a newline, because pasted into a terminal it runs commands. Import it, but mark it high-risk |
| `multi-tasking` | `multi_action` | Lossy. Only if every step maps and there are 2-8 steps; one step becomes a plain binding. If any step is Unsupported or Dangerous, the whole tile is left unbound, so a partial sequence never runs with different meaning |
| `profile` | `profile` `{direction}` | Lossy. `dropdownOption` 0 → `next`, 1 → `previous`. The official runtime selects `func.profileId` when it is set (`App.vue` l.1391); that id refers to the exporting machine's profiles and is dropped and reported |
| `volume-control` (key) | `volume` or `toggle_mute`, chosen by `volumeAction` (`volume-up`, `volume-down`; the mute value is Unverified) and checked against `dropdownOption` (0 up, 1 down, 2/3 mute) | Lossy. Step size is not in the file; use a fixed default (for example ±5) and report it. A non-zero `volumeControlProcessId` (per-app volume) is Unsupported |
| `volume-control` (knob, dial or roller sides) | `volume {delta: ±N}` per side | Lossy (step size) |
| `media-track-control` | `media` (4 `play_pause`, 5 `stop`, 2 `previous_track`, 3 `next_track`) | Lossless in the profile. Keys run as macOS media keys; `stop` reaches only a running Apple Music or Spotify |
| `undo-redo` | `system undo` / `redo` (by `dropdownOption`) | Lossless in intent. Windows actually presses the stored hotkey |
| `screen-capture` | `system screenshot` | Lossy (Windows region snip vs whole screen to the clipboard). Flagged |
| `app-files` (Open) | none | **Dangerous, Unsupported.** It launches an arbitrary Windows executable path through a shell. Never converted to `launch_app`, never guessed into a bundle id. The path is kept in the report only, redacted to its basename |
| `app-files` (Close) | none | Unsupported (kills a process by path) |
| `play-pause` (audio file) | none | Unsupported (local file path) |
| `switch-desktop`, `zoom-in-out`, `scroll-up-down`, `mh-zoom-in-out`, `media-fast-back-forward` | none | Unsupported (stored hotkeys such as Ctrl+Win+Left are Windows-specific) |
| `timer`, `brightness`, `lighting-effect` | none | Unsupported |
| OBS, Twitch, Streamlabs, Discord, Master CTRL ids | none | Unsupported. Twitch `chat-message` carries user text that would be posted to a network service; it is dropped and reported |
| `ps-*`, `ai-*`, Premiere presets with a stored `hotkey` (keypad) or `left`/`right` hotkeys (knobs, dial, rollers) | `shortcut` per side | Lossy, under the same chord rule as `hot-key`, flagged "application preset" |
| Presets that call `run-photoshop-script`, or need `psBrushOpacity`/`psBrushFlow` loops | none | Unsupported |
| Unknown `btnFunc` | none | Unsupported. Never ignored silently |

Expected result for the bundled templates (by inspection, not by running an importer):

- **Photoshop:** all 15 key tiles map to Lossy shortcuts (`Control+n`, `Control+o`, `Shift+Control+n`, `b`, `e`, `Control+x`, `Control+c`, `Control+v`, `t`, `v`, `r`, `c`, `Shift+Control+Alt+s`, `Shift+Control+s`). All knob and roller sides use punctuation keys (`Shift`/`Control` with `-`/`+`, and `[`/`]`) and are left unbound. The dial `ps-undo-redo` has no stored hotkey. It is left unbound, or mapped by name to `system undo`/`redo` (Lossy) if we decide to trust the id.
- **Default:** 2 `app-files` tiles are Dangerous and left unbound (their paths are empty in the template). 2 `web-url` tiles become Lossy `open_url`. 3 `hot-key` tiles (`Win+I`, `Win+E`, `Win+P`) become Lossy `command` shortcuts that mean nothing useful on macOS. The three-step `multi-tasking` tile becomes a Lossy `multi_action` of the same kind (`Win+X`, `U`, `Enter`, the Windows sign-out menu). Flag it, or leave it unbound as Windows-only. `switch-desktop`, `timer`, both `lighting-effect` tiles and `next-page` (no page 1 exists) are left unbound. `screen-capture` becomes Lossy `system screenshot`. Knob 1 `brightness`, knob 0 `mh-zoom-in-out` and both rollers (`zoom-in-out`, `scroll-up-down`) are left unbound. Knob 2 and the dial `volume-control` become Lossy volume steps. The five faders are unbound in the file and stay unbound.

## Recommended safe import design

Goal: a `.mhz` file is untrusted input from a stranger. Importing one must never run anything, never write outside our profile store, and never hide what was dropped.

1. **Always a new profile.** Import never overwrites or merges. It creates a new schema-2 profile with a new id, named `<profileName> (imported)` and deduplicated like our other copies. Import provenance (source file name, SHA-256, report) and the **dry-run-only** state live in the app's library metadata, not in the profile JSON, because `Profile` uses `deny_unknown_fields`. The profile can run only in dry run until the user opens it in the editor and explicitly clears the flag. The normal Execute confirmation and risk disclosure still apply afterwards.
2. **Never auto-execute.** Import does not start a session, switch the active profile or trigger replay. Imported `profile` actions follow the normal session-manager rules.
3. **Read the archive in memory, never extract to disk.** Accept a ZIP only, read through a bounded reader, and enforce the limits below before and while inflating. A limit hit aborts the import with no partial profile.

   | Limit | Value (recommended) | Why |
   | --- | --- | --- |
   | `.mhz` file size | ≤ 32 MiB | Official Default is 1.5 MB |
   | Entries | ≤ 256, and only these patterns: `profile<digits>_<1-64 [A-Za-z0-9]>`, `uploadImages/<name>`, `gifPools/<name>` | Official exports produce 1 + images |
   | Profile entries | Exactly one; otherwise reject | Avoids the official "last one wins" ambiguity |
   | Entry names | Reject absolute paths, `..`, `\`, drive letters, NUL or control characters, duplicates (after Unicode NFC and case folding), directories other than the two above, symlink or special modes in external attributes | Zip-slip, even though nothing is written to disk |
   | Methods and flags | Stored or deflate only. Reject encrypted entries (flag bit 0, AES extra), multi-disk archives and nested archives | Bounded decoder surface |
   | Uncompressed profile JSON | ≤ 16 MiB, counted on actual inflated bytes, not the header field | Official Default is 4.4 MB |
   | Total uncompressed | ≤ 64 MiB, and ratio ≤ 100:1 per entry, enforced while streaming | Decompression bombs |
   | `uploadImages/`, `gifPools/` | Skipped (not inflated) in the first version. Listed in the report | Images are unsupported in schema 2 |

4. **Parse the JSON defensively.** Nesting depth ≤ 64, strings ≤ 64 KiB, arrays ≤ 100,000 elements. Skip `{type:"Buffer",data}` values with a counting visitor instead of materializing them. Read only the fields the mapping needs and ignore unknown keys. Treat all values as untrusted (wrong types become Unsupported, never a panic). Use the official JSON only as input to a mapper. The resulting `Profile` must pass `Profile::validate` and stay under `MAX_JSON_BYTES` (1 MiB) and `MAX_BINDINGS` (256).
5. **Unsupported and Dangerous become unbound, with a report.** Every source control (module type, index or slot, page or folder, side) gets a report row: official `btnFunc`, result (`imported`, `imported_lossy` with reason, `unbound_unsupported`, `unbound_dangerous`), and a short reason. Show the report before the profile is saved, and keep it with the provenance. Redact `appFilePath`, `gifPath`, `url` and `content` in the report the same way the diagnostics bundle does (basename only for paths; `scheme://host` for URLs; `<N chars>` for text).
6. **Risk flags shown at import time:** any `text` with `press_enter` or a newline; any `open_url`; any `multi_action` combining `open_url` or `text` with `shortcut`; `system screenshot`; Windows-specific shortcuts. These are the categories [the action catalogue](../masterhub-actions.md#security-considerations) already treats as high-risk.
7. **Strip identifying data.** Do not keep serials, `devices`, local paths or Master ID data from the file. Do not offer upload to Master ID (proprietary backend, out of scope).
8. **Export (later, optional).** Writing `.mhz` would need the exact official shape, including image Buffers that the official app reads back without checks. Defer it until an official-app round trip can be tested. Note that a `.mhz` written by us would carry no signature, just like official files.

## Implementation notes

The importer follows the design above. Where it differs or decides something the design left open:

- **Input path.** The webview reads the file the user picked or dropped and sends its bytes as the raw `import_mhz_profile` request body, like the JSON import (`dragDropEnabled` is off, so no path exists). No path ever reaches the backend, so the frontend cannot make it read another file. The backend checks the 32 MiB limit before parsing.
- **ZIP reader.** A small reader of our own on top of `flate2` (raw deflate) and `crc32fast`, both already locked, instead of a ZIP crate. Only the central directory lists entries; each local header must match it; ZIP64, multi-disk, encrypted (flag bits 0 and 6, method 99, AES or strong-encryption extra fields), masked directories, symbolic links and special files, overlapping entries and methods other than stored and deflate are refused. Entry names must be valid UTF-8 without `\`, `:`, NUL, control or invisible characters, absolute paths, `.`, `..` or empty components; duplicates are compared case-insensitively (not NFC-normalized). Inflating stops one byte past the declared size, and the actual size and CRC-32 must match. The 100:1 ratio applies to entries over 1 MiB; smaller entries are bounded by the other limits. A multi-profile `.zip` of up to 8 `.mhz` files (one level) imports each as its own profile; one bad inner file rejects all.
- **JSON.** Depth 64, keys up to 1 KiB, at most 100,000 stored items per array and 500,000 stored values. Numbers inside arrays (the image `Buffer` bytes, `hotkey.code[]`) are counted and dropped, and strings over 64 KiB (base64 images) are replaced by a marker, so image payloads are never kept. A shortcut whose `key[]` contains a number is refused rather than read with the number removed.
- **Names and serials.** Names, labels and dial function names drop control and invisible characters and are clamped (128 or 64 bytes). Any stored string that contains a serial from the file (base serial, section keys, the device inventory, the entry name) is replaced (labels) or left unbound (URLs, text). No `serial_number` selector is ever written. Profile names get ` (imported)`, ` (imported 2)`, … and never collide.
- **Keypad.** The first keypad section with any bound tile is imported (the empty `CT99999999999` placeholder is skipped). Page 1 is the root; each later page is linked as a folder from the first `next-page` key of the page before it, within 8 levels and 64 pages; unreachable pages are reported key by key. `folder[]` becomes a child page; a non-empty key 1 that is not the Back tile is reported; folders inside folders are reported (official one level). Unbound keys keep their label with no binding.
- **Other modules.** Knob functions beyond the current one are reported in a warning. Dial functions keep their names (made unique) and are kept only when at least one side maps. Knob, roller and dial sides map `volume-control` (step 5, side `volumeAction` or the side's direction), `hot-key` and Adobe presets with a stored shortcut; everything else is unbound. Faders map only `volume-control` without a process id.
- **Mapping details.** `volume-control` on a key needs `volumeAction` and `dropdownOption` to agree when both are set. `undo-redo`, `media-track-control` and `text` count as imported without changes; shortcuts, URLs, screenshot, volume, profile and multi-step actions as imported with changes. `ps-flatten-image` and `ps-tool-auto-masking-tool` are treated as script presets. Unknown `btnFunc` values are reported as `unknown`, never echoed. The shortcut warning counts only shortcuts that end up bound.
- **Dry-run provenance.** Kept in `<app data>/imported-profiles.json`: per saved id a record (source SHA-256 and import time for imports, tombstones for deleted profiles), plus flagged content hashes (canonical profile JSON without its name). Every save of flagged content, or from a flagged source or over a flagged id, stays flagged. A sentinel makes a missing list an error, recovered only by "Mark all profiles as needing review". Allowing names the content hash the confirmation showed. Details are in [the session manager](../masterhub-session-manager.md#imported-profiles-dry-run-only).
- **Report.** Rows are capped at 1,000 (counts still include every control, and a warning says how many are not listed). Every redacted value and reason is checked for the file's serials; program paths show only a file name with an extension, never a folder such as a user's home. A sequence row keeps each step's reason and redacted value. The report is returned to the UI, kept for the session (the dry-run banner reopens it), and not stored. Its per-row high-risk mark follows the import rules above (pasted text with Return or a line break, web pages, screenshots, sequences mixing typing or pages with shortcuts); the confirmation before allowing actions uses the profile capability rules of the Execute confirmation, which may rate more profiles high risk.
- **More limits.** A file naming more than 64 distinct serial-like identifiers (8 to 64 ASCII letters and digits with at least one digit, once everything but letters and digits is removed and compatibility forms are folded with NFKC; numeric serials stored as JSON numbers count by their digits; the setting names under `deviceLightingEffect.wholeDevice` and `currentLightingEffectDevice` are not serials). Serials are collected from the entry name, the base `serialNumber`, every module section key, and the value of any key ending in `serialNumber` (any case) anywhere in the file, including keypad tiles, folder children and sequence steps is refused, which keeps serial scrubbing bounded. Names are checked for serials before they are clamped, URLs also after percent-decoding, and serial matching ignores case, separators and compatibility forms (fullwidth, mathematical, circled). Dial function names are checked again after a duplicate suffix is added. Scans are bounded: a label is scanned only as far as it can be stored plus 512 bytes (the stored label is a prefix of what was scanned), a URL longer than 2,048 bytes is refused before scanning, and a key name is scanned only when it is at most 16 bytes; values longer than 256 bytes are never serial candidates. Text is checked for serials only after the binding cap and payload budget, so text left unbound anyway costs nothing; URLs are checked first, so no report branch shows a serial. Report action ids are a closed catalogue; anything else is a prefix bucket (`ps-*`, `ai-*`, `pr-*`, `discord-*`, `streamlabs-*`) or `unknown`, and the serialized report stays under 512 KiB. Program paths show a file name only for program extensions (`exe`, `lnk`, `bat`, `app`, …) outside a user folder. The payload budget (400 KiB) counts JSON-escaped text, URLs and labels, and only actions that carry text or URLs are charged against it. A final check makes the pretty-printed profile fit 1 MiB minus 1 KiB (room for the later rename): it unbinds the last text-carrying bindings first, then any binding from the end, and marks their report rows and counts as left unbound; if even an empty set of bindings cannot fit, the import is refused with a clear error, never silently. Lighting and brightness keys stay unbound ("not supported yet"). A page with numbers among its keys is never offered as the next page. Numbers among a keypad's pages or among the faders make that list malformed. Tiles on dropped pages, including extra tiles and folder children, are all counted. Numbers among the keys of a page or folder, the steps of a sequence or the dial functions make that part malformed (unbound and reported) instead of shifting positions. Mute on a rotating side, a recorded macro on a side or its control, and per-application volume on the dial function or the dial stay unbound. A text whose `afterEnter` is neither true nor false (nor absent) is left unbound rather than guessed.

## Open questions

- Whether `folder[0]` of a `new-folder` tile is always the generated Back tile.
- Which field (`file` or `appFilePath`) an `app-files` multi-task step reads at run time.
- Official volume step size for keys, knobs and the dial, and the meaning of `sliders5List[].slider.volumeRange`.
- Physical direction of knob and dial `left`/`right`, and the order of the two rollers (shared with the [protocol evidence](../masterhub-protocol-evidence.md) hardware items).
- Whether the bundled `7z.exe` blocks `..` entries (zip-slip) in this build, and what `killApp` does.
- Meaning of the `CT99999999999` keypad serial in the Default template.
- Whether a real user export contains `uploadImages/` and `gifPools/` as described. No user export was available; only the two templates were inspected.
