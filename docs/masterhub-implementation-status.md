# MasterHUB implementation status

Updated on 3 October 2026. The SDK implements discovery, input decoding, dial controls, JPEG output, and session lifecycle hooks. The CLI prepares static images offline. A separate binding service adds profiles and explicit macOS actions. Automated tests use synthetic reports, simulated transports, and mock host processes; limited device-free macOS checks are recorded in the [smoke guide](masterhub-macos-smoke.md). Hardware compatibility and complete action acceptance remain pending. See the [roadmap](masterhub-macos-plan.md), [diagnostic guide](masterhub-diagnostics.md), and [service guide](masterhub-service.md).

## References

- [macOS roadmap](masterhub-macos-plan.md), including the Windows capture procedure and Raspberry Pi USB constraints.
- [Capability evidence table](masterhub-protocol-evidence.md), distinguishing implementation, synthetic tests, and hardware acceptance.
- [Official MasterHUB 1.0.0.113 static analysis](research/masterhub-1.0.0.113-static-analysis.md), with package/payload hashes, installed-name mappings, recovered input/display fields, newer PIDs, and reproduction procedure. No Windows executable or native addon was run.

## Implemented

- Shared HID discovery with lossless paths and interface/usage metadata. Exact identity opening verifies the selected enumeration snapshot and the opened handle before initialization is available.
- Module PID aliases: base `01f6`/`9999`, knobs `0202`/`9996`, and dial `0206`/`9994`, with vendor `2516`. Discovery and typed handle matching accept both generations. Original `PRODUCT_ID` constants remain unchanged; custom modules can override `Module::accepts_product_id`.
- Separate neutral opening and explicit initialization on typed module handles. The serial example no longer initializes the IPS dial as a side effect.
- Fakeable report transport and bounded input reader; actual received lengths are respected, idle timeouts and transport errors are distinct, and input reading performs no feature queries/configuration writes.
- Safe serial response parsing with actual-length, header, report-ID, and payload bounds validation. Existing lossy UTF-8/NUL-padding behavior is preserved.
- Pure M0 framing and M003K logical parsing with typed rotation and press/release transitions. Malformed reports never partially change state; unknown channels/report IDs retain original bytes.
- Bounded M131E framing and a stateful dial decoder. Queue 0 produces application RIGHT/LEFT rotation labels and press/release transitions; physical clockwise/counterclockwise mapping remains unknown. Ignored payload byte 1 and extra bytes are retained. Other queues, unsupported selectors, and unknown layouts retain the original raw report.
- Dial feature builders and explicit transport/typed-handle methods for software/hardware mode, backlight, solid color channels, backlight queries, and firmware information selectors. Replies are bounded by the actual returned count. Reply byte 1 and firmware bytes retain unresolved semantics; no Windows returned-count increment is copied. Color channel order remains unverified.
- Pure HID descriptor inspection and descriptor-sized JPEG output. The packet iterator accepts an encoded 240×240, 8-bit, three-component baseline JFIF JPEG with one interleaved scan; it checks markers/dimensions but does not decode entropy data. Chunk sequence/length fields and final flags follow the recovered SDK layout. Header byte 6 and unused padding are zeroed deliberately. Typed display upload queries the selected interface's descriptor and fails if its output size cannot be established; short writes stop without retry or inferred acknowledgement.
- Explicit `DialSession` activation, suspension, caller-timed resume, input reads, and shutdown. Construction/drop perform no commands; firmware queries and display refresh remain explicit caller operations. The two-second resume delay is application-derived policy. `DialGestureTracker` optionally classifies a known press/release at the app's 600 ms threshold using caller-provided monotonic timestamps; it does not invent a firmware long-press event.
- `masterhub-diag list`, `record`, and `replay`. Recording has a finite deadline, exact-path selection, no-overwrite output, descriptor result, provenance, and error footer. Software mode is an explicit M131E-only option.
- Strict, versioned JSONL replay with bounded records, count/timestamp/footer checks, original raw reports, explicit packet parse errors, and final capture status. Replay never opens hardware or dispatches actions.

## Dial CLI integration

- Replay selects M003K or M131E decoding using actual VID/PID, including legacy and newer aliases, rather than a potentially incorrect model label. Dial output preserves raw bytes and includes optional host gestures at the 600 ms boundary.
- `dial mode`, `dial backlight`, and `dial color` preview complete feature reports by default. Only an explicit `--path-hex` opens and writes to an exact selected dial. `dial info` explicitly queries firmware fields and backlight.
- `dial jpeg` plans packets offline from a saved HID descriptor and encoded JPEG, or explicitly writes to a selected dial using its descriptor. The JSONL plan has separate provenance/schema from input captures. Inputs are validated before plan creation or the first display write; existing output files are never overwritten.
- `dial prepare-image` decodes static PNG/JPEG, fits and optionally rotates/mattes the image, and encodes 240×240 RGB baseline JFIF. Source bytes, dimensions, and decoded pixel buffers are bounded; APNG and unsupported formats are rejected. EXIF orientation and color profiles are not automatically applied. Full conversion and framing validation precede no-overwrite file creation; no HID context is constructed.
- Opt-in recording attempts hardware-mode restoration after success, read failure, or even a failed initial software-mode write. Primary capture/enable errors, footer failures, and cleanup failures remain visible. The input footer alone does not establish successful cleanup.
- Help, examples, fixture provenance, and command constraints are documented in [masterhub-diagnostics.md](masterhub-diagnostics.md). Offline paths do not enumerate or open hardware.

## Binding and action service

