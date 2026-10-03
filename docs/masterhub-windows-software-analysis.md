# Extracting protocol information from the Windows software

Static analysis of MasterHUB 1.0.0.113 covers the M0064 base, M003K three-knob module, and M131E IPS dial. The [protocol report](research/masterhub-1.0.0.113-static-analysis.md) records package hashes, source locations, native addresses, and unresolved fields. The application was extracted without execution on 3 October 2026.

## Package and protocol findings

- The [official software page](https://www.coolermaster.com/en-global/software-masterhub.html) lists MasterHUB `1.0.0.113`, released on 27 May 2026.
- The [download](https://coolermaster.egnyte.com/dd/4qMpfQw9jqkr) contains `MasterHUB-Setup_1.0.0.113.zip`, 332,445,419 bytes. Archive and installer SHA-256 hashes are recorded in the protocol report.
- The installer is a WiX Burn package. CAB and MSI extraction recovered all 254 application files and their installed-name mappings. The application uses Electron and Vue/Vuex, with original application sources in renderer source maps.
- The active dial library is `CoolTEK_SDK_CM_M131E_V03.02.00.dll`, despite an older version appearing in its cabinet File ID. Its 65 exports and native addon callers establish input framing, display chunk fields, ordinary feature commands, and relevant transport behavior.
- The app constructs 240×240 JPEG dial images, interprets rotation/press payload fields, and recognizes both PID generations for these modules. Hardware behavior remains unverified.
- The upstream probe's [M003K feature notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m003k/feature_reports.md) distinguish firmware commands present in the SDK from the command used by the application, and describe SDK response processing. These notes concern older analyzed software, not a verified analysis of version 1.0.0.113.
- The probe's [M4315 output notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/m4315/output_reports.md) describe that module's JPEG conversion and packetization. The M131E dial uses the separate layout documented in the protocol report.

## Analysis targets

| Target | Information to look for | Contribution to this project |
| --- | --- | --- |
| Native SDK binaries | Exports, symbols, HID imports, report constructors, callback dispatch, parser branches, command lengths | Dial input decoding, initialization, lighting, firmware queries, command framing |
| Application code | SDK callers, image preparation, profile-to-device mapping, retry/acknowledgement logic | Display encoding, dimensions, orientation, transfer sequencing, lifecycle behavior |
| Configuration and resources | Device catalogs, model identifiers, interface filters, UUIDs, defaults, image templates | More precise discovery and identification of the M131E display path |
| Update metadata and firmware assets | Version manifests, module mappings, asset URLs, container headers | Firmware identification and compatibility research without flashing |

The protocol report identifies recovered fields and unresolved behavior for each component.

## Analysis procedure

1. Download the official archive into a separate analysis workspace. Record the source URL, version, retrieval date, SHA-256, and extracted-file hashes. Keep proprietary binaries out of the source repository.
2. Identify the installer format and extract its payload without executing it. If static extraction is impractical, installation in a disposable Windows environment is a later fallback.
3. Inventory the payload. Inspect native PE exports/imports and strings first. Search specifically for M131E, M003K, M0064, the known VID/PIDs, HID entry points, image encoders, device catalogs, and SDK capability names.
4. Follow M131E code from discovery through initialization, input dispatch, and display transfer. Recover packet constructors and parser behavior before translating them into Rust.
5. Document each operation's binary hash, function/export or address, report kind, field offsets, lengths, sequencing, and unresolved fields. Implement recovered behavior with synthetic fixtures for tests.
6. Mark results as software-derived and hardware-unverified. Confirm report-ID handling, real input buffers, timing, acknowledgements, and firmware compatibility against captures after delivery.

PE metadata, Capstone disassembly, and Electron source maps provide the evidence locations in the protocol report.

## Dynamic analysis without the physical device

In a Windows environment, native-call instrumentation can observe SDK functions and HID boundaries such as `HidD_GetFeature`, `HidD_SetFeature`, `ReadFile`, and `WriteFile`. [Frida's Interceptor API](https://frida.re/docs/javascript-api/#interceptor) supports observing and replacing native function behavior.

A small shim could simulate device discovery and selected serial/firmware/acknowledgement responses, allowing the official application or SDK to construct commands for a simulated module. Whether this succeeds depends on the application's discovery checks and handshake requirements; it is not yet demonstrated for MasterHUB. Directly invoking a recovered SDK packet constructor may be simpler than simulating the entire application.

This is a software middleman and needs no Raspberry Pi USB gadget. It complements later USBPcap captures. Static code or simulated responses alone cannot verify what the real device accepts or emits.

## Firmware metadata

The public firmware service at `https://dev.pms.masterctrl.coolermaster.com` returned these stable versions on 3 October 2026:

- [M0064 stable metadata](https://dev.pms.masterctrl.coolermaster.com/api/devices/ebf5c18b-0386-5dba-ba01-498ffdae2908/firmwares/stable): `2.0.4.0`.
- [M003K stable metadata](https://dev.pms.masterctrl.coolermaster.com/api/devices/1f7b995d-ee2f-5705-a07f-381a8ffcd2ac/firmwares/stable): `1.0.5.0`.
- [M131E stable metadata](https://dev.pms.masterctrl.coolermaster.com/api/devices/93e23f53-b0b2-509c-8777-09f51522dab7/firmwares/stable): `1.0.4.0`; the dial UUID was recovered from the application's catalog.

Versions are a dated observation. Asset paths end in `.sys`; the extension does not establish that these module firmware assets are Windows drivers. No firmware was flashed or executed. See the probe's [firmware notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/0e72f87547537193309c3a5f2eae9ef483d80d7f/docs/MasterHUB/modules/firmware.md) for the documented service behavior.

## Implementation and hardware validation

The SDK implements PID aliases, M131E decoding, feature commands, descriptor-sized JPEG packets, and lifecycle hooks. See [implementation status](masterhub-implementation-status.md) and [diagnostic commands](masterhub-diagnostics.md). Physical-device validation remains pending; dynamic instrumentation can investigate discovery and handshake behavior if static analysis leaves a blocking question.
