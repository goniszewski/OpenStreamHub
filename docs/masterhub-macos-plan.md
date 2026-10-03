# MasterHUB macOS support plan

Updated on 3 October 2026. This roadmap covers macOS support for the M0064 base, M003K three-knob module, and M131E IPS dial. The SDK builds on Apple Silicon macOS; device access, physical input, and display behavior still require hardware validation.

## Scope and intended outcome

Build direct macOS support for the M0064 base, M003K three-knob module, and M131E IPS dial. The first release should read their controls and dispatch configurable macOS actions. Add dial display content once its protocol is validated. Keep profiles, action integrations, and the GUI separate from the reusable SDK.

Firmware flashing/recovery, additional module types, and complete MasterCTRL parity are later projects. Capture traffic to reproduce existing device capabilities; new shortcuts, profiles, and application integrations can live in the macOS application without modifying firmware.

Windows captures can resolve protocol questions left open by static analysis. Verify VM passthrough and capture-driver compatibility, particularly on Windows on ARM, before relying on a virtualized capture setup.

The official Windows installer, application, and SDK provide protocol information for the M131E input and display path; see the [Windows software analysis](masterhub-windows-software-analysis.md). Captures are still needed to confirm actual device behavior.

The [static-analysis findings](research/masterhub-1.0.0.113-static-analysis.md) underpin the implemented PID aliases, dial input decoding, controls, descriptor-sized JPEG output, and lifecycle/gesture APIs. The SDK and CLI pass offline validation; see [implementation status](masterhub-implementation-status.md).

## Verified SDK baseline

Baseline commit `d996ede` passed `cargo check --workspace --all-targets --locked` on Apple Silicon macOS with Rust 1.98.1. This baseline predates the diagnostic and dial implementation.

The baseline workspace contains `cooltek-sdk` and `cooltek-sdk-masterhub`, requires Rust 1.97.0, and lists both Apple Silicon and Intel macOS targets. The HIDAPI dependency enables `macos-shared-device` for shared handle access. Device sessions still need to coordinate competing writers.

| Target | Default VID and PID | Implemented at baseline | Missing at baseline |
| --- | --- | --- | --- |
| Base M0064 | `2516:01f6` | Identity and serial feature report `0x30` | Firmware information, relevant configuration/lighting, lifecycle handling |
| Three-knob M003K | `2516:0202` | Identity and serial feature report `0x30` | Rotation/press decoding and lighting |
| IPS dial M131E | `2516:0206` | Identity, serial feature report `0x4a`, automatic software mode | Dial input and screen control |

The SDK also recognizes M005S, M032LE, and M4315, which are outside this roadmap. At baseline, only serial-number and software-mode capabilities were implemented.

Baseline limitations:

- `ModuleHandle::connect()` opens the first VID/PID match. Discovery must permit explicit path/interface selection and identify multiple instances.
- Connecting to M131E automatically sent a 32-byte feature report beginning `03 49 01`, followed by zero padding. `0x03` is the report ID and `0x49` the command. The serial example also initialized the dial.
- The serial parser discarded the actual received length and trusted a length byte before slicing a 32-byte buffer.
- Type state means a handle was opened successfully; it does not establish that the hardware remains attached. Reconnect and sleep/wake are runtime concerns.
- Tests at baseline concern VID/PID validation and API shape; they provide no protocol or device evidence.

The implementation adds neutral opening, actual-length serial parsing, exact interface selection, and alternate PIDs while retaining the original default IDs. `connect()` still selects the first matching interface and initializes it; `open_by_identity()` opens an exact identity without initialization. See [implementation status](masterhub-implementation-status.md) for the complete capability inventory.

## Reference protocol and uncertainty

Use `projectzero7/cooltek-sdk` as the architectural foundation and `projectzero7/cooltek-sdk-probe` as a protocol notebook. The probe is experimental and should not be copied as a production transport/session implementation.

