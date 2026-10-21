# MasterHUB diagnostic tools

`masterhub-diag` inventories Cooler Master HID interfaces, records raw input for one explicitly selected interface, and replays captures without hardware. It also previews dial feature commands, converts static artwork, and plans JPEG output offline. M003K decoding comes from public protocol notes; M131E, M4315, M005S and M032LE decoding and IPS mode commands come from static analysis of the official Windows software. Physical-device behavior is not yet verified.

## Build and replay without a device

From the repository root:

```sh
cargo build -p cooltek-sdk-cli --locked
cargo run -p cooltek-sdk-cli --locked -- --help
cargo run -p cooltek-sdk-cli --locked -- replay crates/cooltek-sdk-cli/tests/fixtures/m003k-synthetic.jsonl
cargo run -p cooltek-sdk-cli --locked -- replay crates/cooltek-sdk-cli/tests/fixtures/m131e-synthetic.jsonl
cargo run -p cooltek-sdk-cli --locked -- replay crates/cooltek-sdk-cli/tests/fixtures/m131e-software-derived-synthetic.jsonl
```

Replay constructs no HIDAPI context, opens no USB interfaces, and sends no commands. `m003k-synthetic.jsonl` covers clockwise/counterclockwise rotation, press/release transitions, unchanged snapshots, and unknown/malformed packets. `m131e-synthetic.jsonl` covers an unsupported channel. `m131e-software-derived-synthetic.jsonl` covers RIGHT/LEFT rotation, button transitions, malformed/unknown packets, and the 600 ms gesture threshold. The three additional-module fixtures exercise keys, absolute fader positions and roller directions; see their [provenance](../crates/cooltek-sdk-cli/tests/fixtures/README.md). All input fixtures are synthetic.

Decoded output retains the source label, module identity, relative timestamp, and raw hex bytes. Knob indices are zero-based (0, 1, 2); their physical placement still needs checking. Consecutive identical rotation reports are separate events. Repeated press snapshots produce only state transitions. A new decoder assumes all buttons are released and is scoped to a single recording.

Dial directions use the application's `right`/`left` labels; their physical clockwise mapping remains unknown. `gestures` contains `short_press` or `long_press` on release, with elapsed `held_micros`. A long press is at least 600 ms. This is host policy from the application, not a firmware event. Malformed/unknown input retains raw bytes and does not change button state.

## Device inventory

```sh
cargo run -p cooltek-sdk-cli --locked -- list
```

Inventory enumerates all Cooler Master VID `0x2516` HID interfaces, including unknown products, and prints JSON. It does not open devices, send feature queries, or initialize software mode. USB enumeration serials may differ from the vendor's feature-report serials.

Each entry includes a lossless `path_hex`, VID/PID, serial and manufacturer/product strings where available, release number, interface number, usage page, usage, and recognized model. A recognized model is metadata, not proof that its capabilities are implemented. Save the inventory alongside the firmware/app/version notes in the first hardware session.

## Bounded input recording

Copy the exact `path_hex` for the intended interface from inventory. Use a new filename for each experiment:

```sh
mkdir -p captures
cargo run -p cooltek-sdk-cli --locked -- record \
  --path-hex HEX_FROM_INVENTORY \
  --output captures/knobs-baseline.jsonl \
  --duration-secs 30 \
  --timeout-ms 100
```

The tool refuses to overwrite existing files or fall back to a first VID/PID match. Opening rechecks device identity. A report descriptor read is attempted and its result/error is preserved. Descriptor queries are reads; the default capture sends no configuration or software-mode commands.

Recording stops at its deadline even when no reports arrive. Add `--max-reports 100` to stop earlier after 100 nonempty reports. Only the actual received bytes are saved, with a maximum of 4096 bytes per input report. Timeouts mean idle; transport failures end the recording with an error footer and a failing exit status.

For either IPS module, after a neutral baseline has been saved, `--software-mode` explicitly enables software mode. For example:

```sh
cargo run -p cooltek-sdk-cli --locked -- record \
  --path-hex DIAL_PATH_HEX_FROM_INVENTORY \
  --output captures/dial-software-mode.jsonl \
  --duration-secs 30 \
  --software-mode
```

