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
| M005S faders | 512 | Input verified on all five faders in default mode (11 October 2026): positions arrive as 0 (bottom) to 100 (top). Lighting verified; see the LED sections. Fader 1 drove a real OBS input's volume on 19 October 2026 ([OBS results](#obs-studio-with-a-real-obs-19-october-2026)). |

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

### Fourth arrangement: placed from the slot table (14 October 2026)

Slot-table placement (derived from the official app's base layout, no per-module rules) checked on the connected base. Modules (rollers not on the bus): 15-key `1.1.1.1.1`, 5-fader `1.1.2.3`, dial `1.1.1.4.3`, knobs `1.1.1.3.4`, base `1.1.1.4.4`, so the base's hub is `1.1` and the slots are `111`, `23`, `143` and `134`. The table placed the 15-key module top-left (hanging above the base, from the observed entry for `111`), the fader module bottom-left, the dial top-right and the knobs bottom-right. The user compared the app's base layout with the real arrangement and reports a 1:1 match; no code change was needed for the new arrangement. Still unconfirmed: the rest of the roughly 80 table entries that have not been seen on this base, and a second base.

### Slot turns and several more arrangements (14 October 2026)

The slot table also gives each connector's turn for a module (the official drawing's rotation). With the fader module on connector `113` (`1.1.1.1.3`) the table puts it at column 4 standing up, 2 columns by 4 rows; drawn flat it would have hung two columns off the right edge, which the user saw as wrong. The app now stands a module's footprint up and turns its art when the connector turns it a quarter, and every one of the 92 table entries then fits on the 6 x 4 base. The official table has no 15-key entry for connectors `113` and `143`; on the 11 connectors both tables share, the 15-key and fader modules line up on one edge (bottom when flat, left when standing up, right when upside down), so the missing entries are inferred that way (`113`: column 4 standing up, hanging one column past the right edge; `143`: column 2, row -1, flat). With the fader and keypad swapped (keypad on `113`, fader on `111`) the user confirmed the layout matches. The user then tried several more arrangements of the dial, knobs, faders and keypad and reports that each one looked right. Not tested: the roller module in these arrangements, the base turned to other orientations with standing-up modules, and which way the physical key and dial images turn.

### Key pictures on a turned 15-key module (14 October 2026)

The 15-key module on a connector that turns it 90 degrees clockwise (`1.1.1.2.2`, slot `122`), its run started with `--display-turn 90` by the app. The user reports the key pictures read upright on the device. The first run after the module was moved started with a stale turn of 0 (the app had kept the turn from the module's previous connector; seen in the service's arguments, the screen was not looked at); the app now keeps only the base rotation and any rotation the user chose, and adds the connector's turn at each start. The 270 degree turn was checked too (module on connector `33`): the first try drew the pictures unturned because the service read the turn as an 8-bit number, which cannot hold 270; with that fixed the pictures read upright. A module turned 180 degrees (its COOLER MASTER text upside down) does not connect to the base at all, so only 0, 90 and 270 are physical positions. Not checked: the dial.

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

## OBS Studio with a real OBS (19 October 2026)

Real OBS Studio with obs-websocket 5.7.4 on this Mac, WebSocket server on port 4455 with a password (OBS asks for authentication). Dev build of the app, 15-key, knob, dial and 5-fader modules connected, the *Streaming (OBS)* profile running in Execute.

- **Connection.** After the password was entered once, Settings › OBS Studio showed **Connected**. The editor's OBS name fields were fed from the real OBS.
- **Password reaches the service only at start.** The first run failed to act because the modules had started 80 seconds before the password was saved: a service reads the endpoint and password once, when it starts. After Stop and Start the modules got `--obs-password-stdin`.
- **Fader to OBS input volume.** Fader 1 of the M005S bound to `obs_input_volume_from_position` for the input *Mic/Aux*. With the first mapping (a linear multiplier, `percent / 100`) the owner found the change uneven against OBS's slider (the first half of the travel did almost nothing). After the mapping followed OBS's own logarithmic slider scale (about -19 dB at 50%), the owner confirmed it works as expected.
- **Faders did not start by themselves** at first: the app auto-started only the keys, dial and knobs. Every control module now starts automatically; confirmed once the fader session ran.
- **Keys on the real OBS (19 October 2026, later the same day).** The owner pressed every key of the *Streaming (OBS)* profile and reported that all worked as described: Record (start and stop), Pause rec (pause and resume), Screenshot (PNG in `~/Pictures`), Virtual camera, Replay buffer, Save replay, Studio mode, Transition, Mic mute, Mic -3 dB and +3 dB, Play / pause and Stop media, and knob 2 (step and mute). No failures were reported; the per-key details (file names, refusal texts) were not recorded.
- **Not checked yet on a real OBS:** `obs_stream` (it goes live, so it was left out on purpose), `obs_scene`, `obs_scene_collection`, `obs_profile`, `obs_filter` and `obs_source_visibility` (not in the test profile), the failure paths (a refused press, a missing input), and the wrong-password, wrong-port and not-running states.

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

