# MasterHUB keys, faders and rollers: protocol evidence

Analysis date: 3 October 2026; keypad image evidence extended on 4 October 2026. This extends the [version-pinned Windows software analysis](masterhub-1.0.0.113-static-analysis.md) to M4315, M005S and M032LE. The official application's extracted renderer, native SDK and public probe notes provide enough information for initial input support and offline keypad image planning. The Windows executables were not run; device behavior remains unverified.

The manufacturer's product sheet establishes module names and specifications, but does not provide a HID protocol or display refresh-rate guarantee. The packet details below come from software analysis and reverse-engineering notes.

The [manifest](masterhub-1.0.0.113-manifest.json) records these installed binaries. Installer file IDs retain older version strings and do not identify the analyzed binary's version.

| Installed native SDK | SHA-256 | Installer file ID |
| --- | --- | --- |
| `CoolTEK_SDK_CM_M0_V02.05.00.dll` | `8e97c1f78d6231d70fc4a8e74958dab30ed1ab7710f69713e9b96aa6ea3d0f93` | `CoolTEK_SDK_CM_M0_V01.05.00.dll` |
| `CoolTEK_SDK_CM_M4315_V03.02.00.dll` | `21a49758faf86ea9808cec870f5db6434f9a78e7441d9184e272e9a435e2f35d` | `CoolTEK_SDK_CM_M4315_V02.00.00.dll` |

Both are installed under `resources/build/Release/`. Renderer and `background.js` hashes are pinned in the original report. Native RVAs below are relative to each DLL's image base.

## LED commands (knobs, faders, base)

Recovered by disassembling the M0 SDK DLL (`CoolTEK_SDK_CM_M0_V01.05.00.dll`, the exports `Adjust_LED_Color_CM_M0`, `Adjust_LED_Brightness_CM_M0`, `Adjust_LED_Mode_CM_M0`, `Adjust_LED_Speed_CM_M0`) and the call sites in the native addon `stackaddon.node`. Nothing was run; every byte below is static analysis and unverified on hardware.

Each command is an output report with ID `02`, sent with the length the interface's descriptor declares (`63` payload bytes on a captured M003K), zero padded:

| Command | Bytes after `02` | Validation seen in the DLL |
| --- | --- | --- |
| Colour | `2d type index a b c d` | Type and index checked as below. The four value bytes are probably red, green, blue and a save-to-firmware flag, from the order of the application's `(r, g, b, saveFw)` call; not confirmed |
| Brightness | `2e type index value` | Type and index checked as below; no range check on the value was recovered (the dial uses 0 to 100) |
| Mode | `2c type index mode save` | Mode at most `9`; type and index checked as below |
| Speed | `30 value` | Value at most `100`; no type or index |

