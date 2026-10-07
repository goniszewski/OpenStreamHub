# MasterHUB display synchronization (offline)

`cooltek_sdk_service::display_sync` decides which key and dial images a MasterHUB display needs, and applies those decisions to a transport-agnostic sink. It is the offline half of live display support. Everything in it is tested with an in-memory fake. **No hardware transport is wired, and nothing in the service binary, the CLI, or the desktop app calls it yet.**

## Design

### Sink

`DisplaySink` is the boundary that real hardware code will implement later:

- `capabilities()` reports `key_count` (0 to 15 keys, wire indexes from 0) and whether a `dial` display exists.
- `write_key_image(key_index, jpeg, between_chunks)` writes one complete 72×72 key image.
- `write_dial_image(jpeg, between_chunks)` writes one complete 240×240 dial image.

`between_chunks` has the same shape as the hook of `dial::display_jpeg_with_pacing`, so a hardware sink can pass it straight through. Errors are `SinkError::Transport`, `SinkError::ShortWrite { written, expected }`, or `SinkError::Unsupported(target)`. A sink must stop at the first error or short write and must not retry, which is the SDK's existing policy.

`FakeDisplaySink` records every attempt (`writes`), returns queued faults per target (`fail_next`, or `fail_after` to let some attempts succeed first), and simulates `chunks` reports per image so pacing hooks can be checked.

### Artwork providers

The module renders nothing. It asks caller-owned providers for prepared bytes:

- `KeyImageProvider::key_image(&KeyImageRequest) -> Option<Vec<u8>>`. The request has the wire `key_index` and a `KeyContent`:
  - `Empty { page_id }`: an empty slot. Return a blank image so the previous page's artwork is cleared.
  - `Action { page_id, label }`: an action tile.
  - `Folder { page_id, target_page_id, target_name }`: a folder tile. Its label is the child page's name.
  - `Back { depth, parent_page_id }`: the generated Back key in slot 0 of every child page. The artwork is a left arrow with an `L{depth}` badge for the **current** depth (1 to 8), as described in [keypad folders](masterhub-keypad-folders.md). Drawing the badge is the provider's job.
- `DialImageProvider::dial_image(&DialImageRequest) -> Option<Vec<u8>>`. The request carries opaque `profile` and `function` strings. The caller maps its current profile and dial function onto them.

`None` means no artwork is available. The key or dial is reported as missing and is not written, so it may keep showing an older image.

### Planning

Planning goes through `DisplaySync::plan_page(&ProfileSession, &dyn KeyImageProvider)`. It is pure apart from reading the coordinator's drawn state. The underlying `DisplayPlan::for_page` and `DialPlan::new` are crate-private, so every plan is bound to the coordinator that made it. It takes the session's validated schema-2 profile and current path, requests all 15 keys in ascending wire order, and validates every returned image with `keypad::validate_jpeg` (72×72 baseline JFIF). It keeps only keys whose bytes differ from the last complete write. Diffing compares exact bytes, so unchanged artwork is never resent and edited artwork behind an unchanged request is still redrawn. One invalid image fails the whole plan before anything is written. Schema-1 profiles return `NotKeypadProfile`.

`DisplaySync::plan_dial(request, provider)` does the same for the dial with `dial::validate_jpeg` (240×240). The masterhub crate now exports that validator; before this change it was private.

`DrawnState` holds the last completely written bytes per key and for the dial. `None` means unknown or dirty. Callers can read it but not change it. It also holds the owning coordinator's ID, plus separate key and dial generation counters. A generation changes on every update to that surface's state: each successful write, each failure, and each invalidation.

### Applying

`DisplaySync<S: DisplaySink>` owns a sink and its `DrawnState`:

- `plan_page`, `apply(plan)`, and `sync_page` (plan, then apply) for keys. `apply_dial(plan)` for the dial.
- Before any write, `apply` returns `StalePlan` for a plan made by another `DisplaySync`, or for one whose key generation no longer matches. A generation mismatch means a later write, failure or invalidation happened after the plan was made. For example, a plan made before an apply that left key 4 half-written is rejected, so it cannot report success over a partial image. `apply_dial` does the same with the dial generation. `apply` also returns `Unsupported` for any key index the sink's capabilities do not cover.
- Writes are sequential in ascending wire order. A successful write records the bytes. A failed write sets that target to dirty, because the device may now hold a partial image. A fresh plan therefore rewrites it even when the bytes are unchanged. Plans made earlier are rejected, as above.
- `RetryPolicy { transport_retries }` defaults to 0, so at most `transport_retries + 1` attempts are made (256 at most). `WriteFailure::attempts` is a `u16`. Only `Transport` errors are retried. A short write is never retried. Retries run back to back, with no pacing or backoff between them.
- A `Transport` failure is recorded in `ApplyReport.failures` and the remaining writes continue. A `ShortWrite` stops the plan. Every key not yet attempted is listed in `not_attempted` and stays dirty.
- `with_chunk_pacing(hook)` is forwarded to the sink between reports of one image. `with_image_pacing(hook)` runs between consecutive key writes within one plan. It receives the number of keys already attempted, including failed ones. It does not run between retries or for `apply_dial`. Neither hook is set by default, and no delay value has been established.