## Base orientation byte (12 October 2026)

The base firmware (see [firmware images](research/masterhub-firmware-images.md)) answers feature report 0x35 with one orientation byte. A read-only `masterhub-diag features --report-id 0x35` on the base (PID 502, serial `MHBS01AA…`, with the app running) returned `35 00 01 00 …`: state 1, with the base lying down in landscape with its USB-C ports at the top and left. The desktop app can read this report every two seconds and follow it (the layout's experimental "Follow sensor" switch, off by default). **With the base stood up (12 October 2026, three reads a second apart): still `35 00 01 …`.** The state did not change. The firmware only updates the byte while the sensor's vertical axis reads about 1 g (counts 500 to 1024), so a truly upright base probably leaves it at the last value, and the 0 and 1 states are decided by small tilts of x and y (80 and 50 counts of about 1024, a few degrees) rather than by lying down against standing. What the two values mean physically is therefore **not established**. **Sampled while moving the base (12 October 2026):** about three reads a second for 50 seconds while the base was held in turn lying flat in landscape, lying flat turned a quarter on the desk, standing landscape and standing portrait. All 198 reads were `35 00 01 00`. The byte did not follow any of those positions on this base, so it is not a usable orientation reading; the app's "Follow sensor" is experimental and off by default. Why it stays at 1 is open: the firmware's poll routine returns at once when a sensor status byte is not zero, so the sensor may not have started on this unit, or the routine may not run in this mode. **Pending:** the `01 2b …` input report while the base moves, and the other firmware routines that set the byte.

## Timer countdown on the key (checked 20 October 2026)

`masterhub-service run --execute --software-mode --display-sync --duration-secs 90` on one M4315 with a temporary profile of two timer keys on the root page: key 0 labelled "15s" (15 s, label "15 second test", sound `glass`) and key 1 labelled "60s" (60 s, `none`). The user pressed them in turn and together and reported that all of it worked: the remaining time drawn on the key, the picture returning, and the end of a countdown. The log agrees: 5 `timer_started` (the second countdown running at the same time as the first, `running_count` 2), 4 `timer_cancelled` with the reason `pressed_again`, and 1 `timer_expired` (the 15 s key, left to run out); no `action_failed`, no `timer_alert_warning`. The run ended with `display_sync_count` 68, `display_failure_count` 0 and `display_dropped_record_count` 0. The log keeps only the first display record, so the one-second redraws are known to the user's eye and the counters, not from individual records.

The key picture was an action tile with only a text label, not a picture with an icon. The user's report was one sentence: the exact look of the digits, whether the other keys flickered, and the notification text and sound were not described separately. The end sound and notification ran with `--execute` in a terminal on this Mac, not through the desktop app.

## Key press feedback (12 October 2026)

`masterhub-service run --profile examples/profiles/keypad-folders.json --software-mode --display-sync --press-feedback --duration-secs 60` on one M4315, without `--execute` (no action ran). The user pressed keys and folder keys: the held key dimmed, the keys stayed responsive. The log agrees: 12 `press` and 4 `navigation` redraws, all complete with no error; a press wrote only the held key (for example `[0]`, `[1]`, `[2]`), and opening a folder drew the new page with that key already dimmed (`[0,1,2]`, `[0,1]`). Session: 16 reports, all processed, 0 dropped or coalesced, 0 display failures. No per-write timing is logged, so the 150 ms cutoff was not measured; feedback never switched itself off during the run. The service refuses to write its output to a plain file (pipe it through `tee`).

**Feedback off, via the service (12 October 2026):** the same command without `--press-feedback`. The user pressed keys and folder keys and nothing dimmed. The log agrees: no `press` display records, only the initial `software_mode` redraw and 3 `navigation` redraws; 15 reports, all processed, 0 dropped, 0 display failures.

**Animated key, via the service (12 October 2026):** a temporary copy of the example profile with a two-frame animated key (400 ms frames, looping) on key 0 of the root page, run with `--press-feedback`. Three holds of about 4 s each on that key wrote `[0]` on every press and every release (6 writes), and the run's `display_sync_count` of 123 shows the animation kept redrawing alongside them. Presses on blank keys wrote nothing (`[]`), as designed. 32 reports, all processed, 0 dropped, 0 display failures. The log cannot show whether the animation visibly kept playing while dimmed; no problem was reported.

**Write timing (12 October 2026):** keypad `display_sync` records now carry `elapsed_micros`. On the 15-key module with the service and `--press-feedback`, 35 press redraws took a median of 8 ms (maximum 15 ms), 15 navigation redraws a median of 26 ms (maximum 33 ms) and the first full draw 35 ms, so a press never came near the 150 ms cutoff. The user had reported a noticeable dimming delay and one blink at the start in the desktop app; the write time does not explain it, so the delay is not the key write.

**Desktop app (12 October 2026):** with the setting on, the first app run showed a noticeable dimming delay and one blink at the start; a standalone service run right after it was responsive and did not flicker. After restarting the app with the timing build, the user reported it looked fine, so the delay was not reproduced. The cause of the first run's delay and blink is unknown (the first run followed a fresh `tauri dev` build, and the app can restart a run in place at launch); the Run panel's `elapsed_micros` values were not read. The setting-off half was not recorded separately.

**Still pending:** the dimming delay returning in the desktop app (then read `elapsed_micros` in the Run panel); the action latency compared with the setting off (run without `--execute`); and the check through the desktop app rather than the service.

The original checklist, for the remaining items:

On the 15-key module with the desktop app (Module control App, "Dim a key while it is pressed" on): press and hold an action key and check it dims within a blink and returns to normal on release, while the action itself still runs without any delay compared with the setting off; hold a folder key and check the folder opens with that key dimmed until release; check an animated key keeps animating while dimmed. Turn the setting off, start the profile again, and check nothing dims. Watch the run log for `press` display records and see whether a press write ever exceeds 150 ms (feedback then switches itself off for the run). Applies at the next profile start.

## In-place mode and profile switch (13 October 2026)

Desktop app with three runs executing (the 15-key module with key display sync, the other two in software mode), keys module with a timer key and a second bound key. The user pressed keys by hand; the service output was not read.

- **Mode:** a bound key ran its action in Live, did nothing but report a test result after switching to Test, and ran again after switching back to Live.
- **Profile swap:** after changing that key's action and waiting for the autosave, the key did the new action, and the key images did not flash or redraw.
- **Timer across a switch:** a timer started in Live and switched to Test while counting kept its countdown on the key without a redraw and, on expiry, only logged (no notification, sound or action).
- **Which path ran:** the service output is not logged outside the app, but the three service processes (the keys display run, the dial display run and one run without display sync) kept the same process IDs and the same profile files (unchanged since 08:21) through the whole check, so none of them was restarted. By the code, a profile swap on a display run must restart it, so the edited key was probably on the run without display sync; this was not confirmed.
- **Not checked:** a profile swap on the keys display run itself, and a change to key artwork.

## Dial backlight during a run and screen brightness (13 October 2026)

Desktop app, dial running its display and the 15-key module running its keys.

- **Dial lighting through a run:** the Lighting page's dial write first sent the backlight and the colour. The colour fills the whole screen, so it wiped the dial's picture (the screen went grey). A running dial now takes only the backlight, through the run's control channel (`dial_look`), and the page disables the colour picker and Reset for it. The user then confirmed the backlight changes with the picture kept (user-observed).
- **Screen brightness per profile:** a "Dial backlight" slider on the dial's Display tab and a "Key screen brightness" slider in the keypad inspector (10 to 100 percent, stored in the profile as `screen_brightness`). The dial's value is the backlight report `03 44`, sent right after software mode; the 15 keys have no known brightness command, so their pictures are drawn dimmer. Both worked on the real modules (user-observed); no measurements were taken.

## Fader input (11 October 2026)

`masterhub-diag record` on the M005S (product ID 512), default mode, nothing sent, 60 seconds, the faders moved by hand. The module sends input reports in default mode, unlike the roller module. A first capture (45 reports, the faders only partly moved) showed the frame: each report is `01 2a 07 00 02 05` followed by five position bytes, and the first report carries all five positions. `replay` decodes only the faders that changed after that.

- **Full-range run:** all five faders were set to the middle (start values 54 to 60), then each in turn, 1 to 5, was pushed fully down and then fully up. 395 reports. Each fader reached 0 first and then 100, in that order, so 0 is the bottom and 100 the top. All five ended at 100.
- **Not done:** the move was by hand and not timed; jitter and a fader's resolution at rest are not measured, and the microphone-volume behaviour is the separate pending check.
- **Not asked of the module:** there is no read-on-demand for positions; a position becomes known with the first report after a fader moves, which carries all five.

## Dial GIF playback (measured and seen on the dial, 19 October 2026)

`masterhub-service run --software-mode --display-sync --dial-content image --duration-secs 20` on the M131E, no `--execute`, with a profile of six busy frames (random 4×4-pixel colour blocks) of about 44 KiB each, close to the 48 KiB frame limit, every frame 200 ms, looping forever. Four runs, each ended cleanly (`duration_elapsed`, hardware mode restored) with every record complete and 0 display failures. The first three drew 88–90 dial writes in 20 s (the code before the cadence fix below); the fourth drew 99. Records carry `trigger: "animation"` apart from the first.

**Write time.** The second run (the first had no timing in its records; dial `display_sync` records now carry `elapsed_micros`, as keypad records do): minimum 86 ms, median 102 ms, 90th percentile 111 ms, maximum 118 ms (121 ms in run four), for a frame near the size limit. So a worst-case frame takes about half of the 200 ms floor, and the floor is safe with margin; smaller frames write faster. A floor of 150 ms would also have fit; 200 ms stays because it is safe with margin and the five-a-second cadence is enough.

**Frame cadence.** The first two runs woke the display thread only at its 100 ms tick. A frame was then drawn up to a tick after its deadline, and since the rate limit counts 200 ms from the start of the previous write, the lateness piled up until a frame was skipped: 10 of 89 steps advanced by two frames (11 of 88 in the first run; a third run with a finer wake but still ignoring the rate limit gave 11 of 88 as well). While an animation plays the thread now sleeps until the draw can happen, which is when the rate limit opens if the frame on the clock is not the one drawn, or when the current frame ends otherwise. Run four: 99 writes in 20 s (5 a second, the most 200 ms frames allow), 1 of 98 steps skipped a frame, median write 103 ms, maximum 121 ms, 0 failures.

**Run five, floor lowered to 150 ms (19 October 2026).** The floor and the animation interval went from 200 to 150 ms (the official app's is 66 ms; see [GIF playback in the official app](research/masterhub-1.0.0.113-app-features.md#gif-playback-and-frame-timing-source-map-pass-19-october-2026)). The same six 44 KiB frames at 150 ms, 20 s: 131 dial writes (133 is the most 150 ms allows), 0 failures, 0 dropped records, ended on schedule. Dial records are now sampled (the first, then one in 50), so the cadence is read from the counters and from the sampled faces: sampled writes took 81, 93, 99 and 100 ms (a sample, not the maximum), and by the third sample (write 101) the face was one frame ahead of an unskipped run, so about one frame was skipped in the first 100 steps. The worst-case write measured earlier (121 ms) leaves about 30 ms under 150 ms.

**Run six, rate limit anchored to the frame deadline (19 October 2026).** At the floor the rate limit counted 150 ms from the start of the previous write, so a frame drawn a few milliseconds late made the next one late by the same amount, and the lateness built up until a frame was skipped (about 2 skipped in 20 s in run five). The limit now counts from when the frame was due, handing back at most a fifth of the interval (30 ms), so two writes stay at least 120 ms apart. The same profile for 60 s: 398 dial writes (400 is the most 150 ms allows), 0 failures, 0 dropped records, sampled write times 92 to 113 ms. The sampled faces show no skipped frame through write 251 and two skipped by write 301 (one of the writes or a stall was slow; the faces then stayed in step), so two in 60 s instead of two in 20 s, and the lateness no longer accumulates. The remaining skips are single events, not a build-up; their cause was not found.

**Seen on the dial (19 October 2026).** Two 45 s runs of the six noise frames at 150 ms (`--dial-content image`): no failed write, sampled write times 82 to 157 ms, and the user saw it smooth, without tearing or flicker, with the dial's built-in animation back afterwards. The noise was deliberate: random colour blocks compress to about 44 KiB, close to the worst case for a frame. Then a 60 s run of a 12-frame bouncing orange square (36 KiB in all, 150 ms per frame) while the user turned and pressed the dial: 144 right and 148 left turns, 21 short and 2 long presses, all logged and matched to their bindings (actions logged only, no `--execute`), no failed write, and the sampled faces advanced in step with a steady loop. The user saw it smooth. Light frames wrote in 3 to 10 ms, against about 100 ms for the noise frames, so the write time follows the frame size and the 150 ms floor is set by the worst case.

**Backend import path (20 October 2026).** The app's own `prepare_dial_animation` (`artwork.rs`, the call the editor makes) was run on the bouncing-square GIF with the editor's defaults (quality 85, cover fit, no rotation): 12 frames at 150 ms, looping forever, 36 KiB in all, and the result passed the same validation a saved profile gets. A 30 s run of that profile on the M131E had no failed write, write times of 4 to 9 ms, and sampled faces in step with the loop. This was a temporary unit test, not the app window: the file picker, preview, saving and reloading were not exercised, and nobody looked at the dial for this run.

**Import through the app window (20 October 2026).** `npm run sidecar` and `npm run desktop` (a debug build), then the dial **Display** tab, **Custom image**, and the 12-frame bouncing-square GIF (`bounce.gif`, 5.6 KB) from the file picker. The user reported that it worked. What was not looked at or written down: the exact preview and "Plays 12 frames" text, the look on the dial, and whether the GIF was still there after a restart.

**Loop count 3 (20 October 2026).** The 12-frame bouncing square with `loop_count` 3 (about 5.4 s of playback) ran twice for 30 s on the M131E. Both runs had no failed write, and each log held only two sampled `display_sync` records (the start and one animation write), against 7 to 9 in the 45 to 60 s runs that looped forever. The log samples one animation write in 50, so that fits about 36 writes, which is three loops, and not a loop that kept going. The log cannot show the frame that is held at the end, so the same profile was run again for 30 s with the user watching: the user saw the animation stop on its last frame.

**Restart with the imported GIF (21 October 2026).** After the app had been stopped and started again (`npm run desktop`), the user opened the profile and reported that the imported GIF was still on the dial's Display tab. Not written down: whether the preview text read "Plays 12 frames, looping.", whether the app offered a draft restore or the profile had been saved, and whether it played on the dial after the restart.

**Playback after restart, and function switching (21 October 2026).** With the app restarted and the imported GIF in the profile, the user ran it on the M131E and reported that all checks were green: the GIF played on the dial after the restart, and the dial switched between the animated picture and its other functions and back. Not written down: the frame count in the preview text, and any log numbers (no log was kept for this run). The in-app previews showed only the first frame until the same day, when the module illustration and the dial Display tab preview were made to play the GIF with the stored frame times; the user confirmed that they play.

**Still to check:** nothing for the dial GIF. (Mixed static and animated keys with folder navigation were checked on 21 October 2026; see the end of this file.) In the app: dial **Display** tab, **Custom image**, choose a GIF (up to 8 MiB, at most 30 frames, canvas up to 512×512; shorter frame times are raised to the frame's floor of 66, 100 or 150 ms by size; the frames together may not exceed 384 KiB, so the quality is lowered until they fit; a one-frame GIF becomes a still picture), Module control App. Needs the sidecar rebuilt (`npm run sidecar`).

## Custom dial picture (11 October 2026)

`masterhub-service run --software-mode --display-sync --dial-content image` on the M131E with a profile whose `dial_display` held a prepared 240×240 JPEG (7 KB, `dial prepare-image` at quality 85): a test card of four coloured quadrants, a white cross and a black tab in the top-left corner. The app and its helpers were closed first. Two runs of 25 and 45 seconds: the service reported one display sync with the face `image`, written, no failures, and ended on schedule. The user saw the card on the dial, upright; the black tab sat just outside the round border, as expected for a corner of a square picture on a round screen. After each run the dial returned to its built-in animation.

- **Through the app (11 October 2026):** the same test card, chosen on the dial's Display tab ("Custom image") and drawn by a normal app run, showed on the dial (user-observed).
- **Fit, rotation and background (12 October 2026):** through the app's dial Display tab with a 480×240 test picture (red square top-left, green top-right, blue bottom-left, a yellow F in the middle). Seven settings were tried in turn: contain, cover and stretch at 0°; contain at 90°, 180° and 270°; contain with a changed background colour. The user confirmed that each looked as expected on the dial. The direction each rotation turns was not written down separately. The service log showed no write errors.
- **Reopen (12 October 2026):** after quitting and restarting the app, the same profile's dial Display tab still showed the fit, rotation and background controls, so the source picture was kept in the profile. Changing a setting updated the dial (user-observed).

## Mixed static and animated keys across folders (M4315, 21 October 2026)

`masterhub-service run --profile … --software-mode --display-sync --duration-secs 90` on one M4315, dry run (no `--execute`). A temporary schema-2 profile with three pages: the root page held a static key, a two-frame blink (400 ms) and a three-frame fast cycle (150 ms) plus a folder key; the Sub page a static key, a two-frame blink (300 ms) and a folder key; the Deep page one two-frame blink (250 ms). The user walked root, Sub, Deep, back to Sub, back to root and reported all good by eye: the animations started cleanly on each page, with no stale frame, flicker or stuck key.

The log agrees: one `software_mode` redraw of all 15 keys (28 ms) and four `navigation` redraws (Sub, Deep, Sub, root; 8 to 23 ms), every one complete with no error. A navigation wrote only the keys whose picture changed (the gray folder key, identical on root and Sub, was not rewritten). Session: 10 reports, all processed, 0 dropped or coalesced, 666 display syncs including the animation frames, 0 display failures, ended by `duration_elapsed`. The first attempt was discarded because my own `head` cut the log pipe short.

## Media keys and media stop with Apple Music (M4315, 21 October 2026)

`masterhub-service run --profile … --software-mode --display-sync --execute --duration-secs 180` on the M4315, with a temporary schema-2 profile of four keys: key 1 `media` `play_pause`, key 2 `next_track`, key 3 `previous_track`, key 4 `stop`. (`--display-sync` is needed because the profile has keypad pages.) The first attempt, run without `--display-sync`, was refused at start; a second 150 s run got no key press at all (0 reports) because the user was not yet at the keypad; neither says anything about the actions. In the third run, with a song loaded in Apple Music, the log holds eight presses, each followed by `action_executed`: key 1 four times, key 2 once, key 3 twice, key 4 once. There is no `action_failed`, and stderr is empty. The user reported that all four keys worked: play and pause, next and previous track, and stop, on Apple Music.

Not checked or not written down:
- Which macOS permission prompts appeared (Accessibility for the media keys, Automation for the `stop` command), and which app they named. The user did not say, and the log has no record of them.
- A browser tab as the media-key owner. Spotify is not installed on this Mac, so Spotify's `pause` was never sent.
- Whether `stop` also worked when neither player was running, or when both were.

**Another service was running on the same keypad.** The desktop app was open (`npm run desktop`) and had started its own `masterhub-service` on the same M4315 with the profile "Streaming (OBS)", whose key 1 is "Record" (`obs_record` toggle). Both services therefore received the presses and drew the key faces, and key 1 showed RECORD right after it was pressed, as the user saw. That text is the app profile's label, not something this test wrote: the test's own service wrote the 15 key faces once, at start (`display_sync` `software_mode`, 15 written, none missing or failed). The `obs_record` toggle was not looked at; no OBS process was seen running at the time, and whether the app's service logged a failed action was not checked. A test that needs the keypad to itself should stop the app first.

## Macro recorder and replay (M4315, 21 October 2026)

Done by the user in the desktop app (`npm run desktop`, a debug build) on a real Mac: the **Record macro** button of the macro editor, then **Use recording**, then a run of the profile with Module control App and actions on, pressing the key on the M4315 with a text editor focused.

**Two defects found and fixed.**
1. **Clicking Record macro crashed the app** (twice, 14:16 and 14:17; both macOS crash reports show the same stack: `SIGBUS` in `CFRunLoopAddSource` called from the recorder's tap thread). `recorder.rs` declared `kCFRunLoopCommonModes` as an opaque value and passed its address; the symbol holds a `CFStringRef`, so Core Foundation read the variable's bytes as a string object. It now declares the pointer and passes its value. The bug had been there since the recorder was added; only a real event tap reaches that call, so no unit test could have found it. A search found no other `&kCF…` or `&kCG…` use in the Rust sources.
2. **Use recording did not close the dialog.** The handler applied the steps but never closed the recorder. It now does both.

**Result after the fixes.** The user recorded a sample, used it, and reported that the replay of the macro works. Not written down: what was typed, how many keystrokes the recording became, whether the recorder dialog showed the permission rows as expected, which permission prompts appeared and which app they named, and whether the replayed timing matched. The key chord replay of Command combinations and named keys was not singled out.

## Profile key, and the editor following it (M4315, 21 October 2026)

Two test profiles were imported through the app (`AAA Switch A` and `AAA Switch B`, generated for the check): key 1 is a profile key (`next` on A, `previous` on B) and key 2 a 5 s timer without sound. The names sort first, so A's next is B. The app (`npm run desktop`, a debug build) ran A on the M4315 with Module control App.

**What the user saw first.** (1) After importing A, the editor and module image showed A while the device kept showing the profile it was already running; this is by design, because the editor and the running profile are separate. (2) Pressing the profile key switched the device to the next profile, but the module image in the app stayed on the old one. The user asked for the editor to follow the run.

**Change.** When a profile key switches a run, the editor now opens the switched profile, unless that would replace unsaved changes (it keeps them and shows a note instead). Eight unit tests cover the rule; the wiring in `App.tsx` has no test, because the vitest setup has no DOM. The user checked it on the device and reported it verified.

Not written down: whether the dry-run switch went A to B and back, whether quick presses switched only once per 500 ms, what the executing run's confirmation dialog said, and whether the keypad faces changed with the profile after confirming. The profile key's switch on a real run is therefore only partly checked: the device switching on a press and the editor following were seen.

**Executing run and quick presses (21 October 2026, later).** With `AAA Switch A` running with actions on, the user pressed the profile key, went through the confirmation, and tried quick presses and a declined confirmation (the three checks listed above), and reported "All good". The banner and Resume dialog text, whether key 2 stayed idle before the confirmation, and how many confirmations the quick presses needed were not written down, so this records that nothing looked wrong, not the details.

## The remaining OBS actions with a real OBS (21 October 2026)

OBS Studio 5.7.4 on the Mac, WebSocket server on (port 4455, password required, the same endpoint and saved password as the app's OBS setting). OBS was closed while test content was added as new files, with nothing existing touched: scene collections `OSH Test` (scenes `OSH Live` and `OSH Other`; a colour source `OSH Box` in `OSH Live` with a colour-correction filter `OSH Tint`) and `OSH Test 2` (scene `OSH Two`), and a profile `OSH Test Profile` (a copy of the `Untitled` settings with a custom stream target `rtmp://127.0.0.1:1935/live`, where nothing listens, so a stream could only fail to connect). These files were written by hand from the format of the existing `Untitled` collection. A temporary schema-2 profile `OSH OBS check` (it passes `masterhub-service validate`, 11 bindings) put one action on each of 11 keys of the M4315: `obs_scene` to `OSH Live` and to `OSH Other`; `obs_filter` toggle; `obs_source_visibility` toggle; `obs_scene_collection` to `OSH Test 2` and back; `obs_profile` to `OSH Test Profile` and back to `Untitled`; and `obs_stream` start with `confirm` `press_twice`, stop, and toggle. It was imported, allowed to execute actions, and run with actions on.

**Result.** After pressing the keys the user reported "All good". That covers the scene, filter, source visibility, scene collection, profile and stream actions against a real OBS for the first time.

Not written down: what each key looked like in OBS, any activity-log lines, whether the stream keys showed the expected connection failure in OBS and not an error in the app, whether one press of the armed stream key only armed it, and whether OBS answered a collection or profile switch before or after its reload. No broadcast was made; the stream target cannot receive one, so this does not show a stream going live.

## Dial write time by frame size (M131E, 21 October 2026)

To choose a size-aware dial floor, six profiles each held two different noise frames of one size, played at the 150 ms floor with `masterhub-service run --software-mode --display-sync --dial-content image` (dry run, 20 s each, one run per size). The service was built with its sampled logging of dial animation frames temporarily set to every frame (not committed), so each write has its own `elapsed_micros`. The frames came from `masterhub-diag dial prepare-image` at JPEG quality 3, 10, 20, 40, 55 and 70 on one 240×240 noise image, which is a near worst case for a given quality.

| Frame size | Writes | Min | Median | 90th pct | Max |
|---|---|---|---|---|---|
| 2.8 KiB | 133 | 1.8 ms | 6.5 ms | 9.4 ms | 14.2 ms |
| 7.7 KiB | 133 | 10.7 ms | 18.8 ms | 22.9 ms | 29.3 ms |
| 14.3 KiB | 133 | 15.3 ms | 31.5 ms | 36.7 ms | 47.1 ms |
| 26.7 KiB | 132 | 47.3 ms | 62.1 ms | 69.4 ms | 82.8 ms |
| 34.7 KiB | 132 | 63.4 ms | 81.6 ms | 93.3 ms | 99.3 ms |
| 45.6 KiB | 132 | 85.7 ms | 105.6 ms | 116.1 ms | 123.3 ms |

The time grows about linearly with size: roughly 2.3 ms per KiB at the median, and a maximum of about 6 ms plus 2.7 ms per KiB. Every run made 132 or 133 writes in 20 s (133 is the most 150 ms allows), and no write failed while the animation ran. The three largest runs each logged one failed write, the last frame before the end of the run (`display writes are closed`): the final frame racing the shutdown, not a slow or short write. It counts in `display_failure_count`. Nobody looked at the dial for these runs.

What this means for the floor: the largest frame (45.6 KiB, maximum 123 ms) leaves about 27 ms under 150 ms, as before. A frame must not be due again before its write is done, so a floor of about the maximum write time plus 30 ms is safe: roughly 2.7 ms per KiB plus 35 ms.

## Size-aware dial floor, seen on the dial (M131E, 21 October 2026)

**Change.** The dial's shortest frame time now follows the frame's size instead of one 150 ms for all: 66 ms for a frame up to 8 KiB, 100 ms up to 20 KiB, 150 ms above (`DialAnimation::min_delay_ms_for`). Profile validation, the GIF importer (it raises a delay to the floor of the prepared frame's size) and the playback loop (the write rate limit follows the current frame's floor) all use it. The tiers come from the measurements above; the largest frames keep the 27 ms margin they had at 150 ms. Older builds refuse a profile that uses a delay under 150 ms. Rust tests cover the tier boundaries, the validation by size (at the floor accepted, one millisecond below refused, for noise frames in all three size classes), the importer's floor and the playback floor; clippy and the workspace and app tests pass.

**Seen.** A generated GIF, a flat dark background with an orange square bouncing off the floor and across the screen (28 frames of 70 ms, about 14 frames a second, 10 KB; the earlier first attempt had a static square, because `drawbox` has no time variable, and a dithered background, and was replaced), imported through the app window and run with Module control App. The user reported that it was smooth and fast. Not written down: the preview text, the frame sizes the importer produced (a few KiB each, so the 66 ms floor applied and the 70 ms delay was kept), the write times and whether any frame was skipped. No log was kept for this run, and a large-frame GIF was not run on the dial after the change (large frames keep the 150 ms floor measured before).

**Three versions of a dial GIF, in the app window (21 October 2026, later).** On top of the size-aware floor, the dial Display tab now prepares an imported GIF three ways (the app command `prepare_dial_animation` takes an optional `frame_budget_kib`, 1 through 48): Sharp (quality 85, the 48 KiB frame limit as before), Balanced (frames up to 20 KiB, so a floor of 100 ms) and Fast (frames up to 8 KiB, a floor of 66 ms), shown one at a time as a playing preview with arrows and a Use button, and a **Compare versions of a GIF** toggle (on by default; off prepares only the one in use). A test GIF of a detailed test pattern, 30 frames at 40 ms, was made for this (frame sizes measured at quality 95, 85, 70, 55: about 24, 14, 10 and 8 KiB, so the three classes are reached). The user looked at it in the app window and gave design feedback on the layout twice (the version picker was above the controls it depends on, the arrows sat far from the picture, the text was repeated, the preview was square while the dial is round, and the same animation was shown twice); each was changed. The tests pass (Rust clippy and the workspace and app tests, vitest 1255). Not written down or not done: what the three versions looked like on the dial, whether Fast was visibly faster than Sharp, the frame rates the cards showed for the test GIF, and whether a GIF too detailed for the Fast setting showed its card as unavailable. The Balanced and Fast versions have not been run on the dial.

