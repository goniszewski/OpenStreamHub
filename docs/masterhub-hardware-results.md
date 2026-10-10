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
| M005S faders | n/a | Not connected. |

## USB topology and reported physical layout

The following maps the read-only macOS `ioreg` location IDs to the user's reported current arrangement (9 October 2026). This is a mapping for this connected base and orientation, not a general layout discovery implementation.

| Module | `locationID` path | Reported base position | Product footprint |
| --- | --- | --- | --- |
| M4315 15-key | `1.1.2.2` | Top-left | 4x3 |
| M4315 15-key, second arrangement (10 October 2026) | `1.1.2.3` | Bottom-left, with the rollers above it | 4x3 |
| M032LE 2x roller | `1.1.1.1.4` | Bottom-left, below the keypad | 4x1 |
| M131E dial | `1.1.1.4.3` | Top-right | 2x2 |
| M003K knobs | `1.1.1.3.4` | Bottom-right, below the dial | 2x2 |
| M0064 base | `1.1.1.4.4` | Base; USB-C ports are on the left and top edges in landscape orientation | 4x6 |

Interpreting the product-name dimensions as module footprints, they total 24 cells (12 + 4 + 4 + 4), matching the 4x6 base. With the base in landscape and its USB-C port edges used as the top/left reference, the reported arrangement tiles the base: keypad and rollers occupy the left 4x4 area; dial and knobs occupy the right 2x4 area. The user reports no free module space. The physical positions come from the user's description; the paths come from the earlier read-only `ioreg` inventory. No slot information has been read from a base or module HID feature report, and a second arrangement has not been tested.

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
- **Still pending.** A check against a real OBS 28+ install, including the password-required path and refresh after a scene rename.

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
- The M005S faders (not connected) and all lighting writes beyond the dial backlight and color.
- Most of the desktop app against real hardware, and every `obs_*` action against a real OBS.
- Sleep and wake, executing-session reconnect, and reconnect on other modules.