### Invalidation and resume

- `sink_mut()` gives direct access to the sink, for example to inject faults into the fake. Writes made that way bypass validation and tracking, so handing out the sink invalidates all drawn state.
- `invalidate()`: forget everything drawn (after a reconnect, a profile replacement, or a focused-app switch). The next plans redraw every key and the dial, and older plans are rejected.
- `suspend()`: the same, because the device returns to hardware mode before sleep.
- `resume()`: invalidates and returns `ResumeAction { steps: [EnterSoftwareMode, QueryFirmware, RefreshDisplay] }`. This is the official app's sequence after its 2 s settle delay, from [static analysis](research/masterhub-1.0.0.113-static-analysis.md). The caller performs the device steps. This module sends no HID reports. For `RefreshDisplay`, call `sync_page` and plan the dial again.

## Not wired, not verified

- **No live transport.** No `DisplaySink` exists for a real device. The SDK has no live key upload (`keypad::JpegPackets` is pure). Only the dial has `display_jpeg_with_pacing`.
- **No integration.** The service binary still rejects live schema-2 keypad runs. The CLI and the desktop app do not call this module. No artwork provider exists yet, including Back-badge rendering and a blank key image.
- **No hardware acceptance.** Physical key order (wire index to physical position), orientation, firmware image acceptance, header byte 6, pacing and acknowledgement, and the HID output report length are all unknown. Tests use the 72×72 fixture and framing-valid variants of it, including a 240×240 header rewrite whose entropy data is not valid for that size. They are not device captures.
- **Unknown timing.** Whether redrawing a full page is fast enough to feel responsive is unknown. The diff only reduces how many keys are written.

## Integration checklist for the live run path

1. **Hardware sink (masterhub crate).** Add an explicit live `keypad::display_jpeg_with_pacing(transport, key_index, jpeg, output_report_length, hook)` that mirrors the dial version: validate first, write `JpegPackets` with `write_output`, and stop on an error or short write. Implement `DisplaySink` over it and over `dial::display_jpeg_with_pacing`, using the output length parsed from the selected interface's report descriptor (`descriptor::output_report_length(descriptor, 2)`, as the CLI's `dial jpeg` does). Map a short write to `SinkError::ShortWrite` and every other error to `SinkError::Transport`. The dial helper currently reports a short write as a generic `HidError` message. The sink needs a typed short-write signal from the SDK, or it must report every dial failure as `ShortWrite` so that it is never retried.
2. **Software mode.** Enter keypad software mode before the first key write and leave it on exit. `live_runtime.rs` already does this when `LiveOptions::software_mode` is set (`crate::mode::set`). Display writes must require that option.
3. **Lift the rejection.** `crates/cooltek-sdk-cli/src/bin/masterhub-service.rs`, `live_bindings`, returns "nested keypad profiles are offline only until live display integration is implemented" for schema-2 M4315 runs. Remove that branch only once steps 1, 2, 4 and 5 exist. Keep it behind an explicit opt-in flag until hardware acceptance.
4. **Initial draw and navigation.** In `crates/cooltek-sdk-cli/src/live_runtime.rs`: after opening the keypad, call `sync_page` once. After every `KeypadResolution::Navigation`, call `sync_page` with the session's new path. Log each `ApplyReport` (written, failures, not_attempted) and each plan's `missing()` in the live JSONL records. A display failure must not dispatch or suppress actions. Navigation state is already final in `ProfileSession`.
5. **Artwork provider.** Build a `KeyImageProvider` from the profile's prepared artwork, reusing `image_prep` and the app's `artwork.rs` 72×72 output. It needs a blank image for `Empty` and a rendered Back key per depth L1 to L8. Prepare and cache the images before the run starts, so planning does no image conversion on the input path.
6. **Dial.** Map the active profile and dial function (see the dial function cycling work in `ProfileSession`) to `DialImageRequest`. Call `plan_dial` and `apply_dial` when either changes.
7. **Reconnect and profile switch.** Call `invalidate()` after a reconnect and whenever the profile is replaced, including a focused-app switch.
8. **Tauri session manager resume.** `apps/open-stream-hub/src-tauri/src/session.rs` stops the service on suspend and restarts it after wake (`Event::Resume`, 2 s settle). A restarted service process creates a new `DisplaySync`, which starts dirty and redraws everything on its first `sync_page`. If the service is ever kept alive across sleep instead, call `suspend()` on sleep and `resume()` after the settle delay, then perform its steps in order: software mode, firmware query (`dial::read_firmware_info` for the M131E), then `sync_page` and dial planning.
9. **Hardware session.** Before enabling by default: verify key order and orientation, image acceptance, pacing (start with no delay, then add `with_chunk_pacing` sleeps if images tear), the behaviour after a short write, and page-transition latency. Record the results in the [first hardware session guide](masterhub-first-hardware-session.md).
