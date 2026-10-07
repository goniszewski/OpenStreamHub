# MasterHUB display synchronization (offline)

`cooltek_sdk_service::display_sync` decides which key and dial images a MasterHUB display needs, and applies those decisions to a transport-agnostic sink. It is the offline half of live display support. Everything in it is tested with an in-memory fake. A HID-backed sink and a prepared artwork provider exist in the CLI crate, also tested only with fakes. The live runtime and `masterhub-service run` use them **only behind the experimental `--display-sync` opt-in** for M4315 keypad pages. The desktop app does not pass that flag. Nothing has been run on hardware.

## Design

### Sink

`DisplaySink` is the boundary that hardware code implements (`HidDisplaySink`, below):

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

## Hardware sink and artwork (implemented, wired behind an opt-in)

These pieces are tested only with fake transports. `masterhub-service run --display-sync` is the only caller over a real device.

- **Typed short write (masterhub).** `dial::display_jpeg`, `dial::display_jpeg_with_pacing`, and the new keypad writers return a write count that differs from the report length as `HidError::IncompleteSendError { sent, all }`, HIDAPI's typed variant. `cooltek_sdk_masterhub::ShortWrite::from_error(&error)` returns `Some(ShortWrite { written, expected })` for it and `None` for every other error. Public signatures are unchanged (`HidResult<usize>`); only the error variant and its message changed. A count larger than the report is treated the same way.
- **Live key writer (masterhub).** `keypad::display_jpeg(transport, key_index, jpeg, output_report_length)` and `keypad::display_jpeg_with_pacing(..., between_chunks)` mirror the dial writer: they validate the key index, the complete image and the report length before the first write, write `keypad::JpegPackets` in order with `write_output`, run the hook between reports (n − 1 calls), and stop at the first error or short write without retrying or reading an acknowledgement. They send no mode command. Both writers share one internal report loop.
- **Shared transports (masterhub).** `ReportTransport` is now implemented for `&T`, `Rc<T>` and `Arc<T>`, so one opened device can back both an `InputReader` and a display sink.
- **`HidDisplaySink<T: ReportTransport>` (`crates/cooltek-sdk-cli/src/display_sink.rs`).** Implements `DisplaySink` for an M4315 (15 keys, no dial) or an M131E (dial, no keys). Construction reads the descriptor once and fails unless output report `0x02` has a length from 9 through 65,535 bytes. It never guesses a length. A typed short write becomes `SinkError::ShortWrite`, so `DisplaySync` never retries it. Every other error becomes `SinkError::Transport`. A target the module does not have is `SinkError::Unsupported`. `open_hardware(registry, identity)` opens a second handle and is not called: macOS opens HID devices exclusively by default, so a second handle is not expected to work there.
- **`SharedTransport<T>` (`display_sink.rs`).** `HidDevice` is `Send` but not `Sync`. The planned `Rc<HidDevice>` would keep display writes on the input thread, where input reads would wait for every key of a page. Instead, one opened device sits behind a mutex shared by the input reader and the display thread:
  - Reads take the lock in slices of at most 5 ms and return a normal timeout while another holder has it. Input reports queue in the OS meanwhile, so an in-flight display write can delay input reads.
  - A writer yields to a waiting reader after at most 16 output reports since the reader's last turn. Tests check this ordering by counting.
  - Feature reports and descriptor reads (mode commands) go before writers and wait at most 2 s, then fail with `SHARED_BUSY`.
  - `write_gate()`/`close_writes()` make every later output write fail immediately, checked before and after taking the lock.
  - A backend call that blocks inside HIDAPI is still not interruptible.
- **`PreparedKeyArtwork` (`crates/cooltek-sdk-cli/src/display_artwork.rs`).** A `KeyImageProvider`. Schema-2 tiles have no image field, so it serves deterministic generated placeholders: black for `Empty`, a solid slate tile for `Action`, a folder silhouette for `Folder`, and for `Back` a left arrow over an `L1` to `L8` badge drawn with a built-in 5×7 bitmap font (no font files). Each image is drawn into a 72×72 RGB buffer, encoded through `image_prep::prepare_keypad_image`, checked with `keypad::validate_jpeg`, and cached by `PreparedKeyArtwork::prepare()` before a run. Serving an image copies cached bytes. Nothing is encoded per input event. `Back` depths outside 1 to 8 return `None`.

## Live integration (opt-in, not verified)