- Separate `cooltek-sdk-service` crate with strict schema-1 profiles, bounded action validation, serial-specific precedence, input normalization, and the host dispatcher boundary.
- `masterhub-service validate`, `replay`, and `run`. Replay constructs no HID context and always plans actions. Live runs own one exact selected interface, log planned actions by default, and require `--execute` for host operations. Dial software mode is a separate explicit option.
- macOS system output volume/mute, ASCII keyboard shortcuts through System Events, and application launch by bundle ID. The backend uses absolute system binaries with argument validation, five-second subprocess timeouts, bounded output, and no retries.
- Public `AudioSnapshot`, `audio_snapshot()`, and `restore_audio_snapshot()` helpers support bounded state queries and exact best-effort restoration. Device-free smoke examples verify audio effects and restoration, or request a TextEdit launch/plain scratch keystroke; explicit execution is required. Native command completion is distinguished from independently observed effects.
- Live input is timestamped and normalized independently of the action worker. Owned reports enter a 64-report FIFO queue; overflow ends the session with explicit received/queued/processed/discarded counts. Actions remain ordered and serialized.
- Live logging uses a 500 ms total record deadline and bounded records on terminals, pipes, or sockets. A failed/partial record cannot be retried; regular-file redirection is rejected and an active `tee` pipeline can save output. The owned stdout duplicate restores status flags when dropped.
- Normal input limits allow a ten-second worker drain, followed by cancellation when necessary. Ctrl-C/SIGTERM and failures cancel queued work and in-flight native subprocesses; completion is observed before joining. Requested dial hardware-mode cleanup runs once after the finite worker phase and any software-mode attempt, including failed enabling. Primary, action, drain, output, and cleanup errors are preserved in the final status.
- Diagnostic path parsing now treats a lossless path as one required value, avoiding a Clap type mismatch on valid live arguments. Existing diagnostic commands remain the Cargo default binary.
- Tests cover input continuing during blocked actions, both gesture thresholds, rapid ordered bursts, queue overflow, blocked pipes, subprocess cancellation, failed-action stopping, and uncooperative-worker deadlines. Host arrival timestamps and queues do not establish device timing, sustained hardware throughput, or absence of HID buffering loss. HIDAPI feature cleanup is synchronous without an exposed timeout. No automatic reconnect, native sleep/wake observer, or UI is provided.

## Validation evidence

Validated on Apple Silicon macOS with Rust 1.98.1.

| Check | Result |
| --- | --- |
| Workspace/all-target compilation | Passed `cargo check --workspace --all-targets --locked` |
| Workspace/all-target tests | Passed 208: 79 SDK unit, 43 CLI unit, 38 CLI integration, 39 service unit, 9 audio example tests |
| Documentation tests | Passed 12, including compile-fail type-state coverage |
| Formatting and diff whitespace | Passed `cargo fmt --all -- --check` and `git diff --check` |
| Clippy across workspace/all targets with warnings denied | Passed |
| API documentation build with warnings denied | Passed |
| CLI help, synthetic replay, and offline planning smoke checks | Passed four feature previews, 16 dial reports, both gesture thresholds, and 31 JPEG packets at a synthetic 65-byte output length |
| Documentation links | Passed local link checks |
| Binding service | Passed strict profiles, serial precedence, knob/dial replay plans, gesture boundaries, mock host commands/timeouts, live cleanup and cancellation tests |
| Live runtime | Passed independent input/gesture timing, rapid bursts, overflow counts, bounded drain/cancellation, panic handling, one-shot cleanup, and 13 stdout deadline/descriptor tests |
| macOS script syntax | Seven backend AppleScript forms compiled with `osacompile`; live execution is scoped separately below |
| Image preparation | Ten unit and four CLI integration checks passed: entropy decoding, alpha/matte, fit/crop, all quarter-turns, corrupt/animated/limited sources, packet reassembly, preflight, and no-overwrite |
| Device-free macOS runtime | Passed volume/mute effects with one-point target tolerance and exact restoration on this host; TextEdit launch request completed; scratch keyboard effect remains unverified. See [masterhub-macos-smoke.md](masterhub-macos-smoke.md) |

The JPEG smoke plan reconstructs the generated 1,725-byte fixture exactly (SHA-256 `cd46c629dc680404454dc55dd564b815b5c225bed40c5e7af9e498bd0ab69600`). The 65-byte descriptor is synthetic and does not establish the device's real output size. Gesture smoke checks classify 600,000 µs as long and 599,999 µs as short.

An additional offline smoke converted an original synthetic 480×240 RGBA pattern with contain fit, 90-degree clockwise rotation, quality 90, and matte `202020`. Its 5,834-byte JPEG reconstructed exactly from 103 packets using the synthetic 65-byte descriptor (SHA-256 `c54a75f7ed8a582e888dd62b8135d74a3be44d794a47fed526812b0414fa5a02`). This is image-conversion and packet evidence, not a device capture or display acceptance.

Automated validation uses pure parsers, simulated transports, synthetic captures, and mock action subprocesses. Device-free host observations are scoped in the smoke guide. Physical devices, native Intel Mac, Windows, and Linux builds have not been tested.

## Remaining gates

The first hardware session must establish descriptors/topology, interface selection, actual serial/report framing, physical control order and gestures, required initialization, and native HID timeout/open behavior. Complete dial payloads, descriptor sizes, physical direction, color order, feature-field semantics, image acceptance/orientation, header byte 6, and pacing still require captures or device tests. Implemented aliases, decoders, commands, and packet construction are not hardware acceptance.

Automatic macOS sleep/wake observation and reconnect, base/knob lighting, complete action/permission runtime acceptance, UI, packaging, and firmware compatibility acceptance remain later roadmap milestones. Explicit lifecycle hooks do not constitute automatic device recovery. Static PNG/JPEG conversion is implemented; BMP, animation, and progressive JPEG output remain unsupported. Firmware flashing/recovery remains excluded.