The probe documents M003K rotation and press reports. Its rotation payload uses `0x00` for counterclockwise, `0x01` for clockwise, and apparently `0xff` for unchanged; press state uses `0x00` and `0x01`. The notes include logical channel tags. Confirm raw USB/HID framing and report-ID behavior before treating these logical examples as exact HIDAPI input buffers.

The probe also documents useful base, fader, roller, and 15-key operations, including chunked per-key JPEG transfer. It has unfinished whole-screen and firmware operations. There is no corresponding M131E protocol section in the probe tree inspected at revision `0e72f87547537193309c3a5f2eae9ef483d80d7f`. Do not infer that the IPS dial shares the 15-key image protocol.

For each capability, record module/firmware, report type and ID, endpoint/interface, framing, payload, acknowledgements, timing, evidence fixture/capture, and status. Distinguish documented, implemented, fixture-validated, and hardware-validated behavior.

## Implementation milestones

### 1. Offline diagnostics (implemented)

The CLI provides enumeration, exact interface selection, timestamped input recording, replay, dial command previews, and JPEG packet planning. HID operations are isolated behind a testable transport; configuration writes require explicit selection.

Serial parsing and M003K/M131E decoders validate received lengths and preserve unknown reports. Tests cover malformed packets, rotations, button transitions, and transport failures. Workspace tests, formatting, Clippy, and documentation checks pass; see [implementation status](masterhub-implementation-status.md).

### 2. First macOS hardware session

Enumerate all matching interfaces and record descriptors, topology, paths, usage pages, serials, and firmware versions where safely available. Record physical controls separately before initialization, then explicitly enable any required mode and repeat. Record module removal and reattachment.

Gate: the three target modules can be identified and their raw reports associated with gestures. Establish actual macOS access before expanding protocol work.

### 3. Three-knob input and first action

The [binding and action service](masterhub-service.md) connects SDK decoders to strict JSON profiles and an explicit macOS action backend. It normalizes module/control/gesture/timestamp events, preserves unknown and malformed reports in logs, and supports system output volume, mute, keyboard shortcuts, and app launching. Serial-specific bindings override module-wide bindings. Replay always plans actions; live execution is opt-in and selects one exact interface.

Offline tests cover decoding, matching, duplicate filtering, gesture boundaries, simulated action failures, and cleanup. Live input normalization runs independently of serialized actions through a bounded queue, with explicit overflow failure and bounded log writes. Connect these paths to observed device framing, then verify actual macOS actions and physical control order. Sustained rotation throughput, HID buffering, and physical timing remain runtime acceptance work.

Gate: an agreed physical gesture sequence gives correct events/actions, including fast and simultaneous movement. Validate press/release state without duplicate actions.

### 4. IPS dial implementation and hardware acceptance

The [Windows software analysis](research/masterhub-1.0.0.113-static-analysis.md) documents M131E packet construction, input parsing, and image preparation. Hardware validation must confirm the recovered protocol and resolve the remaining fields.

Discovery accepts both PID generations; the dial decoder uses the application's RIGHT/LEFT labels. Commands support mode, backlight and raw color channels, plus backlight/firmware queries with actual returned-length validation. Feature reply byte 1 remains opaque, and firmware bytes retain unresolved meanings.

JPEG upload accepts an encoded 240×240, 8-bit, three-component baseline JFIF image with one interleaved scan. The builder checks framing and dimensions but does not entropy-decode the image. Typed upload reads the selected interface's descriptor and derives output report capacity; unavailable or unsupported descriptor information fails rather than using a fixed guess. Packets deliberately zero header byte 6 and final padding, check write completion, and do not infer acknowledgements or add pacing delays. The offline CLI now converts static PNG/JPEG artwork with explicit fit, alpha matte, quality, and rotation; progressive JPEG input is re-encoded as baseline. EXIF/color profiles are not automatically applied. BMP, progressive output, and animation remain unsupported. Physical display acceptance remains pending.

Complete input payloads, physical direction, color order, opaque feature fields, header byte 6 and device pacing still require observation. The dial implementation follows its own recovered packet layout and direction convention.