Type and index: type `0` is the base (index `0` only), type `1` the knob module (index `0` to `4`, or `ff` for every LED), type `2` the fader module (index `0` to `2`, or `ff`). Any other type or index is refused before a write. The mode numbers are the nine effects of the official Lighting tab plus off (see the probe project's lighting modes list).

The official application only ever addresses a whole module: its addon calls the colour function once per index (`0` to `4` for knobs, `0` to `2` for faders, `0` for the base) with the same colour and never `ff`. So the protocol can colour each LED separately, but the application does not use that. Open questions: what each index drives (the knob module has three knobs but five indexes, so they may be rings plus something else, or segments), the colour channel order, whether `save` persists across a replug, and what the fader module's three indexes light. The addon imports a `V02.05.00` copy of the DLL while the installer ships `V01.05.00`, so the layout could differ on newer firmware.

## Discovery

The official application's `startBaseDeviceCheck` accepts both PID generations with VID `2516`. The SDK's discovery and typed module handles now recognize them.

| Module | Legacy PID | Newer PID |
| --- | --- | --- |
| M4315, 15-key IPS | `01fe` | `9997` |
| M005S, five faders | `0200` | `9991` |
| M032LE, two rollers | `0204` | `9995` |

Product IDs identify a module; actual descriptors and interface selection still require hardware capture.

## Input framing and logical payloads

Known input reports use report ID `01`, a class tag at byte 1, a little-endian payload length at bytes 2–3, and payload beginning at byte 4. M4315 input uses class `34`; M005S and M032LE use the M0 class `2a`. The native receive functions supply these payloads to the app. The Rust parsers bound the declared length against received bytes and ignore outer HID padding only after that check.

| Native path | Evidence RVAs |
| --- | --- |
| M0 `Check_Rcv_Data_CM_M0` | Thunk `12b2` → implementation `4100`; report ID check `41ec`; queue helper `51e0`, tag `2a` at `522c`; length load `427f`, payload copy `4288`/`4297` |
| M4315 `Check_Rcv_Data_CM_M4315` | Thunk `103c` → wrapper `3700` → implementation `67e0`; report ID check `68cc`; queue helper `7aa0`, tag `34` at `7ae2`; length load `695f`, payload copy `6968`/`6977` |

| Module | Logical payload | Interpretation |
| --- | --- | --- |
| M4315 | `02 0f` followed by 15 bytes | Each `00` is released and `01` is pressed. The official renderer's legacy example also uses selector `00`; both selectors are supported. |
| M005S | `02 05` followed by five bytes | Absolute positions, each in `0..100`. No unchanged sentinel is established. |
| M032LE | `01 02` followed by two bytes | `00` = application Up, `01` = Down, `ff` = no update for that roller. No magnitude or tick count is established. |

The renderer's `src/App.vue`, lines 3051 onward, compares all 15 key states; the example at line 3056 uses the legacy selector. Its fader handler at lines 7585–7596 reads payload bytes 2–6. `src/store/module/slider.js`, lines 153–164, copies those values directly into the five positions, which the app uses as percentages. The roller handler near line 7430 labels wire index 0 as its right roller and index 1 as its left roller. Physical placement and direction remain unverified; the SDK keeps wire indexes.

The [fader probe notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m005s/input_reports.md) have a selector inconsistency: their heading and example use `02`, while a table says `00`. This remains an unresolved source conflict. The implementation uses `02` and retains selector `00` as unknown. It does not broaden accepted input based on the contradictory table.

The [key notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m4315/input_reports.md) and [roller notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m032le/input_reports.md) supply their logical payload formats. The pinned renderer and native framing corroborate the input path; they do not prove every firmware emits exactly these layouts.

## Decoder and action behavior

All recognized snapshots require the documented control count and complete values. Invalid snapshots never partially change state; unknown reports retain the complete raw bytes and generate no actions.

- Keys emit only press/release transitions. A new decoder assumes released keys; repeated snapshots do not retrigger actions.
- Faders emit all five positions on the first valid snapshot, then only changed positions. Reset clears the known positions. An initial snapshot can therefore plan an action for an already-positioned fader.
- Rollers emit each reported direction update, including consecutive identical updates. `ff` is idle. They have no inferred press, absolute-position or distance events.

The binding service adds `m4315`, `m005s` and `m032le` while preserving existing schema-1 profiles. Fader `position_changed` events include `value: {"type":"position","raw":63,"maximum":100}`. A `volume_from_position` binding resolves to the concrete `{"type":"set_volume","percent":63}` action before logging or dispatch. The resolver rejects missing/out-of-range values and arbitrary maxima; unresolved binding actions cannot reach a native dispatcher.

OpenStreamHub allows editing each module's controls, selected-event previews, and all five bundled replay captures. It still performs no hardware I/O or host actions. The CLI can read one exact interface and execute actions only with its explicit execution flag.

## Keypad software mode and display boundary

The native M4315 software-mode wrapper at RVA `3440` calls the implementation at `5070`, constructing a 32-byte feature report with prefix `03 3a <bool>`. The app sets this flag false during suspend and true during resume. `keypad::software_mode_report` and `keypad::set_software_mode` implement the explicit setter; CLI recording/live service accept `--software-mode` for either IPS module and attempt one hardware-mode restoration after an enable attempt. Default opening and reading remain neutral.

The native per-key image wrapper at RVA `35d0` calls sender `5bb0`. Header stores at `5c89`–`5c96`, `5ee8`–`5efe` and `5f27`–`5f48` establish the following layout:

| Offset | Field |
| --- | --- |
| 0–1 | Output report ID `02`, command `36` |
| 2–3 | Little-endian `u16` sequence, starting at zero |
| 4–5 | Little-endian `u16` payload length |
| 6 | Wire key index, 0–14 |
| 7 | Final packet flag, zero or one |
| 8 onward | Encoded image payload |

The sender derives capacity from the HID output length minus eight (`5eb7`–`5ec6`), copies payload at `5f93`, and writes the complete report length (`5fe3`–`5fff`). The [pinned probe output notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m4315/output_reports.md) corroborate this layout and key range. Their table's 255 limits conflict with the two-byte native fields; the implementation uses `u16` bounds. Their 320-byte report example does not establish a universal device report size.

The renderer's `src/store/module/keypad.js` creates a 72×72 canvas at line 2405 and can encode it as JPEG at quality 1.0 at lines 2455–2457. Clearing a key creates a 72×72 black JPEG near line 2103; fallback images resize to 72×72 near line 3394 and are passed to M4315 near lines 3482–3486. `background.js`, UTF-8 byte offset `1737497`, forwards those bytes, length, key index and serial unchanged. Nearby transformations for M3704 belong to a different model and are not applied to M4315.

Native image helper RVA `7440` checks for a BMP signature or `ff e0` at JPEG offsets 2–3. It does not establish complete dimension, scan, color-space or entropy restrictions. The SDK's 72×72, eight-bit, three-component baseline JFIF with one interleaved scan is a conservative supported format based on the renderer output; it is not a proved list of every firmware-supported encoding.

`keypad::JpegPackets` constructs descriptor-sized reports offline, validates indexes and supported JPEG framing before producing packets, rejects sequence overflow, and zero-fills padding. The CLI prepares static images and writes packet plans without opening HID. **Physical keypad uploads remain unimplemented.** Firmware acceptance, orientation, physical key order, software-mode requirements, transfer pacing and concurrent input responsiveness require device validation. The native sender exposes no inter-packet delay or acknowledgement here. The desktop app does not upload either IPS display yet.

## Offline validation and hardware acceptance

The [synthetic fixture provenance](../../crates/cooltek-sdk-cli/tests/fixtures/README.md) describes the new captures. Tests cover simultaneous controls, repeated snapshots, endpoints, invalid values, truncated reports, unknown selectors/classes, state preservation, reset, both PID generations, concrete fader action resolution, and mode cleanup using simulated transports. These are implementation checks, not device recordings.

When hardware is available, establish complete report lengths, descriptors/topology, interface selection, initialization, physical control order/direction, fader range/jitter and startup behavior. Then verify actual macOS actions, display writes and input responsiveness. Refresh-rate limits and complete firmware compatibility remain unknown.