`run_live_with_display` in `live_runtime.rs` takes an optional `LiveDisplay` (a boxed `DisplaySink` and `KeyImageProvider`, so tests inject `FakeDisplaySink`). It requires software mode, an M4315 session, and a schema-2 profile. The sink and provider move to a dedicated `masterhub-display` thread, which owns its `DisplaySync` and a copy of the profile session and applies the newest requested page path with `set_path`, then `sync_page`. The input owner stores a request in a single slot (newest wins) and never waits. Each outcome becomes a `display_sync` JSONL record that the action worker logs, and `service_end` gets display counters. The record and fields are documented in the [service guide](masterhub-service.md#keypad-display-sync-experimental).

The display runs on its own thread instead of the action worker, so display failures never dispatch, suppress, or reorder an action. When the input loop ends, the runtime closes the write gate (a required `LiveDisplay::new` argument; `display_sink::live_display` builds the sink and passes the `SharedTransport`'s own gate, and the binary uses it), waits up to the cancellation budget for the display thread, and restores hardware mode with priority over display writes. If the lock is still busy, it retries the restore once. `service_end.display_stopped: false` signals that the display thread outlived cleanup behind the closed gate. Setup fails closed: a sink or display thread that cannot be created ends the run before software mode is entered. After that, a display failure is logged and never changes the exit status by itself.

## Not verified

- **Desktop app.** The app never passes `--display-sync`, so the app's live keypad runs still refuse schema-2 profiles.
- **No dial artwork provider.** There is no `DialImageProvider` yet.
- **No hardware acceptance.** Physical key order (wire index to physical position), orientation, firmware image acceptance, header byte 6, pacing and acknowledgement, and the HID output report length are all unknown. Tests use the 72×72 fixture and framing-valid variants of it, including a 240×240 header rewrite whose entropy data is not valid for that size. They are not device captures.
- **Unknown timing.** Whether redrawing a full page is fast enough to feel responsive is unknown. The diff only reduces how many keys are written. On a shared device, an idle input reader holds the lock for read slices of up to 5 ms with gaps of about 1 ms, and a writer sends at most 16 reports per gap. A page redraw may therefore be noticeably slow. Measure it on hardware.
- **Image cut short at session end.** When the write gate closes mid-image, that key's JPEG transfer is left incomplete just before the hardware-mode restore. How the firmware handles a partial image followed by a mode change is unknown and hardware-unverified.
- **Panics.** A panic inside the input loop is contained. The run then ends with `input loop panicked`, and cleanup, including the hardware-mode restore, still runs. A panic elsewhere on the coordinator thread (outside that loop) still unwinds without the restore.

## Integration checklist for the live run path

1. **Hardware sink (masterhub crate and CLI). Done, offline only.** See above: typed short write, `keypad::display_jpeg_with_pacing`, `HidDisplaySink`. Tested with fake transports for exact report bytes, stop on short write, pacing counts, nothing written for an invalid image, `ShortWrite` never retried by `DisplaySync` (even with `RetryPolicy { transport_retries: 3 }`), the chunk pacing hook reaching the HID writer, and unsupported targets.
2. **Software mode. Done, offline only.** `live_runtime.rs` enters keypad software mode when `LiveOptions::software_mode` is set (`crate::mode::set`, after the worker reports ready). The sink sends no mode command. A display requires software mode: the CLI rejects `--display-sync` without `--software-mode` before HID access, and the runtime rejects it again before starting the worker.
3. **Lift the rejection. Done behind an opt-in.** `live_bindings` keeps the schema-2 keypad tree for an M4315 only when `--display-sync` is given, and rejects the flag for any other module. Without the flag it still returns "nested keypad profiles are offline only until live display integration is implemented". Keep the flag opt-in until hardware acceptance (step 9).
4. **Initial draw and navigation. Done, offline only, with one deliberate change.** The planned `Rc<HidDevice>` sharing would keep display writes on the input thread, so the sink is shared through `SharedTransport` and runs on a dedicated display thread instead (see above).
   1. Caller (`masterhub-service.rs`): `PreparedKeyArtwork::prepare()` runs before HID access. The device is opened once and wrapped in `SharedTransport`. `InputReader` and `HidDisplaySink::new` get clones. A failed sink construction is a setup error before software mode (fail closed). Tests never construct this path.
   2. Right after `crate::mode::set(…, true)` succeeds, the first page is requested (`trigger: "software_mode"`).
   3. After each report, once its work was offered to the worker or coalesced, a page is requested if any of its events is a keypad `Navigation`. The display thread keeps one pending request, and newer pages replace older ones.
   4. Each `ApplyReport` and the plan's `missing()` are logged as a bounded `display_sync` record. `service_end` adds `display_sync_count` and `display_failure_count`, plus coalesced-request, dropped-record, and `display_stopped` fields.
   5. A `DisplayError`, failed write, or stalled sink is logged and ignored. It never returns an error from the loop, dispatches or suppresses an action, or changes the exit status by itself. An in-flight write can delay input reads (see `SharedTransport`). No sync starts after the input loop ends or the interrupt flag is set, and the closed write gate makes an in-flight sync fail its remaining keys at once. Hardware mode is restored with priority and retried once on a busy lock. A short write leaves keys dirty, and the next navigation redraws them.
5. **Artwork provider. Done, offline only.** `PreparedKeyArtwork` (above). Real tile artwork needs an image field in the profile format first, which does not exist.
6. **Dial. Not done.** Map the active profile and dial function (`ProfileSession::current_dial_function`) to `DialImageRequest`, add a `DialImageProvider` with prepared 240×240 images, and call `plan_dial` and `apply_dial` when either changes. `HidDisplaySink` already writes the dial for an M131E.
7. **Reconnect and profile switch. Nothing to wire today.** The live runtime has no reconnect, software-mode re-entry, or in-process profile replacement path. A read error ends the session, and the desktop app restarts the process for wake and for a focused-app profile switch. Each process creates a new `DisplaySync`, which starts dirty and redraws all 15 keys on its first sync. If such a path is ever added, call `invalidate()` there and then request a sync.
8. **Tauri session manager resume. No change needed today.** `apps/open-stream-hub/src-tauri/src/session.rs` stops the service on suspend and restarts it after wake (`Event::Resume`, 2 s settle). A restarted service process creates a new `DisplaySync`, which starts dirty and redraws everything on its first `sync_page`. A resume hook is needed only if the service is ever kept alive across sleep. In that case call `suspend()` on sleep and `resume()` after the settle delay, then perform its steps in order: software mode, firmware query (`dial::read_firmware_info` for the M131E), then `sync_page` and dial planning.
9. **Hardware session. Not done (no hardware available).** Before enabling by default: verify key order and orientation, image acceptance, pacing (start with no delay, then add `with_chunk_pacing` sleeps if images tear), the behaviour after a short write, and page-transition latency. Record the results in the [first hardware session guide](masterhub-first-hardware-session.md).