Capture Windows official-app startup, idle traffic, dial movement and press/release, brightness changes, static test images, and profile changes. Establish input framing and initialization first. Determine display encoding, dimensions, orientation, chunk headers, acknowledgements, and timing from observations rather than analogies.

Keep original captures, timestamped action logs, annotated fixtures, and evidence links. Replay only understood ordinary operations. Firmware and bootloader writes are outside this milestone.

Gate: macOS reads dial gestures and repeatedly displays known patterns while input remains responsive.

### 5. Reliable device sessions

Use a single owner for each device session to coordinate input and commands. Add bounded queues, explicit overflow reporting, bounded retries/timeouts, reconnect, and reinitialization. Identify modules by serial where available, with documented fallback when serials are missing or duplicated; paths may change.

The live service owns input and mode cleanup on one thread, normalizes arrival-timestamped reports before a 64-report queue, and uses one action/log worker. Overflow, action failure, or stalled output cancels pending work. Normal limits use a finite drain; subprocess cancellation and one-shot dial cleanup are explicit. Worker/log deadlines do not bound synchronous native HID feature cleanup. Reconnection and OS lifecycle observation remain pending.

`DialSession` now provides explicit activation, suspension, caller-timed two-second resume and shutdown, clearing stale decoder state on transport failure. It does not subscribe to macOS notifications or reconnect automatically; firmware queries and display refresh remain caller operations. Optional host gesture tracking classifies a press/release at the application's 600 ms threshold. Actual lifecycle reliability and the remaining automatic service behavior are pending.

Gate: base unplug/replug, module reinsertion, app restart, and sleep/wake recover without duplicate actions, stuck presses, or stale sessions. Fixture tests can prepare these behaviors, but actual lifecycle acceptance requires hardware.

### 6. Everyday macOS application and release

The reusable `cooltek-sdk-service` crate and `masterhub-service` CLI provide profiles, binding lookup, normalized input, and explicit macOS actions. Add reliable sessions and a small menu-bar UI once physical input and action behavior have been validated. Document permissions only for actions that require them. Tauri is an option; defer the GUI choice until device APIs are usable. System/app audio integrations need separate feasibility checks; do not equate generic volume control with arbitrary per-app mixing.

The [controlled macOS smoke guide](masterhub-macos-smoke.md) provides device-free audio snapshot/restore checks and a scratch keystroke procedure. Observed effects, system-tool completion, and unresolved focus/output behavior are recorded separately from physical-device acceptance.

Add useful dial content after the display milestone. Package, sign/notarize, and document compatible firmware and tested Mac architectures.

Gate: all target controls work during a real usage session, including reconnect and sleep/wake. Track support separately for input, display, lighting, and lifecycle.

## Windows capture procedure

The simplest useful capture is Windows plus the real MasterHUB plus USBPcap/Wireshark. No proxy is required.

1. Identify the root hub and all devices/interfaces associated with the kit. Save app and firmware versions.
2. Start capture before reconnecting the kit so enumeration/descriptors are included.
3. Launch the official app and record startup plus an untouched idle baseline.
4. Make short, timestamped, one-change-at-a-time captures: each knob/dial one detent in each direction, faster movement, press/release, lighting/brightness, test images, profile switch, and module reinsertion.
5. Repeat experiments and compare both directions against the baseline. Separate counters/checksums/timestamps from meaning-bearing fields.
6. Preserve originals and convert confirmed observations into annotated fixtures. Match report kinds and IDs correctly; feature reports use control transfers, whereas input events and output writes can use other paths.

USBPcap captures USB transfers/request blocks rather than every electrical bus packet. Transfer payloads are generally the useful layer for application protocol reconstruction. An action configured in Windows may be implemented entirely in host software; USB captures alone do not reveal all host-side logic.

## Recommended first capture approach

