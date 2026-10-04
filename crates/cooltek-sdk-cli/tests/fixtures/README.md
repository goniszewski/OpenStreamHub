These JSONL files are **synthetic**, not recordings from MasterHUB hardware.

`m003k-synthetic.jsonl` combines the outer framing described in the
[probe's M0 implementation](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/src/m0.rs)
with the documented
[M003K logical input formats](https://github.com/projectzero7/cooltek-sdk-probe/blob/main/docs/MasterHUB/modules/m003k/input_reports.md).
It includes simultaneous rotation, press transitions, a repeated snapshot,
an invalid snapshot, an unknown channel, and a truncated report. The final two
formats exercise preservation and error handling; they do not document additional
hardware capabilities.

`m131e-synthetic.jsonl` deliberately uses the same example bytes with the IPS dial
identity. Replay must preserve them as unknown, because input decoding is selected
by the session VID/PID and these bytes use the M003K class rather than the dial's
software-derived queue-0 class `44`.

`m131e-software-derived-synthetic.jsonl` exercises the dial fields recovered from
MasterHUB 1.0.0.113, as documented in
[the static analysis](../../../../docs/research/masterhub-1.0.0.113-static-analysis.md#dial-input).
It includes current and legacy rotation selectors, repeated rotations and button
updates, current and legacy release selectors, unsupported values/classes, an
incomplete payload, and release timestamps at 600 ms and just below that host
gesture threshold. Arbitrary byte-1 values and an extra payload byte exercise
preservation of fields whose meaning is unresolved. These minimum field examples
do not establish the complete payload length emitted by hardware. RIGHT/LEFT are
application labels; physical clockwise/counter-clockwise mapping is unverified.

`dial-report2-65-byte.hid` is a synthetic descriptor containing
`85 02 75 08 95 40 91 02`: report ID 2, 64 eight-bit output fields. HIDAPI's
report-ID byte makes each output report 65 bytes. This descriptor tests deriving
packet lengths and does not describe a captured device descriptor.

`dial-240x240-baseline.jpg` is a generated 240×240 baseline JFIF JPEG,
used to reconstruct an encoded image from the offline packet plan. Its SHA-256 is
`cd46c629dc680404454dc55dd564b815b5c225bed40c5e7af9e498bd0ab69600` (1725 bytes).
Packet construction is software-derived; these fixtures do not validate display
acceptance, orientation, pacing, or device acknowledgements.

`m4315-software-derived-synthetic.jsonl`,
`m005s-software-derived-synthetic.jsonl`, and
`m032le-software-derived-synthetic.jsonl` exercise the additional input modules
described in the [protocol evidence](../../../../docs/research/masterhub-additional-modules-1.0.0.113.md).
Their headers use legacy identities; tests also substitute the newer IDs and
misleading module labels to verify routing by VID/PID. They include unknown and
malformed reports, which are deliberately constructed diagnostics.

The key fixture has eight reports, six press/release events (including the
renderer's legacy selector), one unknown class and two malformed reports.
The fader fixture has eight reports and seven position events: all five initial
positions, then index 0 changing to 60 and 100. Duplicate snapshots generate no
events; positions above 100 are invalid, and the probe's contradictory selector
`00` remains unknown. The roller fixture has seven reports and five discrete
direction events, including repeated updates and `ff` idle controls. Neither
roller distance nor physical control placement is asserted. The fader and roller
fixtures each also contain one unknown and two malformed reports.

Synthetic paths, timestamps, identities, and missing descriptors are explicitly
recorded in their session headers. Every fixture has a schema version and a
mandatory end record. A malformed HID report inside a complete capture is a
`parse_error` diagnostic, while corrupt JSONL or a missing end record causes the
replay command to fail.
