# MasterHUB protocol evidence

The table lists implemented capabilities, offline test coverage, and remaining hardware validation. Protocol details derived from the Windows software have not yet been verified on a physical device.

| Capability | Reference or implementation | Offline evidence | Hardware status |
| --- | --- | --- | --- |
| Neutral HID inventory and exact-path open | SDK `DeviceRegistry`; HIDAPI enumeration | Identity matching and stale-selection tests; source review of no-initialization path | Pending |
| Module PID generations | `ModuleKind`, `Module::accepts_product_id`, typed handle selection; base `01f6`/`9999`, knobs `0202`/`9996`, dial `0206`/`9994` with VID `2516` | Version-pinned source evidence and tests for aliases, wrong vendor, unsupported IDs, and custom module overrides | Pending |
| Serial feature response | `CanGetSerialNumber`, report `30` for M0 models and `4a` for M131E | Header/ID/length/padding tests and fake transport; actual received length used | Pending |
| M131E software/hardware mode | `dial::software_mode_report`, explicit transport/typed-handle methods; `03 49 <bool>`, 32 bytes total | Exact construction and fake writer tests; explicit lifecycle transitions | Pending |
| M131E backlight and solid color | `dial` builders/setters; `03 44 <0..100>` and `03 46 <three channel bytes>` | Bounds validation before writes, zero-padding policy, fake transport failures | Color channel order and visible effect pending |
| M131E feature queries | Backlight ID `4c`; firmware selectors `47`, `48`, `49`; bounded reply parsers | Actual returned count, ID and required offsets checked; reply byte 1 remains opaque; firmware bytes preserved without inferred meanings | Pending |
| M0 raw input frame | Probe `src/m0.rs`: ID `01`, tag `2a`, little-endian length, payload at offset 4 | Pure framing tests including truncated headers/payloads and outer padding | Pending |
| M003K rotation | Probe channel-0 logical payload `00 03` plus three direction bytes | Synthetic reports; `00` counterclockwise, `01` clockwise, `ff` unchanged | Pending |
| M003K press/release | Probe logical payload `03 03` plus three states | Synthetic state transitions, simultaneous controls, malformed/unknown state preservation | Pending |
| Other input channels/report IDs | Documented but not mapped to knob actions; channel 3 has a different layout | Raw unknown retention | Pending |
| M131E input | `parse_m131e_report`, `parse_m131e_payload`, `M131EDecoder`; queue-0 raw `01 44 u16LE length` | Static SDK/addon/source trace; bounds, application RIGHT/LEFT labels, legacy selectors, duplicate press filtering, repeated rotations, unknown retention and state preservation tested | Requires actual payload/direction capture |
| M131E host gestures | `DialGestureTracker`, 600 ms long-press threshold from app policy | Fake monotonic timestamps, exact threshold, duplicate presses, unmatched release, backward time and reset | Physical sequences pending; gesture is not a firmware event |
| M003K/base lighting | Probe notes; not implemented | None | Pending |
| HID output report sizing | Pure `descriptor::output_report_length`; selected-interface query through `ReportTransport` | Multiple report IDs, constant padding, split fields, push/pop, malformed descriptors and bounded lengths tested; no guessed fallback | Actual descriptors pending |
| M131E display | `JpegPackets`, explicit `display_jpeg`; 240×240 baseline JFIF JPEG; `02 45`, sequence/length, final flag, payload offset 8 | Static source trace; real encoder-generated image fixture; packet reassembly, bounds, zero-fill policy and short-write stop tested; typed upload uses descriptor-derived size | Requires byte 6, pacing, orientation and acceptance evidence |
| Explicit dial lifecycle | `DialSession`: activate/suspend/resume/advance/shutdown; caller supplies notifications and time | Fake transport mode writes, idempotence, two-second resume policy, failures and decoder reset | Requires physical acceptance; no automatic macOS observer or reconnect |
| Dial diagnostic commands | Identity-routed replay, default feature previews, offline JPEG plans; live writes require an exact path | Synthetic aliases/events/gestures; complete feature bytes; 31-packet JPEG reconstruction; no-overwrite/preflight; failed enable/footer cleanup tests | Live queries, writes and cleanup require physical acceptance |
| Mac shortcuts/volume/profiles | `cooltek-sdk-service` and `masterhub-service`; strict profiles, serial precedence, output volume/mute, shortcuts, app launch; independently normalized input with bounded work/logging | Synthetic action plans, blocked-action/rapid-burst tests, gesture timing, explicit overflow counts, pipe deadlines, mock subprocess cancellation, finite drain and one-shot cleanup | Physical input, actual macOS effects/permissions, sustained throughput and HID buffering pending |

Source references:

- [M0 framing implementation](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/src/m0.rs)
- [M003K logical input reports](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/m003k/input_reports.md)
- [M003K feature reports](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/m003k/feature_reports.md)
- [Synthetic capture provenance](../crates/cooltek-sdk-cli/tests/fixtures/README.md)
- [Official software static-analysis evidence and hashes](research/masterhub-1.0.0.113-static-analysis.md)
- [Implementation status](masterhub-implementation-status.md)
- [Binding and action service](masterhub-service.md)
- [Roadmap](masterhub-macos-plan.md)