| Approach | Assessment |
| --- | --- |
| Windows, real MasterHUB, USBPcap | Recommended first capture approach |
| Stock Pi 3B impersonating MasterHUB | Not possible through exposed USB ports: these operate as hosts; the micro-USB port is power input |
| Pi 3B Linux test host | Useful for direct SDK testing or a later network event bridge |
| Pi Zero 2 W emulator | USB peripheral support; its sole controller cannot simultaneously host the real kit |
| Pi 4 active proxy | Plausible research path using USB-C toward Windows and independent USB-A host ports toward the real device; arrange adequate power |

Adding a separate USB device-controller board could make the Pi 3B part of a two-board proxy, but this is considerably more work than capture on Windows. GPIO/configuration alone cannot add the missing exposed peripheral USB connection.

An emulator needs more than copied VID/PID: matching descriptors, interface layout, strings, serial/firmware/module responses, acknowledgements, and timing may be required before the official application sends useful commands.

Topology matters. A single composite device is different from independently enumerated devices behind a hub. Raw Gadget cannot normally emulate a hub using common USB device controllers. Inventory the real topology before selecting proxy hardware; one module may be a more feasible target than the complete kit.

Linux Raw Gadget plus libusb is a potential laboratory proxy foundation, not a component of the shipping macOS SDK. Do not assume a generic keyboard `g_hid` recipe implements the vendor feature/control protocol: available handling depends on the kernel and configuration. A proxy is justified later for response substitution, handshake investigation, or reproducible device simulation.

Captures can establish how to reproduce existing hardware operations. They do not prove new firmware-level features can be added or that firmware modification is necessary.

## Sources

- [Cooler Master product and module ecosystem](https://www.coolermaster.com/en-global/products/masterhub.html)
- [Official MasterHUB software page](https://www.coolermaster.com/en-global/software-masterhub.html)
- [Upstream SDK](https://github.com/projectzero7/cooltek-sdk)
- [Protocol probe](https://github.com/projectzero7/cooltek-sdk-probe)
- [M003K input notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/m003k/input_reports.md)
- [M003K feature notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/m003k/feature_reports.md)
- [Probe module metadata](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/metadata.md)
- [M4315 output notes](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/m4315/output_reports.md)
- [HIDAPI macOS support](https://github.com/libusb/hidapi)
- [Rust HIDAPI shared access](https://docs.rs/hidapi/latest/hidapi/)
- [HIDAPI report operation semantics](https://docs.rs/hidapi/latest/hidapi/struct.HidDevice.html)
- [USBPcap capture guidance](https://desowin.org/usbpcap/tour.html)
- [Wireshark USB capture](https://wiki.wireshark.org/CaptureSetup/USB)
- [Raspberry Pi official OTG whitepaper](https://pip-assets.raspberrypi.com/categories/685-app-notes-guides-whitepapers/documents/RP-009276-WP-1-Using%20OTG%20mode%20on%20Raspberry%20Pi%20SBCs)
- [Raspberry Pi USB hardware documentation](https://www.raspberrypi.com/documentation/computers/raspberry-pi.html#usb)
- [Raw Gadget limitations](https://github.com/xairy/raw-gadget#limitations)
- [Linux Raw Gadget API](https://docs.kernel.org/usb/raw-gadget.html)
- [Pi 4 Raw Gadget/libusb proxy example](https://github.com/AristoChen/usb-proxy)

## Current acceptance status

- Baseline macOS compilation: passed on Apple Silicon.
- Device communication and physical control/display/lifecycle tests: pending hardware.
- Diagnostic implementation: complete; see the [diagnostic guide](masterhub-diagnostics.md), [implementation status](masterhub-implementation-status.md), and [protocol evidence](masterhub-protocol-evidence.md).
- Official-software static analysis, recovered SDK capabilities, and CLI integration: complete and offline-validated. Hardware-dependent milestones remain pending.
- Binding/action service: implemented with synthetic replay, simulated transports, and mock host processes. Device input, actual host actions, throughput, and permissions require runtime acceptance.
