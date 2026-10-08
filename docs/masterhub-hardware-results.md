# Hardware results

What has actually been observed on real MasterHUB hardware, and what has not. Everything else in these docs is derived from the Windows software and tested against fakes. Only the results below come from a device.

**Rig and date.** 8 October 2026. Apple Silicon Mac running macOS, with the MasterHUB unit connected directly to a Mac port. The unit's modules appear behind an internal USB hub chain. Five modules were present: M0064 base, M003K knobs, M4315 keys, M032LE rollers and M131E dial. The M005S faders were not connected. All captures used `masterhub-diag record` and `replay` (read-only unless noted). The raw logs stay on the test machine.

| Module | Product ID | Result |
| --- | --- | --- |
| M4315 keys | 510 | Input and display verified; see below and [display sync](masterhub-display-sync.md#hardware-results-m4315-8-october-2026). |
| M003K knobs | 514 | Rotation and press verified in default mode. |
| M0064 base | 502 | Sends a constant heartbeat only. No input. |
| M032LE rollers | 516 | **Silent**: no report in four captures. Unresolved. |
| M131E dial | 518 | Input, a raw 240x240 image upload and the four generated faces verified in software mode; backlight and color writes not tested. |
| M005S faders | n/a | Not connected. |

## M4315 keys

- **Default mode.** A passive 30 s capture recorded 0 reports. The keys report only after software mode (`--software-mode`, which writes `03 3a 01` and restores hardware mode on exit).
- **Software mode.** A 30 s capture recorded 50 reports. All 15 key indices (0 to 14) decode as press and release with the existing decoder. Each report starts `01 34 11 00 02 0f`, followed by one state byte per key.
- **HID output report.** ID 2 is 320 bytes long (from the real descriptor). Offline tests had used a 65-byte stand-in.
- **Display and actions.** Key images, folder navigation and `--execute` actions are in the [display sync results](masterhub-display-sync.md#hardware-results-m4315-8-october-2026).
- **Not established.** The physical position of every key index. Only keys 0 to 2 were seen to agree between input and display.

## M003K knobs

- **Default mode works.** No software mode is needed. A passive capture recorded 32 reports in 40 s, and a separate counter-clockwise capture recorded 23 reports in 30 s. Both ended normally.
- **Physical mapping** (as the user turned them): wire index **2 = top** knob, **0 = left**, **1 = right**.
- **Rotation.** Reports start `01 2a 05 00 00 03`, then one byte per knob at byte 6 plus the wire index: `01` clockwise, `00` counter-clockwise, `ff` idle for the others. This confirms the documented values.
- **Press and release.** Each knob press and release decodes as its own event.
- **Counts.** About 8 clicks per knob and direction gave 7 to 9 reports each. One report per click is plausible but not proven, because the clicks were counted by hand.
- **Re-enumeration.** Disconnecting and reconnecting the module gave it a new HID path (the user reseated it). A capture that was open on the old path failed with `hid_read_timeout: error waiting for more data` and recorded nothing. A service run that outlives a replug therefore needs a fresh path. This matches the session manager's reconnect design but was not run against it.

## M0064 base

- A passive 60 s capture recorded 117 reports, all byte-identical: `01 2b 01 00 01` followed by zeros, one every 512 ms. The payload did not change while both rollers and all three knobs were used, so it is a heartbeat or status report and not an input stream. Its meaning is unknown.
- The base does not relay roller or knob events.
- The unit's lighting (the base and the knobs module) changed colors on its own in hardware mode. The host sent no lighting commands.

## M032LE rollers (unresolved)

- Four passive captures (45 s, 40 s, 40 s and 60 s) recorded 0 reports each, while the user scrolled both rollers up and down (about 5 to 10 clicks each way). Every capture ended normally with no transport error, and the module's path never changed.
- The roller module has the same descriptor shape and report IDs as the working knobs module, and the docs expect no software mode. The expected `00` Up, `01` Down and `ff` idle values were therefore **not observed**.
- The base does not carry roller events (see above).
- Possible causes, none tested: the rollers need an init or mode write; the module is half-seated or faulty; they report only in a mode not found yet. Do not guess at writes to this module. Next step if it is revisited: reseat the module, capture again, then compare against the official software on Windows.

## M131E dial

- **Default mode.** A passive 45 s capture recorded 0 reports while the dial was turned and pressed. Like the keys, the dial reports only in software mode.
- **Software mode.** A 45 s capture with `--software-mode` (the tool writes `03 49 01` and restores with `03 49 00`) recorded 20 reports and ended normally.
- **Entering software mode** stopped the dial's built-in animation, and its screen went black. Nothing draws on it until the host does. After the capture ended, the animation came back, so the restore worked (the user's observation).
- **Rotation.** Reports start `01 44 03 00 03 01`, with a final byte `00` or `01` per direction. In the order tried (clockwise first, then counter-clockwise), the decoder's `right` label matched clockwise and `left` matched counter-clockwise. This settles the physical mapping that the docs listed as unverified.
- **Press and release.** Reports start `01 44 03 00 04 01` with a final byte `01` for press and `00` for release. A short press and a long press gave two ordinary press and release pairs. The dial itself reports no hold duration. Long-press detection is the service's 600 ms timing (`DialGestureTracker`) and was not exercised here.
- **Display upload (writes).** With the dial in software mode (`dial mode software --path-hex`), `dial jpeg --path-hex` sent a 240x240 baseline JFIF JPEG (6195 bytes) as 20 output reports of 320 bytes (header `02 45`, byte 6 zero, final flag on the last). The report length comes from the real descriptor. The tool exited cleanly and the image was displayed.
- **Orientation and color.** The test image had a red square top-left, a green one top-right, a blue one bottom-left, and a white gamma shape lower right. The user saw exactly that on the dial, so `--rotate 0` is upright for the installed orientation, nothing is mirrored, and the RGB order is correct.
- **Pacing.** Two stress runs alternated between two different images (upright and rotated 90 degrees) and ended on the upright one: 21 uploads back to back in 0.3 s, then 21 with a 50 ms gap in 1.9 s. No upload returned an error, and the dial was clean and upright after each run. The firmware needed no acknowledgement or delay in this test.
- **Mode switching.** `dial mode software` and `dial mode hardware` were run by hand. The tool does not restore hardware mode on its own (unlike `record`), so the last step has to be run explicitly. After it, the built-in animation came back.
- **Generated faces.** `masterhub-service run --software-mode --display-sync --dial-content function|clock|analog-clock|cpu` ran for 25 s per face with the example `dial-functions.json` profile and no `--execute`. Each run ended cleanly with no display failure. The user saw `volume:` for the function face, a digital time, an analog clock, and a CPU percentage that kept updating, and judged the text legible and upright. The CPU face wrote about once a second (20 writes in 25 s, `cpu:5` to `cpu:8`), matching the 1 s limit. The clock faces wrote once in 25 s because the minute did not change, so a minute rollover was not observed. The animation came back at the end of each run.
- **Function cycling.** A 45 s `--dial-content function` run with the example `dial-functions.json` profile (Volume, Scroll, Apps), without `--execute`. Three short clicks logged `short_press` and a redraw each, `function:1`, `function:2`, then `function:0` (wrapped), all complete with 0 display failures. The user confirmed the labels changed correctly on the dial. Four dial turns each way were routed to the current function (planned volume +5 and -5, not executed), and a press held for about 4.4 s logged `long_press` with the planned mute toggle. The service emits `long_press` when the button is released, so the 600 ms threshold was not timed separately here.
- **Not tested.** Flicker during the uploads (only the final state was inspected); rotations other than 0; whether `volume:` was the whole function label or was clipped; minute rollover of the clocks; backlight and color writes (so the color channel order for lighting is still unknown); firmware queries; and the dial function cycling in the service.

## Not tested

- The M005S faders (not connected).
- Lighting writes of any kind.
- The desktop app against a real device.
- `shortcut`, `launch_app` and every `obs_*` action.
- Reconnect, sleep and wake behavior.
