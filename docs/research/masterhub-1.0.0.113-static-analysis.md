# MasterHUB 1.0.0.113 static analysis

Analysis date: 3 October 2026. The official installer was hashed and extracted on Apple Silicon macOS without executing the Windows application or SDK. Findings describe the software's M131E input, display, discovery, and lifecycle behavior; physical-device compatibility remains unverified.

## Package identity and reproducibility

- Official source: [MasterHUB software page](https://www.coolermaster.com/en-global/software-masterhub.html), version `1.0.0.113`, release date 27 May 2026.
- Download: [MasterHUB ZIP](https://coolermaster.egnyte.com/dd/4qMpfQw9jqkr).
- Archive: `MasterHUB-Setup_1.0.0.113.zip`, 332,445,419 bytes; SHA-256 `cf9aef839255b2144b4c6268b274429be054f33a37e12cbf7537542cdaa0d83f`.
- Installer: `MasterHUB-Setup_1.0.0.113.exe`, 333,029,040 bytes; SHA-256 `014c5f5899c4ec9327c6cfcb85df1cdfb8744fe08d9f2b9b476ae79012e7cbcb`.
- Format: native x86 WiX Burn bootstrapper containing two CAB containers, an MSI, and four application CABs. All 254 MSI File records were matched to extracted files and hashed.
- Application: Electron with Vue/Vuex, renderer package version `1.0.0-113`. `resources/app.asar` contains 4,945 files; its renderer source map contains 1,064 source entries, including original application sources.
- Tools: portable 7-Zip 26.03 for macOS, Python 3.14.7, `pefile` 2024.8.26, and Capstone 5.0.9.

The [manifest](masterhub-1.0.0.113-manifest.json) records source identity, installed names, file sizes/hashes, and native metadata. The [extraction script](tools/extract_masterhub_package.py) reconstructs the inventory from the archive. Proprietary binaries and extracted application sources are not included in the repository.

### Reproduce the extraction

Use Python with `pefile` and the native macOS `7zz` executable from the [official 7-Zip download](https://www.7-zip.org/download.html). The script supports the observed WiX Burn v2/two-CAB layout and fails explicitly for unsupported layouts. Supply a new or empty output directory; it preserves partially extracted evidence on failure and does not overwrite a previous analysis.

From the repository root, with `pefile` installed and paths adjusted for the archive and 7-Zip executable:

```sh
python3 docs/research/tools/extract_masterhub_package.py \
  --archive /path/to/MasterHUB-Setup_1.0.0.113.zip \
  --sevenzip /path/to/7zz \
  --output /path/to/new-analysis-directory
```

A second extraction matched all 254 installed paths, sizes, and SHA-256 hashes, plus archive/installer hashes. The inventory contains 96 PE files, two ASAR archives, and 65 dial SDK exports. The script reproduces extraction and metadata; source and native-function traces are documented below.

The cabinet member and MSI File ID `CoolTEK_SDK_CM_M131E_V02.00.01.dll` are identifiers retained by the installer. The MSI installs those bytes as `resources/build/Release/CoolTEK_SDK_CM_M131E_V03.02.00.dll`. The PE export name, `OriginalFilename`, version `03.02.00.0`, and native addon's import agree with that installed name. The analysis covers V03.02.00.

| Installed file | SHA-256 |
| --- | --- |
| `resources/app.asar` | `a80844ac26e9478023213095006bf88db178712fb7a2e1be7b244d8445202f3f` |
| `resources/build/Release/CoolTEK_SDK_CM_M131E_V03.02.00.dll` | `8fff167ab7c162cfd73883ec1245050ab9a34a080bb4d0e5c6ebbeff68b353e1` |
| `resources/build/Release/stackaddon.node` | `005ee05adbdcf313a15c1232cef90688ec151d61b8963abf148ea66eb4acfd87` |

## Module discovery

Application version 1.0.0.113 accepts both product-ID generations below, all with VID `0x2516`. Descriptors and interface selection still need physical-device validation.

| Module | Newer PID accepted by app | Legacy PID accepted by app | Evidence: UTF-8 byte offset in `background.js` |
| --- | --- | --- | --- |
| M0064 base, app name M0x64 | `0x9999` | `0x01f6` | `1735785` |
| M003K three knobs | `0x9996` | `0x0202` | `1734603` |
| M131E IPS dial | `0x9994` | `0x0206` | `1734119` |

The Rust SDK recognizes both generations while retaining the legacy `PRODUCT_ID` constants.

Source `src/components/Common/TopNav.vue`, lines 2254–2256, maps these modules to firmware UUIDs:

- Base: `ebf5c18b-0386-5dba-ba01-498ffdae2908`.
- Knobs: `1f7b995d-ee2f-5705-a07f-381a8ffcd2ac`.
- Dial: `93e23f53-b0b2-509c-8777-09f51522dab7`.

The [dial's stable firmware metadata](https://dev.pms.masterctrl.coolermaster.com/api/devices/93e23f53-b0b2-509c-8777-09f51522dab7/firmwares/stable) returned `1.0.4.0` and its asset path on 3 October 2026. This is the published version, not a reading from a device.

## Dial input

The main process polls `Check_IO_Status_CM_M131E` with a 256-byte destination Buffer, timeout `0`, and selected serial. It schedules polling after 20 ms. The native addon forwards the Buffer's data pointer directly to `Check_Rcv_Data_CM_M131E`, requests queue/type `0`, and returns the SDK's result code unchanged. The application processes result `0` as available input.

The SDK checks raw report ID `01`. Its helper maps queue types `0`, `1`, `2`, `3` to class tags `44`, `45`, `46`, `47`, respectively. For types 0, 1, and 3, it reads the little-endian payload length from bytes 2–3 and copies the payload starting at byte 4. Type 2 instead exposes the whole input report. Known reports for a different requested type are queued.

For the app's ordinary dial input queue, the software-derived raw layout is:

| Raw offset | Field | Evidence |
| --- | --- | --- |
| 0 | Report ID `01` | SDK rejects other report IDs in this receive path |
| 1 | Class tag `44` | Native addon requests queue 0 |
| 2–3 | Payload length, `u16LE` | SDK uses a two-byte load |
| 4 onward | Payload | SDK copies the declared length to the Buffer |

The application then interprets these payload fields:

| Payload byte 0 | Payload byte 2 | App interpretation |
| --- | --- | --- |
| `04` | `01` | Press; record press timestamp |
| `04` | `00` | Release; dispatch configured short/long press behavior |
| `03` or legacy `00` | `00` | RIGHT rotation action |
| `03` or legacy `00` | `01` | LEFT rotation action |

The renderer also accepts legacy button selector `01` for release. Payload byte 1 is ignored by this consumer; its meaning and the complete actual payload length remain unknown. These facts do not justify inventing a canonical complete raw report. RIGHT/LEFT are the application's labels; physical clockwise/counterclockwise mapping needs observation. The M003K consumer uses the opposite direction-bit convention, so a shared direction enum conversion would need module-specific handling.

Long press is a host decision based on at least 600 ms since the press timestamp. It is not evidence of a firmware long-press event. Short click switches the current dial function; long press executes the configured press action.

The Windows SDK does not bound the declared payload length against the actual read length or destination capacity. The Rust decoder validates the declared length against the received bytes, borrows the payload without copying it, and retains unknown reports.

## Dial display: image preparation and packet construction

The current application action `dialEncoderImageBufferGenerator` creates a 240×240 `OffscreenCanvas`. User images are cropped to a centered square and scaled to that canvas. JPEG output uses `convertToBlob` with `image/jpeg` and quality `1.0`. The image Buffer, byte length, and target serial reach the native display call. GIF import produces individual JPEG frames for transmission; it does not send the GIF file as a device-native animation format.

These are application construction facts. Actual display acceptance and orientation remain unverified; no rotation transform was found in the reviewed canvas generator. The main handler blocks image writes while a firmware update is marked in progress and requires a target serial.

The native `Display_Picture_On_Screen_CM` implementation constructs output reports as follows:

| Offset | Bytes/field |
| --- | --- |
| 0 | Output report ID `02` |
| 1 | Image command `45` |
| 2–3 | Chunk sequence, `u16LE`, starting at 0 |
| 4–5 | Bytes of image payload in this chunk, `u16LE` |
| 6 | Not assigned by this function; meaning/value requirement unknown |
| 7 | Final-chunk flag: `01` when remaining bytes fit; otherwise `00` |
| 8 onward | Image payload |

Chunk capacity is `OutputReportByteLength - 8`. The output length comes from HID descriptor capabilities, not a recovered fixed number. Every chunk is passed to `hid_write` using the complete output report length.

The image validation helper accepts BMP `BM` or `FF E0` at input offsets 2–3, a narrow JPEG/JFIF signature check. It does not prove support for every valid JPEG variant. BMP processing also exists, but its transformation was not fully reconstructed; JPEG is the first implementation target because the current app uses it.

The picture loop checks write completion and does not read a protocol acknowledgement or explicitly sleep between chunks. The Windows transport waits for overlapped write completion; that is not a device acknowledgement. Other receive classes and device-side pacing may still matter and need capture evidence.

The Windows SDK output buffer is zeroed once at connection and reused. Byte 6 and final-chunk tail bytes are not cleared by the picture function. The Rust packet builder zeroes these bytes; physical acceptance of that policy remains unverified.

## Ordinary feature commands

All three setters below use 32-byte feature reports. Only assigned fields are shown; trailing bytes in the official reused buffer are not guaranteed to be zero.

| Operation | Assigned prefix/arguments | Native implementation RVA |
| --- | --- | --- |
| Software mode | `03 49 <mode>`; app uses true/false | `0x4f90` |
| Backlight | `03 44 <value>`; SDK accepts 0–100 | `0x5030` |
| Solid color | `03 46 <color0> <color1> <color2>` | `0x5fa0` |

`Get_Backlight_CM` requests feature ID `4c` and reads the value from reply offset 2. `Get_FW_Infos_by_type_CM` selects IDs `47`, `48`, `49` for type 0, 1, 2; the ordinary M131E firmware wrapper selects type 1. These are static query findings, not tested device responses. Do not infer identical meanings for feature IDs and output/input command bytes merely because numbers overlap.

## Startup and macOS transport implications

- Both native connect paths allocate descriptor-sized input/output buffers, initialize queues, and query feature report `4b` for device information. The meaning of all returned device-information fields remains unresolved. Native connect itself does not write software mode.
- The app connects by serial, sets software mode true after success, and schedules a firmware query. On suspend it sets hardware mode; on resume it waits two seconds, re-enters software mode, queries firmware, and requests a display refresh. Exit also returns hardware mode and disconnects. These are design references for a later macOS session manager, not verified timing requirements.
- `hid_open_path` obtains input/output report lengths from `HidP_GetCaps`. The native first-match open path does not apply a usage/interface filter. The local SDK's explicit path/interface discovery remains useful.
- The Windows reader preserves a nonzero report ID such as `01`; it strips a leading zero only for an unnumbered report.
- The Windows feature reader unconditionally adds one to its successful returned byte count. Consequently a requested 32-byte feature buffer is checked against SDK return 33. That is a transport counting convention, not evidence of a 33-byte request. The macOS implementation must preserve HIDAPI's actual returned count instead of copying this increment.

## Evidence locations

Native RVAs are relative to image base `0x180000000` and refer to the hashed SDK/addon above. Export thunks jump through wrappers; the implementation addresses below resolve those jumps.

| Finding | Evidence pin |
| --- | --- |
| Input framing/copy | SDK `Check_Rcv_Data_CM_M131E`: thunk `0x11bd`, wrapper `0x3640`, implementation `0x6080`; ID check `0x6169`, length load `0x61ff`, payload offset `0x6208` |
| Class-tag mapping | SDK helper `0x72c0`; tag comparisons `0x7302`, `0x7345`, `0x7388`, `0x73c8` |
| Addon queue and direct Buffer forwarding | Addon callback `0x134d0`; queue 0 at `0x137ad`, SDK call `0x137bd` |
| Descriptor report lengths | SDK `hid_open_path` `0x1f90`, caps call `0x208c`, output/input assignments `0x20b9` and `0x20c2` |
| Image chunk builder | SDK `Display_Picture_On_Screen_CM` `0x5ad0`; header `0x5b54`, capacity `0x5d85`, sequence `0x5de2`, length `0x5df9`, payload `0x5e6c` |
| Image validation | SDK helper `0x6c60` |
| Feature returned-count increment | SDK `hid_get_feature_report` `0x26a0`, increment `0x2795` |
| Current image generator | Renderer source `src/store/module/knobs.js`, lines 807–825, 875–880, 1029–1082 |
| Dial action interpretation | Renderer source `src/App.vue`, lines 7494–7583 |
| Main input polling | `background.js` UTF-8 offsets `1746889`, `1746942` |
| Main image forwarding | `background.js` UTF-8 offset `1739340` |
| Startup/suspend/resume | `background.js` UTF-8 offsets `1732044`, `1712889`, `1713401` |

The renderer map `js/app.c762092d.js.map` has SHA-256 `79da9c6d60fca74ce4eb7842257f15e92139a8acbc9b38c29e2e332f89658435`; `background.js` has SHA-256 `d3139e7214cb8195c3026165bb1c4b22324c18fca6bedf3b10aa85eba941d0ef`. Source lines refer to `sourcesContent` extracted from that map.

## Implementation and hardware validation

The Rust SDK implements both PID generations, bounded M131E input decoding, feature commands, descriptor-sized JPEG packets, and lifecycle hooks. See [implementation status](../masterhub-implementation-status.md) for test coverage.

Hardware validation must establish descriptors/topology and report sizes, interface selection, firmware, complete input payloads, physical direction/press behavior, display acceptance/orientation, byte 6, pacing/acknowledgements, and reconnect/sleep-wake behavior.
