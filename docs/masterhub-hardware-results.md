# Hardware results

What has actually been observed on real MasterHUB hardware, and what has not. Everything else in these docs is derived from the Windows software and tested against fakes. Only the results below come from a device.

**Rig and date.** 8 October 2026. Apple Silicon Mac running macOS, with the MasterHUB unit connected directly to a Mac port. The unit's modules appear behind an internal USB hub chain. Five modules were present: M0064 base, M003K knobs, M4315 keys, M032LE rollers and M131E dial. The M005S faders were not connected. All captures used `masterhub-diag record` and `replay` (read-only unless noted). The raw logs stay on the test machine.

| Module | Product ID | Result |
| --- | --- | --- |
| M4315 keys | 510 | Input and display verified; see below and [display sync](masterhub-display-sync.md#hardware-results-m4315-8-october-2026). |
| M003K knobs | 514 | Rotation and press verified in default mode. |
| M0064 base | 502 | Sends a constant heartbeat only. No input. |
| M032LE rollers | 516 | **Silent**: no report in four captures. Unresolved. |
| M131E dial | 518 | Input, a raw 240x240 image upload, the four generated faces, backlight and color writes verified in software mode. App-controlled dry-run display, live preview and normal Stop verified below. |
| M005S faders | 512 | Input verified on all five faders in default mode (11 October 2026): positions arrive as 0 (bottom) to 100 (top). Lighting verified; see the LED sections. |

## USB topology and reported physical layout

The following maps the read-only macOS `ioreg` location IDs to the user's reported current arrangement (9 October 2026). This is a mapping for this connected base and orientation, not a general layout discovery implementation.

| Module | `locationID` path | Reported base position | Product footprint |
| --- | --- | --- | --- |
| M4315 15-key | `1.1.2.2` | Top-left | 4x3 |
| M4315 15-key, second arrangement (10 October 2026) | `1.1.2.3` | Bottom-left, with the rollers above it | 4x3 |
| M4315 15-key, third arrangement (11 October 2026) | `1.1.1.1.1` | Top-left, its top row 1 cell above the base's top edge | 4x3 |
| M005S 5-fader | `1.1.2.3` | Bottom-left, below the keypad | 4x2 |
| M032LE 2x roller | `1.1.1.1.4` | Bottom-left, below the keypad | 4x1 |
| M131E dial | `1.1.1.4.3` | Top-right | 2x2 |
| M003K knobs | `1.1.1.3.4` | Bottom-right, below the dial | 2x2 |
| M0064 base | `1.1.1.4.4` | Base; USB-C ports are on the left and top edges in landscape orientation | 4x6 |

Interpreting the product-name dimensions as module footprints, they total 24 cells (12 + 4 + 4 + 4), matching the 4x6 base. With the base in landscape and its USB-C port edges used as the top/left reference, the reported arrangement tiles the base: keypad and rollers occupy the left 4x4 area; dial and knobs occupy the right 2x4 area. The user reports no free module space. The physical positions come from the user's description; the paths come from the earlier read-only `ioreg` inventory. In the third arrangement the keypad (4x3) and the faders (4x2) are 5 rows tall on a 4-row base, so a module may overhang the base; the app places each module by its top-left cell and the rig preview draws the part outside the base (the rollers were not on the bus, so they were not placed). No slot information has been read from a base or module HID feature report, and a second arrangement has not been tested.

## M4315 keys

- **Default mode.** A passive 30 s capture recorded 0 reports. The keys report only after software mode (`--software-mode`, which writes `03 3a 01` and restores hardware mode on exit).
- **Software mode.** A 30 s capture recorded 50 reports. All 15 key indices (0 to 14) decode as press and release with the existing decoder. Each report starts `01 34 11 00 02 0f`, followed by one state byte per key.
- **HID output report.** ID 2 is 320 bytes long (from the real descriptor). Offline tests had used a 65-byte stand-in.
- **Display and actions.** Key images, folder navigation and `--execute` actions are in the [display sync results](masterhub-display-sync.md#hardware-results-m4315-8-october-2026).
- **Not established.** The physical position of every key index. Only keys 0 to 2 were seen to agree between input and display.
- **Key 1 and per-key artwork (8 October 2026, dev app, Module control App, dry run).** An image prepared in the key artwork editor and assigned to root slot 0 (UI Key 1, wire index 0) with a toggle-mute action was drawn on the physical **top-left** key. Pressing that key logged `control 1 pressed: planned toggle mute` (the Run panel numbers controls from 1, so this is wire index 0). So wire index 0 is the top-left key, and input and display agree. Stop restored the keypad's built-in animation (user-observed). A flat profile with no keypad slots drew blank keys, as designed. In a second run the user pressed the top-left and then the bottom-right key; they logged `control 1` and `control 15` (wire indices 0 and 14), consistent with row-major order but not proof of it, since a column-major order gives the same corners. A third set of presses (second key of the top row, first key of the second row, top-right corner) logged controls 2, 6 and 5 (wire indices 1, 5 and 4), exactly the row-major prediction and not the column-major one (4, 2, 13). So the keys are arranged 5 columns by 3 rows with wire index = row × 5 + column, verified on wire indices 0, 1, 4, 5 and 14; the remaining ten are inferred from that pattern, not pressed. Not timed.

## M003K knobs

- **Default mode works.** No software mode is needed. A passive capture recorded 32 reports in 40 s, and a separate counter-clockwise capture recorded 23 reports in 30 s. Both ended normally.
- **Physical mapping** (as the user turned them): wire index **2 = top** knob, **0 = bottom-left**, **1 = bottom-right**. Only this `single-top` mounting was observed on hardware; it is the app preview's default, and the preview's other three orientations (single knob right, bottom or left) are derived by rotation and not observed.
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
- **Feature report probe (read-only).** `masterhub-diag features --report-id N` (GET FEATURE only, declared IDs only, `0x03` skipped) read every other declared ID on both modules, one at a time, with nothing else holding them. Both answered every request with 32 bytes. Rollers declare IDs 0x03, 0x2d to 0x31 and 0x36; knobs add 0x32 and 0x34. `0x2d`: `01.00.00.00` on both; `0x2e` and `0x2f`: rollers `01.00.03.01`, knobs `01.00.05.00`; `0x30` is the module's serial text (it matches the serial `list` shows); `0x31` is `14` on both; `0x36` is a 14-byte value on both (different values). The knobs' `0x32` is `64 64 64` and `0x34` is `09 09 09`; their meaning is unknown. So the roller module is alive and answers the host; it just sends no input reports in default mode.
- **Owner Windows test (8 October 2026).** The user reported that the same 2× scroller works in the official Windows MasterHUB app. This confirms input hardware functionality through the owner's test; this run was not observed. The four macOS captures above remain at 0 reports, and the macOS input path is unresolved. Whether an initialization or mode command, or another host difference, explains the result is not established.
- **Next evidence.** Capture the official Windows app's startup USB traffic with USBPcap/Wireshark to identify its initialization and input path. Do not guess at writes to this module.
- **All-interface macOS capture (8 October 2026).** The new passive `masterhub-diag record-all` command opened the roller, keypad, base, knobs and dial interfaces concurrently for 60 seconds while the user moved both rollers in repeated up/down passes, including a second pass about three seconds later. All five captures ended normally with valid descriptors and `report_count: 0`; no roller signal appeared on another module or the base. This rules out a simple relay through the other currently enumerated interfaces during this capture, but does not reveal the Windows app's host-to-device initialization writes.

- **IOHIDManager control (8 October 2026, read-only).** A small Swift reader (`IOHIDManagerOpen` with no seize, input-report callback, no writes) ran for 20 s on each module. On the knobs (VID 2516, PID 0202 / 514) while one knob was turned it received 23 input reports `01 2a 05 00 00 03 …`, so the reader works. On the 2x roller (PID 0204 / 516) it received **0 reports** in two runs, the second with the user scrolling both rollers the whole time. The same tool and open mode that work for the knobs deliver nothing for the rollers, so the silence is below our hidapi code. Windows shows the same module sending 152 reports `01 2a 04 00 01 02 …` on endpoint 0x81 while it moves, and sends no output or control request to it. The roller's macOS device properties (report elements, poll interval 2000, system event service attached) match the working knobs. Our decoder already matches the Windows frame shape. The cause is not established; the knobs and the roller share the same macOS path, so it is specific to the roller's delivery to macOS.

- **Lifecycle: kill -9 during a keypad session (9 October 2026, release bundle, dry run, Module control App).** The release app was started with the Everyday profile on the 15-key module. Its service child was running in software mode. The app was killed with `kill -9`. Both processes were gone within 3 s, and the user saw the keypad return to its built-in animation. This used the service's stdin-close path (the app's pipe closed), not a clean Stop. The replug fallback was not needed and was not tested on the keypad.

- **Lifecycle: Stop and Quit on the keypad (9 October 2026, release bundle, dry run, Module control App, Everyday profile).** Stop in the Run panel ended the service process, and the user saw the built-in animation return. A normal Quit (Cmd+Q) during a second session ended the app and its service within about 3 s, and the built-in animation returned. Both runs were dry runs, so no actions executed.

- **Sleep and wake on the keypad (9 October 2026, release bundle, dry run, Everyday profile, Module control App).** With the session running, the Mac was put to sleep for about 30 s and woken. The keypad backlight went dark during sleep (expected, since the module loses power). The app process survived. The service process from before sleep ended and a new service process started about 40 s after the earlier check, so the app restarted it on wake. After wake the artwork was back and keys responded (user observed). Stop then ended the service and the built-in animation returned. The replug fallback was not needed. Sleep and wake on the dial and rollers were not tested.

- **Keypad unplug and replug in software mode (9 October 2026, release bundle, dry run, Everyday profile, Module control App).** With a session running, the keypad's USB cable was unplugged. The service ended with an error (`hid_read_timeout` and "restoring module hardware mode failed: Device is disconnected", the expected result, since the module was gone). The app retried, then reconnected when the cable was back, and started a new service with a display sync. The user saw the artwork return and keys respond. Stop ended the service and the built-in animation returned. The fallback path worked without tinkering; the new session ran on the reconnected path.

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
- **Color channel order.** With the dial in software mode, `dial color ff0000`, `00ff00` and `0000ff` (report `03 46` plus three channel bytes) showed red, green and blue. The SDK's channel order is R, G, B. This settles the open item.
- **Backlight.** `dial backlight N` (report `03 44` plus one byte, 0 to 100) takes effect in software mode. Alternating 100 and 10 twice, 10 was visibly dimmer both times, a darker gray that was still easy to see. A read-back with `dial info` after setting 10 returned `backlight: 10`, and after the earlier run it returned `100`, so the setting reads back correctly. Values between and below 10 were not tried.
- **Firmware query.** `dial info` returned firmware raw `26a0d80a`, text `01.00.04.00` (in software mode). The first `dial info` of the earlier run, made in hardware mode, failed with `Dial feature reply must contain a header and fit 32 bytes`. That was seen once, so whether the query only works in software mode is unconfirmed. It also means the dial's own backlight level before the test was never read.
- **Restore.** After the lighting runs, `dial mode hardware` brought the animation back, and the user judged the dial's lighting normal.
- **Not tested.** Whether lighting commands work in hardware mode (the doc asks for this); backlight values other than 10, 20 and 100; flicker during the uploads (only the final state was inspected); rotations other than 0; whether `volume:` was the whole function label or was clipped; minute rollover of the clocks; and firmware query behavior in hardware mode.
- **Still pending: the app's Lighting page.** The desktop app now sends the same three writes in the verified order (software mode, backlight `03 44`, colour `03 46` with R, G, B) plus Reset (`03 49 00`), through `set_dial_lighting`/`reset_dial_lighting`. The in-app check — a live apply changing the dial's lighting, Whole kit reaching every connected dial, a run holding the dial refusing the write, and Reset returning the built-in animation — remains for the next hardware session. The command-level facts above were verified through the CLI.

## Desktop app dial dry-run (M131E, 8 October 2026)

Tested at `3689c3a` with the Everyday profile, Dry run, and Settings > Module control set to **App**. Software mode was set automatically, and no `--execute` was used.

- **Input.** Only plans were checked: 16 events in the first snapshot and 139 after Stop, all planned, none executed, none failed. Action execution and long-press timing were not measured.
- **Display.** The user confirmed the physical display and the live preview needle, which stayed inside the screen.
- **Stop.** The panel showed Stopped, no sidecar process remained, and the built-in animation returned (user-confirmed).
- **Not covered.** Function cycling (Everyday has no `dial_functions`), normal Quit, reconnect, sleep and wake.

## Desktop app keypad dry-run (M4315, 8 October 2026)

Tested at `3689c3a` on the M4315 with temporary, unsaved two-level folders in Everyday. Dry run, App mode, no `--execute`.

- **Navigation.** Root, L1, L2, L1, root, twice: 16 events, no planned or executed actions, nine display-sync redraws. Raw completeness was not counted, and latency was not measured.
- **Display.** The user confirmed the keys redrew, the L1 and L2 badges showed, the preview responded, and the built-in animation returned after Stop.
- **Cleanup.** Undo restored the original empty root.
- **Not covered.** Real actions, other Module control settings, Quit, crash recovery, reconnect, sleep and wake, and all 15 key positions beyond the ones pressed.

## Desktop app dial replug (M131E, 8 October 2026)

- **First replug.** After unplugging and reconnecting, the Run panel fell back to another module and showed Stopped while the dial session was running. This selection defect was fixed in `0a4162c`.
- **Repeat after the fix.** The dial stayed selected and showed Running after reconnect. Recovered input was 11 events, all planned, none executed, none failed. The user confirmed the app display and the built-in animation after Stop.
- **Not covered.** The intermediate disconnected UI, sleep and wake, and executing-session reconnect.

## Crash safety: stdin watchdog (M131E dial, 8 October 2026)

`masterhub-service run --exit-on-stdin-eof` (which the app always passes) exits as a signal would when its stdin closes, restoring hardware mode.

- **Standalone.** With the service in software mode on the dial and its stdin closed, it exited 49 ms later with reason `stdin_closed` and no error.
- **App crash.** With a dry-run, software-mode session running, the app was killed with `kill -9`. The sidecar exited about 120 ms later, and the dial's animation returned within about a second (user-reported).
- **Not covered.** The same crash on the M4315, a crash during session start, an OBS password set, and sleep and wake.

## OBS editor listings against the fake obs-websocket server (9 October 2026)

The editor's OBS pickers were checked against the scripted fake server in `crates/cooltek-sdk-service/src/obs.rs`. No real OBS was involved.

- **Catalog.** Four requests in order over one connection. Scene collections and profiles are bare string arrays, matching the obs-websocket v5 spec. A reply missing a list is an error, not a partial list.
- **Bounds.** Names over the profile's 128-byte limit are dropped. More than 1000 entries are refused.
- **Off.** While OBS is off in Settings, listing commands refuse without connecting.
- **Still pending.** A check against a real OBS 28+ install, including the password-required path and refresh after a scene rename, and `obs_set_input_volume` / `obs_input_volume_from_position` (fader moving a real input's volume; fader range and jitter).

## Run mode and applied saves: press checks (pending, 10 October 2026)

Both checks need a module running a profile with actions, and are for the next hardware session.

- **Run mode.** With the header mode Live, a press runs its action on the Mac; switched to Test, the same press is only logged ("Would run: …") and nothing happens. The badge matches what a press does in both states.
- **Applied save.** With a run active, edit a binding of the running profile and save: the run swaps in place (same session, no stop or restart by the user), the display redraws from the saved profile, and a press runs the new action in Live and only logs it in Test. A save that adds a new kind of action in Live pauses the run once, and the resume dialog lists the additions.
- **Watch.** An applied save restarts the run's process, so the module's display redraws briefly. The automatic save fires about a second after an edit settles; if a running profile is edited in bursts, check the redraw is not disruptive.

## Automatic key artwork: desktop check (pending, 10 October 2026)

Needs the desktop app, not a module. The drawing logic is covered by unit tests with a fake session and a fake icon source; the editor's own wiring (the settle delay, the effect, Remove artwork) is not.

- **Open application.** On a key with no artwork, assign Open application and set the bundle id to an installed app (for example `com.apple.Safari`): within about a second the key shows that app's icon, and the Icon and label studio reopens with it. Undo removes it; redo brings it back.
- **Following the app.** Change the bundle id to another installed app: the drawing follows. Change the label text in the studio first, or upload an image: the key is left alone.
- **Declined.** Remove artwork, then keep the same app: nothing is drawn again until the app changes.
- **Open URL.** Assign Open URL with an https address: a globe with the host appears. An address that is not http(s) draws nothing.
- **Selecting.** Selecting a key that already has an Open application action and no artwork draws nothing.

## Not tested

- The dial's `image` face and the per-profile `dial_display` setting in the app.
- All lighting writes beyond the dial backlight and color, except the base, tested below.
- Most of the desktop app against real hardware, and every `obs_*` action against a real OBS.
- Sleep and wake, executing-session reconnect, and reconnect on other modules.

## Knob, fader and base LEDs (11 October 2026)

Unit with the M0064 base (firmware `02.00.04.00`), M003K knobs (`01.00.05.00`), M005S faders, M032LE, M131E and an M4315 that was not in the inventory. The app and its services were closed. Everything was sent with `masterhub-diag led`, no `--save` unless stated, with the user watching the hardware.

- **Base: works.** `led mode 1 --module base --index 0` then `led color ff0000 --module base --index 0` over the base's own path turned the edge strip solid red (it had been cycling colours). `ff0000` shows red, which fits R first; green and blue were not tried. The effect stayed after the program exited. Neither command changed the base's `0x34` mode (9) or `0x32` brightness (100) readbacks, so those report stored state, not what is lit.
- **Knobs: no effect.** The `led sweep` (red, each index 0 to 4, 2 s each) wrote without errors and lit nothing; the knobs kept fading through colours. The same held for Static (mode 1) plus a colour on all LEDs and on each index 0 to 4; Disable (mode 0) on all LEDs and each index; the same writes over the base's interface (`--via-base` experiment, not kept); and one saved write (`led mode 0 --save`), which was then undone with `led mode 9 --save`. The module's own `0x34` readback stayed `03 09 09 09` throughout, one entry per knob, three entries, not five.
- **Faders: no effect.** `led sweep --module slider` (indexes 0 to 2) wrote without errors, lit nothing, and the faders kept cycling. Disable over the base's interface also changed nothing. Not tried further: colour with Static mode, per-index writes beyond the sweep.
- **SET FEATURE:** a feature `0x34` write (`34 03 00 09 09`) to the knob module failed with an I/O timeout from `IOHIDDeviceSetReport`; the readback did not change. `0x32` was not tried.
- **Static analysis, wrong on two bytes (see the next entry).** The official app's main process connects with `Connect_Device_CM_M0` (reads feature `0x31` and the firmware, writes nothing) and then calls `Adjust_LED_*_CM_M0` once per index with type 1 for knobs and 2 for faders; that reading of the types was swapped, which the Windows capture below corrected. See the [Windows capture instructions and results](masterhub-windows-led-capture.md).
- **Cause found (Windows capture, 11 October 2026).** The official app on Windows sends the same three commands (`2c` mode, `2d` colour, `2e` brightness) as output report 2 on the interrupt OUT endpoint, but with module type **2 for knobs (LED indexes 0 to 2)** and **type 1 for faders (indexes 0 to 4)**; the base is type 0, index 0. The static analysis and the SDK had these two swapped, so every Mac write to the knobs went to a type the module ignores. Per LED the app sends one report; it never sends index `0xff`. A colour change is `02 2d <type> <index> R G B 00`; mode is `02 2c <type> <index> <mode> 00`; brightness is `02 2e <type> <index> <0-100>`. Bytes after these (a leftover version string such as `.00.05.00`) are buffer residue, not part of the command. Static is mode 1, Off 0, Breathing 2, Strobe 3, Cycling 5, Random 6, Wave 7, Water 8, Rainbow 9; mode 4 never appears. Changing a colour in Static makes the app resend colour, mode 1 and brightness for the module. The app sends no setup report before lighting; on startup the captures show only periodic feature report polling. The SDK now uses the corrected types; **pending:** run `led color ff0000 --module knob --index 0` and `--module slider` on the unit to confirm.
- **Knob effects verified on hardware (11 October 2026, after the type fix).** With the corrected type 2, knob LED 0 on the M003K (`01.00.05.00`) accepted colour and mode writes and showed every effect, sent one mode at a time with `led mode N --module knob --index 0`: 0 off, 1 static (solid green after a colour write), 2 breathing, 3 a slow strobe, **4 a fast strobe, about 3 to 4 times faster than 3** (a mode the Windows app never selected in the capture), 5 cycling without transitions, 6 random (shuffles through effects), 7 wave, 8 water (light rotating around the knob, as in the official app), 9 rainbow. So the type bytes were the cause. Mode 4 has no name in the app's list; whether the app's strobe speed setting picks between 3 and 4 is untested. Only knob index 0 was swept; indexes 1 and 2 got Static and green only, and the faders and the `all` index (`0xff`) are still untested.
- **Fader effects verified on hardware (11 October 2026).** With type 1, Static plus green on fader indexes 0 to 4 turned all five LEDs solid green. Then fader LED 0 on the M005S was stepped through modes 2 to 9 and 0 (the other four stayed green); each looked as expected and matched the knob behaviour (2 breathing, 3 slow strobe, 4 fast strobe, 5 cycling, 6 random, 7 wave, 8 water, 9 rainbow, 0 off), then Static green was restored. Not tried: fader indexes 1 to 4 with effects, brightness, other colours, the `all` index (`0xff`).
- **Brightness verified on hardware (11 October 2026).** `led brightness N --module knob|slider --index I` is per LED: with all LEDs at 100, setting knob LED 0 to 50 was not visibly different, but 5 was clearly dimmer than the other two, and the same held for fader LED 0 (the others stayed at 100). Both were restored to 100. Only index 0 was dimmed on each module; a value between 5 and 50 and the `all` index were not tried.
- **Not tried:** colour channel order beyond red, base brightness and the other effect numbers, `--save` on the base.

## OBS connection states (pending)

With a real OBS 28+: check that Settings › OBS Studio shows Connected; Not running with OBS closed; WebSocket off with OBS open and its WebSocket server disabled; Wrong password and Password needed with authentication on; Wrong port with another server on the port. The first two depend on `pgrep -xi obs` matching the OBS process name.

## Lighting page: knobs and faders (pending)

With the desktop app and a kit with knobs and faders: pick the 3-knob module, move the effect, brightness and colour controls and check each knob follows (LED set to All LEDs, then LED 1, 2 and 3 one at a time, the others unchanged); the same on the 5-fader module with LEDs 1 to 5; Whole kit with a knob module picked reaches the faders as well; Strobe, fast is faster than Strobe, slow. A running profile on the module refuses the write with its message. Turn off switches the picked LED, or every LED, to Off; Match modules on the base copies the colour of the first lit knob or fader LED onto the edge strip; and the workspace's module art shows the applied colours (knob rings, fader glow strips). Unplug a module and plug it back in: the app sends the stored look again (the dial keeps its backlight and colour, knob, fader and base keep their per-LED look); a module a running profile drives is left alone until it appears again.

## Macro recorder and key chooser (pending)

On a Mac, with the desktop app: open a key's binding and pick Recorded macro. Without Input Monitoring, Record macro shows the row as Not allowed and Open System Settings opens the right pane; after allowing it, Check again enables Start recording. Record a chord with a pause (for example Command+C, wait 1 s, Command+V) in a text editor: Stop shows the folded keystrokes with the waits and holds, Use recording fills the timeline, and the Planned line reads the same sequence. Press the key on the module in Live mode with Accessibility allowed: the keys replay with the recorded timing; without Accessibility the run reports that macOS did not allow posting events and nothing is sent. Also check the key chooser sends F13 to F20, a keypad key and a media key (Play/Pause) from a shortcut binding, and that the activity log shows only the macro's step count and duration, never its keys.

## Base layout as the editing canvas (pending)

With a real kit: the Workspace shows only the Connected base layout, each module where it sits on the base. Click each knob cap, fader cap, key, the dial and each roller half on its module and check the inspector names the control you clicked (Knob 1 to 3 in the order the hardware numbers them, Fader 1 to 5, Key 1 to 15). Drag a fader cap and check the simulated value follows. Turn a knob and press a key and check the live pointer and pressed highlight appear on that module in the layout. With two modules of one kind connected, check clicking one edits it and the other stays unselected. Unplug a module and check its module in the drawer reads "Not connected", that picking it replaces the layout with the module on its own, and that Go back returns to the layout.

## Focused-app rules from the workspace (pending)

On a Mac, with the desktop app and a saved profile open: in the profile manager, open the picker under "Switch automatically when an app is in front" and check it lists the apps with a Dock icon that are running (not OpenStreamHub), sorted by name, and refreshes when you open it after launching another app. Add Safari, turn automatic switching on, and check the header icon appears with the rule count; bring Safari to the front and check the icon's tooltip and the profile name beside it read the matched profile, and that a dry run switches to it (an executing run waits for confirmation). Bring an app with no rule to the front and check the tooltip says it has no rule. Remove the app with its chip and check the rule disappears from the focused-app rules panel too.

## App preset packs (pending)

On a Mac, for each pack in the chooser's App presets (DaVinci Resolve, Final Cut Pro, Logic Pro, Audacity, Zoom, Microsoft Teams, Music and Spotify): create the profile from the preset, check the profile manager flags it as imported (dry run only), allow it to execute, run it in Live mode with the application in front, and press each control: the shortcuts should do what the key label says in that application (Resolve and Final Cut Pro: blade, marks, tools and J, K, L; Logic: record, cycle, metronome, windows; Audacity: record, pause, split, silence, trim and zoom; Zoom and Teams: mute, camera, share, raise hand and hang up; Spotify: play, skip and volume). Note every shortcut that differs in the installed version, correct the pack file and set `verified` to true for that pack. Check that the notice's "Switch to it when the app is in front" creates the focused-app rule and turns automatic switching on.

## Microphone volume fader (12 October 2026)

`masterhub-service run --execute` with a one-binding profile (fader 1, "Microphone volume follows position"), two 60-second runs on the 5-fader module, with the Mac's default input a USB webcam microphone (Microsoft LifeCam VX-2000). `osascript` reports no readable input volume for that device (`missing value`), yet setting it worked: the Input level slider in System Settings followed the fader (user-observed).

- **Delay found.** In the first run every fader report ran its own `osascript` call (about 0.18 s each), one after another: 64 reports gave 64 calls, and the slider lagged behind the hand. The worker only merged positions when the queue was full, and the microphone action had no merge rule.
- **Fix.** The worker now folds queued absolute positions into the latest one, and microphone volume has a merge rule. The second run showed 347 fader reports served by 72 action records, and the user found the slider "somewhat smooth".
- **Jitter at rest (12 October 2026).** `masterhub-diag record` on the M005S, 60 s, default mode. The first attempt recorded 0 reports because the faders were not moved. In the second the user moved the faders for about 11 s (205 reports, from 17.5 s to 28.6 s) and then left them alone: the module sent nothing for the final 31.4 s, so a resting fader produces no reports and no jitter. During movement no two consecutive reports were identical, the longest pause between reports was 1.7 s, and there was one one-point back-and-forth step (fader 5) among 204 changes. Ranges seen: fader 1 32 to 54, fader 2 58 to 100, fader 3 56 to 100, fader 4 0 to 100, fader 5 46 to 100. Only fader 4 was swept end to end there; the sweep below covers the rest.
- **End-to-end sweep (12 October 2026).** `masterhub-diag record` on the M005S, 40 s, the user moved each fader down and up through its whole travel. 325 reports; every fader reached 0 at the bottom and 100 at the top, so the 0 to 100 scale and the end stops hold for all five.
- **Inspector readout (12 October 2026).** With a fader selected in the inspector, the user moved it on the real module: the "Fader now" value and its range followed the movement, as expected.
- **Not checked:** a microphone with a readable software gain, and the range readout in the inspector.

## Key press feedback (12 October 2026)

`masterhub-service run --profile examples/profiles/keypad-folders.json --software-mode --display-sync --press-feedback --duration-secs 60` on one M4315, without `--execute` (no action ran). The user pressed keys and folder keys: the held key dimmed, the keys stayed responsive. The log agrees: 12 `press` and 4 `navigation` redraws, all complete with no error; a press wrote only the held key (for example `[0]`, `[1]`, `[2]`), and opening a folder drew the new page with that key already dimmed (`[0,1,2]`, `[0,1]`). Session: 16 reports, all processed, 0 dropped or coalesced, 0 display failures. No per-write timing is logged, so the 150 ms cutoff was not measured; feedback never switched itself off during the run. The service refuses to write its output to a plain file (pipe it through `tee`).

**Feedback off, via the service (12 October 2026):** the same command without `--press-feedback`. The user pressed keys and folder keys and nothing dimmed. The log agrees: no `press` display records, only the initial `software_mode` redraw and 3 `navigation` redraws; 15 reports, all processed, 0 dropped, 0 display failures.

**Animated key, via the service (12 October 2026):** a temporary copy of the example profile with a two-frame animated key (400 ms frames, looping) on key 0 of the root page, run with `--press-feedback`. Three holds of about 4 s each on that key wrote `[0]` on every press and every release (6 writes), and the run's `display_sync_count` of 123 shows the animation kept redrawing alongside them. Presses on blank keys wrote nothing (`[]`), as designed. 32 reports, all processed, 0 dropped, 0 display failures. The log cannot show whether the animation visibly kept playing while dimmed; no problem was reported.

**Still pending:** the setting off in the desktop app; the action latency compared with the setting off (run without `--execute`); and the check through the desktop app rather than the service.

The original checklist, for the remaining items:

On the 15-key module with the desktop app (Module control App, "Dim a key while it is pressed" on): press and hold an action key and check it dims within a blink and returns to normal on release, while the action itself still runs without any delay compared with the setting off; hold a folder key and check the folder opens with that key dimmed until release; check an animated key keeps animating while dimmed. Turn the setting off, start the profile again, and check nothing dims. Watch the run log for `press` display records and see whether a press write ever exceeds 150 ms (feedback then switches itself off for the run). Applies at the next profile start.

## Fader input (11 October 2026)

`masterhub-diag record` on the M005S (product ID 512), default mode, nothing sent, 60 seconds, the faders moved by hand. The module sends input reports in default mode, unlike the roller module. A first capture (45 reports, the faders only partly moved) showed the frame: each report is `01 2a 07 00 02 05` followed by five position bytes, and the first report carries all five positions. `replay` decodes only the faders that changed after that.

- **Full-range run:** all five faders were set to the middle (start values 54 to 60), then each in turn, 1 to 5, was pushed fully down and then fully up. 395 reports. Each fader reached 0 first and then 100, in that order, so 0 is the bottom and 100 the top. All five ended at 100.
- **Not done:** the move was by hand and not timed; jitter and a fader's resolution at rest are not measured, and the microphone-volume behaviour is the separate pending check.
- **Not asked of the module:** there is no read-on-demand for positions; a position becomes known with the first report after a fader moves, which carries all five.

## Custom dial picture (11 October 2026)

`masterhub-service run --software-mode --display-sync --dial-content image` on the M131E with a profile whose `dial_display` held a prepared 240×240 JPEG (7 KB, `dial prepare-image` at quality 85): a test card of four coloured quadrants, a white cross and a black tab in the top-left corner. The app and its helpers were closed first. Two runs of 25 and 45 seconds: the service reported one display sync with the face `image`, written, no failures, and ended on schedule. The user saw the card on the dial, upright; the black tab sat just outside the round border, as expected for a corner of a square picture on a round screen. After each run the dial returned to its built-in animation.

- **Through the app (11 October 2026):** the same test card, chosen on the dial's Display tab ("Custom image") and drawn by a normal app run, showed on the dial (user-observed).
- **Fit, rotation and background (12 October 2026):** through the app's dial Display tab with a 480×240 test picture (red square top-left, green top-right, blue bottom-left, a yellow F in the middle). Seven settings were tried in turn: contain, cover and stretch at 0°; contain at 90°, 180° and 270°; contain with a changed background colour. The user confirmed that each looked as expected on the dial. The direction each rotation turns was not written down separately. The service log showed no write errors.
- **Reopen (12 October 2026):** after quitting and restarting the app, the same profile's dial Display tab still showed the fit, rotation and background controls, so the source picture was kept in the profile. Changing a setting updated the dial (user-observed).