`--software-mode` is accepted only for M131E (`2516:0206` or `2516:9994`) or M4315 (`2516:01fe` or `2516:9997`) and sends a 32-byte feature report: `03 49 01` for the dial or `03 3a 01` for keys, followed by zero padding. The session records that initialization was requested. After recording ends, including an ordinary read failure, the tool attempts `03 49 00` (dial) or `03 3a 00` (keys) to restore hardware mode. A cleanup failure produces a failing exit status and an explanatory stderr message; the capture footer describes input recording, not successful mode restoration. Process termination cannot guarantee cleanup. Default captures send neither command. Physical recovery behavior still needs checking.

Record one physical action at a time and keep a timestamped action log. macOS input recording captures the SDK's reads; it is not a sniffer of another application's USB traffic. Use the [Windows USBPcap procedure](masterhub-macos-plan.md#windows-capture-procedure) to investigate official-app commands.

## Read-only Feature report probe

`features` reads the Feature reports that one interface declares. It is meant for comparing modules whose input reports differ, for example an M032LE that sends no input reports next to an M003K with the same descriptor layout apart from Feature IDs 0x32 and 0x34 (the M003K declares them, the M032LE does not):

```sh
cargo run -p cooltek-sdk-cli --locked -- features --path-hex HEX_FROM_INVENTORY --report-id 0x2d
cargo run -p cooltek-sdk-cli --locked -- features --path-hex HEX_FROM_INVENTORY
```

The guarantee is read-only at the HID protocol level: the host sends only GET_REPORT(Feature) requests. What the firmware does internally when it answers a GET is unknown. For a first run on an untested module, read one ID at a time with `--report-id`, and skip Feature ID `0x03`: it is the vendor command mailbox (this SDK sends commands with SET `0x03` elsewhere, for example software mode). macOS opens the interface exclusively, so stop `masterhub-service` and the vendor app first.

Usage: `masterhub-diag features --path-hex <PATH_HEX> [--report-id <ID>] [--timeout-ms <1-5000, default 1000>] [--duration-secs <1-60, default 10>]`

The command is read-only. `--path-hex` is required and selects exactly one inventory entry with the same lossless path format and checks as `record`; there is no automatic selection. The interface is opened the way `record` opens it, without software mode or any initialization. The tool reads the HID report descriptor, lists the Feature report IDs and sizes it declares, and sends one HID GET FEATURE per declared ID, with a buffer sized from the descriptor (payload bits rounded up to bytes, plus the report-ID byte). It never sends SET FEATURE, output reports, or mode commands. It never requests an ID the descriptor does not declare as a Feature report, so input and output report IDs are never requested. `--report-id` (decimal or `0x` hex) reads only that ID, and only if the descriptor declares it as a Feature report; otherwise nothing is requested and the command fails.

This is enforced in the types, not just by convention. The probe is generic over `ReadOnlyFeatureTransport`, which has only a descriptor read and a GET FEATURE. The opened `HidDevice` is moved onto a worker thread that accepts only those two requests, and nothing else holds a handle to it. The worker body is generic over a private trait with only those two reads, so the compiler rejects any other device call there. Each request waits at most `--timeout-ms`, capped by the time left before the `--duration-secs` deadline. A request that times out is an `error`, and it leaves the worker busy: later reports are not sent and are reported as `"status":"not_sent"` with a `reason`, never as `error`. Requests not started before the deadline are reported as `skipped`. A failed GET is recorded and the remaining IDs are still read.

Output is JSON lines on stdout. Shape (values abbreviated):

```json
{"type":"features_session","schema_version":1,"source":"hardware","read_only":true,"identity":{"path_hex":"…","vendor_id":9494,"product_id":516,"serial_number":"…","module":"M032LE","…":"same fields as list"},"timeout_ms":1000,"duration_ms":10000,"report_descriptor":{"status":"available","length":160,"raw_hex":"050c…"},"declared_feature_ids":[3,45,46,47,48,49,54],"declared_feature_reports":[{"report_id":3,"declared_bits":248,"declared_length":32},"…"],"requested_report_ids":[3,45,46,47,48,49,54]}
{"type":"feature","report_id":3,"declared_length":32,"status":"ok","received_length":32,"raw_hex":"03…"}
{"type":"feature","report_id":45,"declared_length":32,"status":"error","error":"…"}
{"type":"feature","report_id":46,"declared_length":32,"status":"not_sent","reason":"an earlier request is still pending on the device"}
{"type":"features_summary","requested":7,"ok":5,"error":1,"skipped":0,"not_sent":1,"elapsed_micros":12345,"status":"completed"}
```

`raw_hex` contains only the bytes the transport actually returned. `received_length` can be smaller than `declared_length`. The identity carries the same fields as `list`, including the USB serial number, so review it before sharing output. If the descriptor cannot be read or parsed, or `--report-id` is not declared, the session line carries the error, no GET FEATURE is sent, the summary has `"status":"aborted"`, and the exit status is non-zero. Per-report results do not change the exit status: it is 0 even when every GET failed, so scripts should read the `features_summary` line (`ok`, `error`, `skipped`, `not_sent`). The descriptor parser and the probe are covered offline by tests that use the M032LE, M003K and M131E descriptors captured with `record`. What the devices return to these requests has not been verified.

## Capture format and replay

The diagnostic format is JSON Lines, schema version 1, and is separate from USBPcap PCAP/PCAPNG. Converting USB captures to these fixtures requires identifying the appropriate input-transfer payloads; this CLI does not parse PCAP files.

The first line is a `session` with provenance (`hardware` or `synthetic`), wall-clock start time, identity, initialization request, and descriptor result. Subsequent `report` records contain microseconds since the session start and contiguous ASCII hexadecimal input bytes. A final `end` record includes count, timestamp, termination reason, and a transport error if applicable.

Synthetic M003K frame assembled from the protocol notes: `012a0500000301ffff`. Its outer ID is `01`, channel tag `2a`, little-endian payload length `0500`, and logical payload `000301ffff`.

Replay validates schema, bounded line sizes (65536 bytes), hex lengths/characters, nonempty reports, nondecreasing timestamps, a single session, matching report count, and the final newline/footer. Truncated captures are rejected, even if earlier reports were printed. A transport-error recording can be replayed for diagnosis; the ending summary preserves its failure reason. Check the replay exit status before using its output as a complete session.

Malformed known packets are emitted as `parse_error` diagnostics without changing button state. Unknown report IDs/channels are emitted unchanged. Decoding is selected by actual VID/PID, including both PID generations; the capture's model label does not override USB identity. No actions are dispatched by replay.

## Offline dial commands and JPEG plans

These commands build feature reports without constructing HIDAPI or opening a device:

```sh
cargo run -p cooltek-sdk-cli --locked -- dial mode software
cargo run -p cooltek-sdk-cli --locked -- dial mode hardware
cargo run -p cooltek-sdk-cli --locked -- dial backlight 50
cargo run -p cooltek-sdk-cli --locked -- dial color 123456
```

Each prints `feature_preview`, `source: software_derived`, and the complete 32-byte `raw_hex`. Backlight accepts 0–100. Color accepts exactly three raw channel bytes in SDK order; RGB ordering is not established. Append `--path-hex DIAL_PATH_HEX_FROM_INVENTORY` to explicitly open that exact dial and send the report. These operations do not perform serial queries, automatic initialization, or firmware updates. A successful feature/output write means HIDAPI completed the operation; it is not a verified display acknowledgement.

To inspect firmware and backlight, select a dial:

```sh
cargo run -p cooltek-sdk-cli --locked -- dial info --path-hex DIAL_PATH_HEX_FROM_INVENTORY
```

The result preserves four raw firmware bytes and eleven text bytes, as well as a lossy text interpretation. Query reply byte 1 is opaque; only actual returned lengths and required fields are checked.

Prepare a static PNG or JPEG without a device:

```sh
cargo run -p cooltek-sdk-cli --locked -- dial prepare-image artwork.png \
  --output /tmp/prepared-240x240.jpg

cargo run -p cooltek-sdk-cli --locked -- dial prepare-image artwork.png \
  --output /tmp/prepared-rotated.jpg --fit cover --rotate 270 \
  --quality 90 --background 202020
```

Preparation defaults to quality 100, no rotation, and `contain`: preserve the complete image with centered black margins. `cover` crops a centered square; `stretch` scales both axes independently. Rotation accepts 0, 90, 180, or 270 degrees clockwise, applied after fitting. `--background` is an RGB matte for transparency and contain-mode margins. Alpha is composited before filtering. A JPEG's EXIF orientation (or a PNG `eXIf` chunk before the image data) is applied to the source before fitting (an explicit rotation then turns the fitted square); color profiles are not applied or copied. Progressive JPEG source images are decoded and re-encoded as baseline output. BMP is decoded like a PNG. APNG, GIF and other formats are rejected here; the desktop app prepares GIFs as key or dial animations.

Source files are bounded to 16 MiB; decoded dimensions cannot exceed 4096 on either axis and raw decoded pixels cannot exceed 64 MiB. Decoder allocation limits are also requested, but are [best effort](https://docs.rs/image/0.25.10/image/struct.Limits.html), not a total process-memory cap. Complete decoding, fitting, encoding, and dial-framing validation finish before a new output file is created. Existing files are never overwritten. Preparation does not enumerate HID or establish physical image orientation, color calibration, or device acceptance.

An offline display plan requires a prepared 240×240, 8-bit, three-component baseline JFIF JPEG with one interleaved scan, and a binary HID descriptor containing output report ID `02`. The packet builder checks JPEG markers, tables, dimensions, and framing, but does not entropy-decode or resize the image; use `prepare-image` for conversion. A generated JPEG and synthetic descriptor are included for offline testing:

```sh
cargo run -p cooltek-sdk-cli --locked -- dial jpeg \
  crates/cooltek-sdk-cli/tests/fixtures/dial-240x240-baseline.jpg \
  --descriptor crates/cooltek-sdk-cli/tests/fixtures/dial-report2-65-byte.hid \
  --output /tmp/masterhub-jpeg-plan.jsonl
```

Use a new output path each time. Invalid inputs fail before file creation, and existing files are never overwritten. The output is a distinct `jpeg_plan` JSONL format, followed by `output_report` packets and a `jpeg_plan_end` count; it is not an input capture and cannot be fed to `replay`. Report length comes from descriptor items, including the report-ID byte. The recovered eight-byte header carries sequence, chunk length, and final flag. Unresolved byte 6 and unused padding are deliberately zeroed; their physical acceptance remains pending. The synthetic descriptor's 65-byte length is a test value, not a claim about the device.

To explicitly upload a prepared JPEG on hardware:

```sh
cargo run -p cooltek-sdk-cli --locked -- dial jpeg prepared-240x240.jpg \
  --path-hex DIAL_PATH_HEX_FROM_INVENTORY
```

The live path reads the selected interface's descriptor, validates the image and packet sizes before the first output write, and stops on a short write or transport failure. It does not automatically enter software mode, retry, wait for a protocol acknowledgement, or guess a fixed report size. Hardware captures must establish the correct interface, mode, byte 6, image acceptance/orientation, and any required pacing.

## Per-LED lighting commands (knobs, faders, base)

The knob module, fader module and base each have LEDs the official application lights through output report `02` (commands recovered statically; see the [protocol notes](research/masterhub-additional-modules-1.0.0.113.md#led-commands-knobs-faders-base)). Without `--path-hex` these commands only print the report:

```sh
cargo run -p cooltek-sdk-cli --locked -- led color ff8800 --module knob --index 3
cargo run -p cooltek-sdk-cli --locked -- led brightness 60 --module slider --index all
cargo run -p cooltek-sdk-cli --locked -- led mode 1 --module knob
```

`--module` is `knob` (indexes 0 to 4), `slider` (0 to 2) or `base` (index 0 only); `--index` is a number or `all` (the default, refused for the base). Colour takes three raw channels in the official app's order (red, green, blue is the assumption, not verified), brightness 0 to 100, mode 0 (off) to 9. `--save` asks the firmware to keep a colour or mode; leave it off while experimenting. The preview uses a 64-byte report.

Add `--path-hex PATH_FROM_INVENTORY` to send. The path must name exactly one inventory entry of the matching module (a knob path with `--module slider` is refused before anything is written), the report length comes from that interface's descriptor, and a short write is an error. Nothing is read back, there is no acknowledgement, and no software mode is involved. macOS opens the interface exclusively, so stop `masterhub-service` and the app first.

To learn what each index drives, sweep one module:

```sh
cargo run -p cooltek-sdk-cli --locked -- led sweep --module knob --path-hex PATH_FROM_INVENTORY
```

It turns every LED off, lights each index in turn for `--hold-ms` (default 2000, red by default through `--color`), prints one `led_sweep` line per index, and leaves the module dark. Watch the hardware and note which part lights for each index. Without `--save` nothing is written to firmware, so unplugging and replugging the module restores its own lighting.

## Offline keypad image preparation and plans

The 15-key IPS module uses a separate per-key header and 72×72 images. Prepare static PNG/JPEG artwork with the same fit, matte, rotation, source limits, and no-overwrite rules as dial preparation:

```sh
cargo run -p cooltek-sdk-cli --locked -- keypad prepare-image artwork.png \
  --output /tmp/prepared-72x72.jpg

cargo run -p cooltek-sdk-cli --locked -- keypad jpeg \
  crates/cooltek-sdk-cli/tests/fixtures/keypad-72x72-baseline.jpg \
  --key-index 0 \
  --descriptor crates/cooltek-sdk-cli/tests/fixtures/dial-report2-65-byte.hid \
  --output /tmp/masterhub-keypad-plan.jsonl
```

Key indices are zero-based, 0–14; their physical positions remain unverified. Preparation defaults to quality 100, contain fit, black background, and no rotation. Its baseline JFIF output policy follows the pinned Windows renderer's 72×72 JPEG path. The packet builder validates an 8-bit, three-component baseline JFIF JPEG with one interleaved scan; it checks framing and dimensions without decoding entropy data. These conservative constraints do not establish every format the firmware accepts.

`keypad jpeg` requires a saved binary descriptor containing output report ID `02` and a new output path. It has no live-device option, constructs no HIDAPI context, and sends no packets. Image, key index, and descriptor validation complete before file creation. The example reuses a synthetic 65-byte descriptor; its filename reflects the original dial tests, and its length is not hardware evidence.

Plans identify `module: m4315` and `key_index` in their header, packet records, and footer. The eight-byte header is `02 36`, followed by a little-endian 16-bit sequence, a little-endian 16-bit payload length, the key index in byte 6, and the final flag in byte 7. Payload starts at byte 8; unused bytes are zeroed. Report lengths are bounded to 9–65,535 bytes and images to 16 MiB, with at most 65,536 chunks to prevent sequence wrap. The JSONL plan is separate from input captures and cannot be passed to `replay`.

Physical upload, key order, orientation, image acceptance, software-mode requirements, and refresh pacing remain hardware checks. The desktop app sends images to module displays only while Settings > Let OpenStreamHub drive the display is App, its default (see [display sync](masterhub-display-sync.md)); this CLI path stays offline-only except for an explicit `dial jpeg --path-hex`.

## SDK building blocks

- `DeviceRegistry` shares enumeration context and opens exact identities without initialization.
- `ModuleHandle::open_by_identity()` opens without configuration; `initialize()` and `connect_by_identity()` explicitly initialize. The legacy `connect()` still opens the first match and initializes for compatibility.
- `descriptor::declared_feature_reports()` lists every Feature report ID with its bit size and HIDAPI buffer length, without I/O.
- `ReportTransport` supports physical or fake input/feature/output operations and descriptor reads. `InputReader` owns a bounded reusable read buffer and distinguishes idle from failure.
- `CanGetSerialNumber::parse_serial_number_report()` and `get_serial_number_with_transport()` validate serial responses and support offline tests.
- `CanSetSoftwareMode::software_mode_report()` builds the known feature command without I/O; writing it is an explicit separate operation.
- `input::parse_m0_report()` handles the documented outer framing; `parse_m003k_payload()` handles logical knob payloads. `M003KDecoder` adds stateful press transitions.
- `input::parse_m131e_report()`, `parse_m131e_payload()`, and `M131EDecoder` decode the recovered dial queue without I/O.
- `dial` exposes pure feature builders, bounded query parsers, `JpegPackets`, and explicit transport operations. Typed M131E handles expose the corresponding commands and descriptor-sized `display_jpeg()`.
- `keypad` exposes software-mode builders/setters, 72×72 image constants, `validate_jpeg()`, and a pure `JpegPackets` iterator with a required key index. Packet planning performs no I/O. `keypad::display_jpeg` and `display_jpeg_with_pacing` write one key image to a transport; the CLI has no live key upload, and only the experimental `masterhub-service --display-sync` uses them, never run on hardware.
- `DialSession` provides explicit activation, suspension, resume after a caller-timed two-second delay, input reads, and shutdown; drop performs no commands. `DialGestureTracker` accepts monotonic timestamps and classifies known releases at 600 ms.

Automatic macOS sleep/wake observers, reconnect, animation, and firmware writing remain later work. The separate [binding service](masterhub-service.md) handles explicit host actions; image preparation is offline. Lifecycle hooks and packet builders have offline validation; device acceptance remains pending.

## Validation

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

These checks and the CLI integration tests use no attached device. They establish parser, transport-boundary, and replay behavior, not hardware readiness. Keep real captures in the ignored `captures/` directory and preserve the original evidence separately when needed; synthetic test fixtures belong in the versioned fixture directory. Capture metadata includes device paths and serial numbers; review those fields before sharing a capture.
