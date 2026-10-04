# MasterHUB keys, faders and rollers: protocol evidence

Analysis date: 3 October 2026. This extends the [version-pinned Windows software analysis](masterhub-1.0.0.113-static-analysis.md) to M4315, M005S and M032LE. The official application's extracted renderer, native SDK and public probe notes provide enough information for initial input support. The Windows executables were not run; device behavior remains unverified.

The manufacturer's product sheet establishes module names and specifications, but does not provide a HID protocol or display refresh-rate guarantee. The packet details below come from software analysis and reverse-engineering notes.

The [manifest](masterhub-1.0.0.113-manifest.json) records these installed binaries. Installer file IDs retain older version strings and do not identify the analyzed binary's version.

| Installed native SDK | SHA-256 | Installer file ID |
| --- | --- | --- |
| `CoolTEK_SDK_CM_M0_V02.05.00.dll` | `8e97c1f78d6231d70fc4a8e74958dab30ed1ab7710f69713e9b96aa6ea3d0f93` | `CoolTEK_SDK_CM_M0_V01.05.00.dll` |
| `CoolTEK_SDK_CM_M4315_V03.02.00.dll` | `21a49758faf86ea9808cec870f5db6434f9a78e7441d9184e272e9a435e2f35d` | `CoolTEK_SDK_CM_M4315_V02.00.00.dll` |

Both are installed under `resources/build/Release/`. Renderer and `background.js` hashes are pinned in the original report. Native RVAs below are relative to each DLL's image base.

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

The native per-key image path was also identified: wrapper `35d0` calls implementation `5bb0`, using `02 36`, sequence at bytes 2–3, payload length at 4–5, key index at 6, final flag at 7, and JPEG payload at 8. Output length comes from the descriptor. This is evidence for a future per-key upload API; **this milestone does not implement keypad image uploads**. Key dimensions/encoding restrictions, image acceptance, orientation, transfer pacing and responsive concurrent input require separate validation. The desktop app does not upload either IPS display yet.

## Offline validation and hardware acceptance

The [synthetic fixture provenance](../../crates/cooltek-sdk-cli/tests/fixtures/README.md) describes the new captures. Tests cover simultaneous controls, repeated snapshots, endpoints, invalid values, truncated reports, unknown selectors/classes, state preservation, reset, both PID generations, concrete fader action resolution, and mode cleanup using simulated transports. These are implementation checks, not device recordings.

When hardware is available, establish complete report lengths, descriptors/topology, interface selection, initialization, physical control order/direction, fader range/jitter and startup behavior. Then verify actual macOS actions, display writes and input responsiveness. Refresh-rate limits and complete firmware compatibility remain unknown.
