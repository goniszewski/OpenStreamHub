# First MasterHUB hardware session

Ordered checklist for the first session with real hardware. It exists to close the gates listed under "Remaining gates" in [implementation status](masterhub-implementation-status.md). Every claim it tests is currently implemented and synthetic-tested only; see the [evidence table](masterhub-protocol-evidence.md). Nothing here is evidence until a step has been run and its result row filled in.

## Ground rules

- No firmware writes, flashing or recovery, ever. The SDK has no such command; do not use any other tool for it in this session.
- Every step is marked `READ` (enumeration or reads only), `QUERY` (vendor feature GET; changes no state, but talks to the device), or `WRITE` (feature/output report to the device). `WRITE` steps are opt-in, one at a time, and each has a restore step. Skip any `WRITE` step you are not comfortable with; the session stays useful without them.
- Always run `READ` steps for a module before its `WRITE` steps (neutral baseline first).
- Record one physical action per capture and keep a timestamped action log (`captures/hardware/action-log.txt`: wall-clock time, module, control, gesture). Captures use the host clock only for the session start; align via the log.
- Captures contain serial numbers and device paths. They stay under the git-ignored `captures/`. Review before sharing.
- macOS recording shows only what the SDK reads. It does not sniff another application's traffic. Close the official MasterHUB software (if installed) and any other HID tool first.
- Keep the working tree and binary fixed during the session; `system.txt` in every output records git revision and dirty state.

## Setup

```sh
cargo build --release -p cooltek-sdk-cli --locked      # builds masterhub-diag and masterhub-service
cargo test --workspace --locked                         # optional, no hardware
mkdir -p captures/hardware
echo "# date  module  control  gesture  expected  observed" > captures/hardware/action-log.txt
```

Helper scripts (see [Scripts](#scripts)) wrap the commands below. They need `jq`, `shasum`, and a prebuilt `masterhub-diag`. All commands are run from the repository root. In commands, `masterhub-diag` means `target/release/masterhub-diag`.

First-run macOS note: if reads fail or return no data, check System Settings, Privacy and Security, Input Monitoring for the terminal application. Record whether a prompt appeared (see step 8).

## Checklist

### Step 0. Inventory and topology (READ) — all modules

```sh
scripts/hardware/collect-inventory.sh
```

Runs `masterhub-diag list` plus `sw_vers`, `uname`, `system_profiler SPUSBDataType` and `ioreg -p IOUSB`. Nothing is opened.

Do it three times: (a) base alone, (b) base plus one module at a time, (c) fully populated. Use a new output directory each time (the script creates one).

| Question | Look at | Expected observable | Confirms or refutes |
| --- | --- | --- | --- |
| Which PID generation does each module use? | `summary.tsv` / `list.json`: `vendor_id` 9494 (0x2516), `product_id` | Base 502 (`01f6`) or 39321 (`9999`); knobs 514 (`0202`) / 39318 (`9996`); dial 518 (`0206`) / 39316 (`9994`); keys 510 (`01fe`) / 39319 (`9997`); faders 512 (`0200`) / 39313 (`9991`); rollers 516 (`0204`) / 39317 (`9995`) | Evidence table "Module PID generations"; status "Module PID aliases". Refuted by any other PID or a `module` of `unknown`. |
| Does each module enumerate on its own, or only through the base? | Entries per `product_id` in (a) versus (b) | Module entries appear only with the base attached, or independently. Note which. | Plan "device sessions": topology assumption (unverified). |
| How many HID interfaces does each module expose, and with which usage pages? | Count of entries per product; `interface_number`, `usage_page`, `usage` | One or more entries per module. Vendor usage page `65280` (0xff00) expected for the control interface (synthetic fixtures use it; this is not hardware-verified). | Gate "interface selection". Refutes the assumption that a single interface exists. |
| Are USB serials present and unique? | `serial_number`; `ioreg_iousb.txt` | Serial strings present, unique per module. Note any duplicates or nulls. | Plan: identify by serial with fallback; list serials may differ from feature-report serials. |
| Is the base (M0064) itself a HID endpoint, or only a hub? | `system_profiler_usb.txt` tree | Note hub chain and where modules attach (powered hub, internal hub, direct). | Topology; Raspberry Pi USB constraints section of the [roadmap](masterhub-macos-plan.md). |

Save: whole output directory. Photograph the physical module arrangement with positions numbered.

### Step 1. Serial/report framing and feature queries (QUERY)

```sh
cargo run -p cooltek-sdk-masterhub --locked --example get_module_serial_numbers
```

Opens each recognized module without initialization and issues the serial feature query (report `30` for M0 models, `4a` for M131E). Compare against the `list` serials.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Do serial replies parse (header, report ID, length)? | One line per module: product name and serial. A `Serial query failed` line means a framing assumption failed; paste the error. | Evidence "Serial feature response". |
| Is the feature serial equal to the USB string serial? | Same or different; record both. | Diagnostic doc note that they may differ. |
| Does every module answer, including M0064? | Per-module result. | Which modules support `CanGetSerialNumber`. |

Dial only:

```sh
masterhub-diag dial info --path-hex DIAL_PATH_HEX        # QUERY
```

Saves four raw firmware bytes, eleven text bytes and the backlight value. Run on each dial interface if more than one exists, and note which succeeds. Expected: success on one interface; reply byte 1 stays opaque. Confirms or refutes "M131E feature queries" (selectors `47`, `48`, `49`, ID `4c`). Record the firmware bytes verbatim for every module version.

### Step 2. Descriptors (READ)

Descriptors are saved by every recording (`report_descriptor` in the first line of the JSONL) and extracted to `*.hid` by `record-session.sh`. A short idle recording is enough:

```sh
scripts/hardware/record-session.sh base 3 --label idle
scripts/hardware/record-session.sh knobs 3 --label idle
scripts/hardware/record-session.sh dial 3 --label idle
scripts/hardware/record-session.sh keys 3 --label idle
scripts/hardware/record-session.sh faders 3 --label idle
scripts/hardware/record-session.sh rollers 3 --label idle
```

The script records every interface of a module concurrently, so interface selection can be compared. Idle captures also show whether a module streams reports unprompted (initial fader positions, for instance).

| Question | Look at | Expected observable | Confirms or refutes |
| --- | --- | --- | --- |
| Is the report descriptor readable per interface? | Line 1 of each `.jsonl`: `report_descriptor.status` | `available` with `raw_hex`, or `error` with message. Keep both. | "HID output report sizing" (no guessed fallback). |
| Output report ID `02` size for dial and keys | `xxd *.hid`; decode with any HID descriptor parser, or run `dial jpeg --descriptor` offline (Step 6c) and read the `output_report` lengths | Dial and keys interface carries output report ID `02`; note its length in bytes including the ID. | Gate "descriptor sizes". Synthetic 65 bytes is a test value only. |
| Input report sizes and IDs | Same `.hid` decoded | Input ID `01` for M0/M131E/M4315 frames expected; note lengths. | "M0 raw input frame", "M131E input". |
| Which interface actually delivers input? | `list`-matched capture files: nonempty `report` records in step 3 | Exactly which `iface<N>` file receives reports. | Interface selection. |
| Idle chatter | `*.replay.txt`, report count in the `end` record | Zero or a stable number of reports at idle. Any periodic report is a finding. | Startup/initialization requirement. |

### Step 3. Physical control order and gestures (READ) — per module

Neutral baseline: no `--software-mode`. For each control, record a short capture, perform exactly the gesture written, and log it. Use `--label` to name the file, for example `--label knob1-cw`. Ten seconds per capture suffices; the script stops at its deadline even when nothing arrives.

Common command shape (each interface of the module is recorded concurrently):

```sh
scripts/hardware/record-session.sh knobs 10 --label knob1-cw-3detents
```

Replay a capture offline at any time:

```sh
masterhub-diag replay captures/hardware/knobs-<timestamp>-<label>/iface0-pid0202-0.jsonl
```

#### M003K knobs (3 knobs plus presses)

Capture order, each as its own file: knob 1 three detents clockwise; knob 1 three counter-clockwise; same for knob 2 and knob 3 (left to right as seen from the front, note the orientation you assume); press and release each knob once; hold each knob for about one second; turn two knobs simultaneously.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Zero-based index to physical placement | Replay shows knob index 0, 1, 2 for the knobs in some order. Fill the placement column of the results table. | Diagnostics doc: "physical placement still needs checking". |
| Direction byte | Clockwise produces channel-0 payload direction byte `01`; counter-clockwise `00`; `ff` unchanged. | Evidence "M003K rotation" (probe notes). A swap refutes it. |
| Press/release | Logical payload `03 03` plus three states; transitions only | Evidence "M003K press/release". |
| Framing | `012a` + little-endian length + payload at offset 4 | Evidence "M0 raw input frame". |
| Simultaneous | One report with two nonzero direction bytes, or two reports | Decoder behavior for combined movement. |

#### M131E dial (neutral baseline)

Files: one detent right, one detent left, three each, a fast spin (about one second), press and release, hold about 0.8 s and release (host long-press threshold is 600 ms), tap several times quickly.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Does the dial report input before software mode? | Nonempty reports in the neutral capture, or none. | Gate "required initialization". If none: step 6 is needed to see input. |
| Direction: application RIGHT/LEFT vs. physical | `replay` prints `right`/`left` for the turn. Record which physical rotation (clockwise from above) yields `right`. | Status: "physical clockwise/counterclockwise mapping remains unknown". |
| Payload layout | Queue-0 raw `01 44 u16LE length`, then payload; payload byte 1 retained; extra bytes retained | Evidence "M131E input"; save the raw `raw_hex` of every distinct report. |
| Press/release and gesture | Press, release transitions without duplicates; held 0.8 s release shows `long_press`, quick tap `short_press`, with `held_micros` | Evidence "M131E host gestures" (600 ms is host policy, not a firmware event). |
| Repeated detents | Each detent is a separate event; fast spin yields many | Decoder; and whether any reports are lost (count detents turned vs. events). |

#### M4315 keys (15 keys)

Files: press and release each key individually, in your numbered physical order (1 to 15, left to right, top to bottom); two simultaneous keys; hold one key for about one second.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Index to physical key | Replay shows key index 0 to 14 for each press. Fill the table. | "Physical order … unverified". |
| Frame | `01 34` + length; 15-state snapshot; selectors `02` and legacy `00` | Evidence "M4315 key input". |
| Neutral input | Reports arrive without software mode, or not. | Required initialization. |
| Release without extra events | Press then release gives exactly two transitions. | Duplicate suppression. |

#### M005S faders (5 faders)

Files: move each fader from bottom to top slowly, then top to bottom; slam one fader quickly; leave all alone for the idle baseline already captured in step 2.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Fader order and scale | Logical `02 05` plus five bytes 0 to 100; each fader maps to one position index. Note endpoint values (0 and 100 exactly, or clamped differently). | Evidence "M005S fader input". |
| Jitter | Position changes while untouched, or flicker of 1 count. | Whether the decoder needs a deadband (a finding, not a claim). |
| Initial snapshot | Positions reported at open or only on movement. | "Startup pending". |
| Direction | Up equals higher value? | Physical range. |

#### M032LE rollers (2 rollers)

Files: each roller one step up, one down, several steps up quickly; simultaneous.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Direction and order | Logical `01 02`; `00` Up, `01` Down, `ff` idle; map to roller 1 and 2 | Evidence "M032LE roller input". |
| Magnitude | One report per step, or repeated | No magnitude is invented by the decoder; confirm none exists. |
| Release/idle | `ff` returns after a step | Repeated directions remain events. |

#### M0064 base

The base exposes no controls in the evidence. Record 10 s with `--label base-idle`. Expected: no input reports. Any report is a new finding; keep the raw file. Check whether the base's lighting or status LEDs react to module insertion (note only; base lighting is tried in step 6e).

### Step 4. Service replay of real captures (READ, no hardware)

Feed real captures to the binding service in plan-only mode to confirm decoding matches the physical gestures:

```sh
masterhub-service replay --profile examples/profiles/all-modules.json captures/hardware/<dir>/<file>.jsonl
```

Expected: planned actions per gesture, no execution, diagnostics only for genuine malformed/unknown reports. Replay never opens hardware or dispatches actions.

### Step 5. Native HID open and timeouts (READ)

| Question | Command | Expected observable | Confirms or refutes |
| --- | --- | --- | --- |
| Does exact-path open succeed without special privileges? | Any record from step 3 | Capture ends with `capture_end` record, exit 0. A permission error is a finding (Input Monitoring). | Gate "native HID open"; plan "confirm actual macOS access". |
| Idle behavior | `masterhub-diag record --path-hex HEX --output captures/hardware/idle-60.jsonl --duration-secs 60 --timeout-ms 100` | Ends at the deadline (see `reason` in the `capture_end` record) with zero or few reports. | Idle timeouts are not errors. |
| Longer read timeout | same with `--timeout-ms 1000 --duration-secs 10` | Deadline still honored (timeout reduced near the deadline). | Bounded deadlines. |
| Does a second process fail cleanly? | Run two `record` commands on the same path simultaneously | Note whether the second fails, shares, or steals reports. | Single-owner design assumption. |
| Interrupt | Start a 60 s record, press Ctrl-C once | `interrupted` `capture_end` record, exit status noted. | Documented interrupt behavior. |
| Wrong-interface read | Record each non-control interface | Errors or silence; note exact messages. | Interface selection. |

### Step 6. Software mode, lighting and display (WRITE) — dial and keys

All WRITE steps need explicit confirmation flags or an explicit `--path-hex`. Preview first (no hardware): previews print the exact 32-byte report.

```sh
masterhub-diag dial mode software          # preview only, expected 03 49 01 + zero padding
masterhub-diag dial mode hardware          # preview only, expected 03 49 00
masterhub-diag dial backlight 50           # preview: 03 44 32
masterhub-diag dial color 123456           # preview: 03 46 12 34 56
```

#### 6a. Software-mode requirement (WRITE)

Compare the neutral baseline of step 3 against a software-mode capture:

```sh
scripts/hardware/record-session.sh dial 15 --label sw-mode --software-mode --yes-write-hardware
scripts/hardware/record-session.sh keys 15 --label sw-mode --software-mode --yes-write-hardware
```

Writes `03 49 01` (dial) or `03 3a 01` (keys), then attempts `03 49 00` or `03 3a 00` at exit. Turn or press during the capture.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Is software mode required for input? | Compare report counts neutral vs. software mode for the same gesture. | Gate "required initialization"; evidence "software/hardware mode". |
| Does the write succeed? | Exit status 0, stderr empty. Failure text is evidence. | Evidence rows for modes. |
| Does restore work? | After the run, press/turn: the module returns to its built-in behavior (screen/keys show factory content) and a neutral capture again reports as in step 3. Also check after killing the process with SIGKILL (a documented non-guarantee): `kill -9` during a software-mode record, then observe state. | "Opt-in recording attempts hardware-mode restoration"; physical recovery behavior. |
| Is a power cycle or replug needed after a crash? | Note exactly what was needed to recover. | Lifecycle gate. |

If a module does not return to hardware mode, run `masterhub-diag dial mode hardware --path-hex DIAL_PATH_HEX` (WRITE), then replug; log it.

#### 6b. Dial color channel order and backlight (WRITE, dial in software mode)

Open the dial in software mode in a second terminal (`record ... --software-mode`, 120 s) or use the direct commands:

```sh
masterhub-diag dial backlight 20  --path-hex DIAL_PATH_HEX    # WRITE
masterhub-diag dial backlight 100 --path-hex DIAL_PATH_HEX    # WRITE
masterhub-diag dial color ff0000  --path-hex DIAL_PATH_HEX    # WRITE
masterhub-diag dial color 00ff00  --path-hex DIAL_PATH_HEX    # WRITE
masterhub-diag dial color 0000ff  --path-hex DIAL_PATH_HEX    # WRITE
masterhub-diag dial info          --path-hex DIAL_PATH_HEX    # QUERY: read back backlight
masterhub-diag dial mode hardware --path-hex DIAL_PATH_HEX    # WRITE: restore
```

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Backlight scale | 20 visibly dimmer than 100; `dial info` returns the set value | "M131E backlight" `03 44 <0..100>`. |
| Color channel order | `ff0000` shows red if channels are R,G,B; green if G,R,B; etc. Record the three observed colors and photograph. | Open item "color channel order". Refutes the RGB assumption if `ff0000` is not red. |
| Does color/backlight need software mode? | Repeat from hardware mode; note any effect | Mode dependency. |

#### 6c. Display image orientation, acceptance, byte 6, pacing (WRITE)

Prepare images offline (no HID). Use an asymmetric test image (an arrow or letter F with colored corners) so orientation is unambiguous:

```sh
masterhub-diag dial prepare-image test.png --output captures/hardware/dial-0.jpg
masterhub-diag dial prepare-image test.png --output captures/hardware/dial-90.jpg --rotate 90
masterhub-diag keypad prepare-image test.png --output captures/hardware/key-0.jpg
```

Plan against the saved real descriptor first (offline, shows packet sizes):

```sh
masterhub-diag dial jpeg captures/hardware/dial-0.jpg \
  --descriptor captures/hardware/<dir>/iface<N>-pid0206-<n>.hid \
  --output captures/hardware/dial-0-plan.jsonl
```

Then, with the dial in software mode (6a), upload:

```sh
masterhub-diag dial jpeg captures/hardware/dial-0.jpg --path-hex DIAL_PATH_HEX   # WRITE
masterhub-diag dial jpeg captures/hardware/dial-90.jpg --path-hex DIAL_PATH_HEX  # WRITE
```

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Does the live upload run to completion? | Exit 0; no short-write error | "no pacing/no ack" design. A short write or stalled transport is a pacing finding. |
| Image appears? | Display shows the test image | Header `02 45`, 240×240 baseline JFIF acceptance. |
| Header byte 6 | Image accepted with byte 6 zeroed (the SDK zeroes it). If rejected, garbled, partial, or stale, record exact behavior. | Open item "header byte 6". |
| Orientation | Which of `--rotate 0/90/180/270` shows the image upright for the installed orientation | Open item "image orientation". Record the rotation value to use. |
| Color accuracy | Test image colors (red, green, blue patches) show true | RGB order in JPEG vs. display. |
| Pacing | Repeat the upload 20 times back to back (shell loop is fine) and watch for corruption or missing frames; then repeat with `sleep 0.05` between. | Open item "pacing". Findings only: no delay is built in. |
| Rejected formats | Optional: nothing needed. | None; progressive JPEG is unsupported by the tool. |

Keys: the CLI has no live key upload. `keypad jpeg` only plans a JSONL packet file offline. Do the key image offline check against the saved real descriptor so the output length is real:

```sh
masterhub-diag keypad jpeg captures/hardware/key-0.jpg --key-index 0 \
  --descriptor captures/hardware/<dir>/iface<N>-pid01fe-<n>.hid \
  --output captures/hardware/key-0-plan.jsonl
```

Record: the output report length, and that live key upload (header `02 36`, byte 6 = key index, final flag in byte 7) remains untested. Do not write the plan to the device by other means in this session.

#### 6d. Experimental dial clock, function name and CPU usage (WRITE, behind the flag)

Only after 6c showed that a dial image appears upright. These runs use the experimental `--display-sync --dial-content` path; no actions execute without `--execute`. Watch the dial and keep the log:

```sh
masterhub-service run --profile examples/profiles/dial-functions.json \
  --path-hex DIAL_PATH_HEX --duration-secs 150 \
  --software-mode --display-sync --dial-content clock | tee captures/hardware/dial-clock.jsonl          # WRITE
masterhub-service run --profile examples/profiles/dial-functions.json \
  --path-hex DIAL_PATH_HEX --duration-secs 60 \
  --software-mode --display-sync --dial-content analog-clock | tee captures/hardware/dial-analog.jsonl  # WRITE
masterhub-service run --profile examples/profiles/dial-functions.json \
  --path-hex DIAL_PATH_HEX --duration-secs 60 \
  --software-mode --display-sync --dial-content function | tee captures/hardware/dial-function.jsonl    # WRITE
masterhub-service run --profile examples/profiles/dial-functions.json \
  --path-hex DIAL_PATH_HEX --duration-secs 60 \
  --software-mode --display-sync --dial-content cpu | tee captures/hardware/dial-cpu.jsonl              # WRITE
```

In the function run, short-click the dial several times, slowly and then quickly. In the CPU run (macOS only), start some load in another terminal halfway through (for example `yes > /dev/null`, stopped with Ctrl-C) so the percentage moves.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Clock appears right after start? | `HH:MM` local time within about a second; `display_sync` record with `trigger: software_mode`, `complete: true` | Live dial path, generated face acceptance. |
| Clock advances once per minute? | One `trigger: clock` record per minute boundary, none in between | Minute-change redraw and rate limit. |
| Legibility and orientation | Digits and hands readable, upright, not clipped by the round mask | Face layout; record a photo. |
| Function name follows clicks? | Name and `n/N` change after each click; rapid clicks settle on the newest function within about a second | `dial_function` trigger, newest-wins coalescing, 1 s rate limit. |
| CPU face follows load? | About a second after start, a percentage and ring (no `--` first); a `trigger: cpu` record only when the whole percentage changes, never more than one dial write a second; the value tracks Activity Monitor's CPU total roughly | CPU sampler, change-only redraw, 1 s rate limit under sustained redraws. |
| Firmware under once-a-second redraws? | Under changing load the dial is written about once a second for the whole run; no tearing, stalls or `failed` records | Whether the 1 s rate limit is safe for continuous redraws. |
| Input stays responsive while drawing? | Rotations log their actions without visible delay during redraws | Shared-device read slicing. |
| Clean exit | Ctrl-C ends the run; `service_end.display_stopped: true`; dial returns to hardware mode; no image is drawn after exit | Gate closed before the hardware-mode restore. |
| Short writes | Any `failed.short_write: true`? The same face is not resent; the next minute or click redraws | No-retry policy on hardware. |

The desktop app passes the same flags by default: with Settings → **Module control** set to **App** (the default), choose the dial content and start a profile on the module; software mode is requested automatically. The setting applies at the next Start only.

#### 6e. Knob, fader and base LEDs (WRITE, no software mode)

Needs the knob module, ideally also the faders and the base. These commands come from disassembling the official SDK and have never been sent to hardware, so run them one at a time, never with `--save`, and stop at the first surprise. Stop the app and `masterhub-service` first. Get each module's `path_hex` from `list` (the interface that declares output report `02`).

```sh
# What does each index drive? Watch the module while this runs, note each step.
cargo run -p cooltek-sdk-cli --locked -- led sweep --module knob   --path-hex KNOB_PATH
cargo run -p cooltek-sdk-cli --locked -- led sweep --module slider --path-hex SLIDER_PATH
cargo run -p cooltek-sdk-cli --locked -- led color ff0000 --module base --index 0 --path-hex BASE_PATH

# Channel order, one LED at a time (expect red, green, blue).
cargo run -p cooltek-sdk-cli --locked -- led color ff0000 --module knob --index 0 --path-hex KNOB_PATH
cargo run -p cooltek-sdk-cli --locked -- led color 00ff00 --module knob --index 0 --path-hex KNOB_PATH
cargo run -p cooltek-sdk-cli --locked -- led color 0000ff --module knob --index 0 --path-hex KNOB_PATH

# Different colours at once, then brightness, then whole module.
cargo run -p cooltek-sdk-cli --locked -- led color ff0000 --module knob --index 0 --path-hex KNOB_PATH
cargo run -p cooltek-sdk-cli --locked -- led color 00ff00 --module knob --index 1 --path-hex KNOB_PATH
cargo run -p cooltek-sdk-cli --locked -- led brightness 20 --module knob --index all --path-hex KNOB_PATH
cargo run -p cooltek-sdk-cli --locked -- led color 0000ff --module knob --index all --path-hex KNOB_PATH
```

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Index to LED | The sweep lights one part per index; the knob module has five indexes for three knobs | Which index drives which ring, segment or other LED |
| Per-LED colour | Two indexes hold different colours at the same time | The protocol colours LEDs separately, which the official app never shows |
| Channel order | `ff0000`, `00ff00`, `0000ff` show red, green, blue | The assumed R, G, B order |
| Brightness | `20` is visibly dimmer than the default; check whether it applies per LED or to the module | Brightness byte layout |
| Faders and base | Three indexes on the faders, one on the base, each lights something | Type bytes `2` and `0` |
| Persistence | Replug the module: it returns to its own lighting (no `--save` was used) | That unsaved writes do not stick; optionally try one `--save` colour and note whether it survives a replug |
| Interaction | Whether a colour write needs the module in any mode, and whether input still works while LEDs are set | Mode dependency |

Record each observation with the index, the colour and a photograph.

### Step 7. Live service on real input (READ, plan-only)

Plan-only run: actions are logged, nothing executes (no `--execute`). A regular-file redirect is rejected; pipe through `tee`.

```sh
masterhub-service run --profile examples/profiles/all-modules.json \
  --path-hex HEX --duration-secs 60 | tee captures/hardware/service-<module>.jsonl
```

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Do real gestures produce the planned actions? | records with `status: planned` in the same order as the action log | Service gate: "agreed gesture sequence gives correct events". |
| Fast/simultaneous input | Spin dial/knob quickly for 5 s | No overflow failure in the final status; report counts consistent with step 3. |
| Stuck presses | Press and hold, Ctrl-C | Clean end, no repeated actions. |

Execution (`--execute`) and volume-changing fader/knob actions need a separate acceptance (see the [service guide](masterhub-service.md) and [macOS smoke checks](masterhub-macos-smoke.md)); leave them out of this session.

#### 7a. OBS scene switching (optional; needs OBS 28+ on this Mac, no module writes)

Only if OBS Studio is installed. In OBS, open Tools > WebSocket Server Settings, enable the server, note the port (default 4455) and, if authentication is on, the password. Create scenes named `Starting soon`, `Live` and `Be right back`. This executes actions, but the only effect is an OBS scene change.

```sh
printf '%s\n' "$OBS_PASSWORD" | masterhub-service run --profile examples/profiles/obs-scenes.json \
  --path-hex HEX_OF_THE_KNOBS --execute --duration-secs 60 \
  --obs-endpoint 127.0.0.1:4455 --obs-password-stdin | tee captures/hardware/service-obs.jsonl
```

Leave out the `printf … |` and `--obs-password-stdin` when OBS authentication is off. In the desktop app, the same check is Settings > OBS Studio (turn it on, enter the password), then start the profile with Execute.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Does a press switch the scene? | Knob 1 press: OBS shows `Starting soon`; knob 2: `Live`; `action_executed` records | `obs_scene` against real OBS (only fake servers so far) |
| Is the password kept out of logs? | `grep -c "$OBS_PASSWORD" captures/hardware/service-obs.jsonl` prints 0; `ps -ww -o args= -p <pid>` during the run shows only `--obs-endpoint 127.0.0.1:4455 --obs-password-stdin` | Password channel |
| Wrong password | Rerun with a wrong password: each press logs `action_failed` with "authentication failed" and the run continues | Close code 4009 handling |
| Missing scene | Rename `Live` in OBS and press knob 2: `action_failed` with "no scene with that name" | Request status 600 handling |
| OBS closed | Quit OBS and press: `action_failed` with "is OBS running" within 3 s; input keeps working | Connect errors, timeout |
| No endpoint | Rerun without `--obs-endpoint`: each OBS press logs `action_unavailable`; knob 3 (a multi-action with an OBS step) does not toggle mute | Unavailable handling |

#### 7b. Countdown timer: notification and sound (optional; executes, no module writes)

Needs a Mac with notifications allowed for the app that starts the service (Terminal here, OpenStreamHub in the app). Save this as `captures/hardware/timer.json`, check it with `masterhub-service validate`, then run it with `--execute`:

```json
{"schema_version":1,"name":"Timer check","bindings":[
  {"module":"m003k","control":0,"gesture":"pressed","action":{"type":"timer","seconds":10,"label":"Tea","sound":"glass"}},
  {"module":"m003k","control":1,"gesture":"pressed","action":{"type":"timer","seconds":10}}]}
```

```sh
masterhub-service run --profile captures/hardware/timer.json \
  --path-hex HEX_OF_THE_KNOBS --execute --duration-secs 60 | tee captures/hardware/service-timer.jsonl
```

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Does the notification appear? | After 10 s a notification titled "Timer finished" with the text `Tea` (knob 2: `0:10 countdown ended`). If macOS refuses it, the log has `timer_alert_warning` and nothing is shown; record which application macOS lists under Notifications. | Notification permission for the process `osascript` is attributed to (unverified) |
| Does the sound play, without delaying input? | The Glass sound plays; turning a knob during it still logs events at once | `afplay` path, alert thread |
| Cancel and expiry records | A second press within 10 s logs `timer_cancelled` (`pressed_again`); otherwise `timer_expired` followed by no `timer_alert_failed` | Countdown ownership in the worker |
| Stop mid-countdown | Ctrl-C before 10 s: `timer_cancelled` with reason `stopped`, no notification or sound afterwards | Cancel on stop |
| Label privacy | `grep -c Tea captures/hardware/service-timer.jsonl` prints 0 | Records carry `label_chars`, never the label |

#### 7c. Media keys and media stop (optional; executes, no module writes)

Needs Apple Music and, if possible, Spotify, each with a few tracks queued. It changes playback only. Save this as `captures/hardware/media.json`, check it with `masterhub-service validate`, then run it with `--execute`:

```json
{"schema_version":1,"name":"Media check","bindings":[
  {"module":"m003k","control":0,"gesture":"pressed","action":{"type":"media","command":"play_pause"}},
  {"module":"m003k","control":0,"gesture":"clockwise","action":{"type":"media","command":"next_track"}},
  {"module":"m003k","control":0,"gesture":"counter_clockwise","action":{"type":"media","command":"previous_track"}},
  {"module":"m003k","control":1,"gesture":"pressed","action":{"type":"media","command":"stop"}}]}
```

```sh
masterhub-service run --profile captures/hardware/media.json \
  --path-hex HEX_OF_THE_KNOBS --execute --duration-secs 90 | tee captures/hardware/service-media.jsonl
```

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Accessibility prompt | The first knob 1 press either works or logs `action_failed` with "did not allow posting the media key" and macOS shows an Accessibility prompt. Record which app macOS names (Terminal here, OpenStreamHub in the app). Allow it and press again | Post-event permission check and which process macOS holds responsible (unverified) |
| Play/pause, next, previous with Music | With Music frontmost and then in the background: knob 1 press toggles playback, turning right skips, turning left goes back, exactly like the keyboard's media keys | The undocumented event fields (83, 149, 150) and `CGEventPost` from `osascript` |
| The same with Spotify, and with a browser tab playing | Whichever app macOS currently routes media keys to reacts; record which one | Media-key routing |
| Stop with Music playing | Knob 2 press: Music stops (position resets); first time, macOS asks for Automation permission to control Music. The first press may log a 5 s timeout while the prompt is open | Music `stop` by process id |
| Stop with Spotify playing | Knob 2 press pauses Spotify (Automation prompt for Spotify) | Spotify `pause` by process id |
| Stop with neither running | Quit both, press knob 2: `action_executed`, neither app opens (check the Dock) | Does not start a player that is not running |
| Repeated presses while Accessibility is denied | Deny the prompt, turn the knob several times: each step logs `action_failed`, and at most one prompt appears per run | Request-once rule |
| Stop with Automation denied | Deny the prompt (or turn it off in System Settings > Privacy & Security > Automation), press knob 2: `action_failed` naming the player and Automation | `-1743` handling |

#### 7d. Profile next/previous (desktop app, dry run is enough)

Save two or three profiles in the app, one of them with a `{"type":"profile","direction":"next"}` binding on a real control. Start it as a dry run with the module's real interface and press the control.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Does the key switch the run to the next saved profile (name order)? | The Run panel shows the new profile name and the log shows `profile_switch_requested` then a restarted service; previous wraps around | Session manager `profile_action` path (unit-tested only) |
| Is it ignored with one saved profile, and does an Execute run pause for confirmation? | No restart with one profile; with Execute the session waits in `needs_confirmation` | Safety rule |
| Standalone service | The same profile under `masterhub-service run` without `--profile-switch-requests` logs `action_unavailable` and does nothing | CLI contract |

#### 7e. Importing a real `.mhz` export (no hardware needed, needs a real file)

Every importer test so far uses synthetic files. Export a profile from the official Windows MasterHUB app that has key images (`uploadImages/`), a folder, a multi-action and a dial function, and, if possible, a multi-profile `.zip`. In the app open **Manage profiles** (next to the profile switcher in the header) and choose **Import official .mhz**.

| Question | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Does the real file import without a refusal? | A new `<name> (imported)` profile and a report; images are skipped without being read | [`.mhz` format](research/masterhub-mhz-format.md) assumptions: a ZIP with one cleartext JSON, no version field |
| Is every control accounted for? | The report lists every control as imported, changed or unbound with a reason; compare with the official app's button list | Mapping table; unsupported controls are never silently dropped |
| Do shortcuts, folders and dial functions look right? | Windows Ctrl stays Control and Win becomes Command; folders become keypad folders | Shortcut mapping |
| Dry run only? | Execute is unavailable for the profile until **Allow this imported profile to execute actions** is used; copies and exports stay dry run only | [Importing MasterHUB profiles](importing-masterhub-profiles.md) |

Record any refusal text and the report's counts; keep the file out of the repository.

### Step 8. Lifecycle: replug, sleep/wake, permissions (READ)

Use a finite recording so the shell is never stuck:

```sh
scripts/hardware/record-session.sh dial 90 --label lifecycle-replug
```

| Event during the 90 s capture | Expected observable | Confirms or refutes |
| --- | --- | --- |
| Unplug and replug the base after about 20 s | Capture ends with a transport error footer and failing status (the CLI has no reconnect; the desktop app's session manager does, see the end of this step). Record exact error text. After replug, rerun `collect-inventory.sh` and compare `path_hex` before and after. | Plan gate "unplug/replug"; "paths may change". |
| Remove and reinsert one module (if hot-swappable; check the vendor guidance first, otherwise skip) | Same; which interfaces vanish and reappear, and their serials | Module reinsertion; serial-based identity. |
| Sleep the Mac (`pmset sleepnow`) with a capture running, wake after 30 s | Capture ends by deadline or error; note which. After wake: inventory and a fresh capture. | Sleep/wake gate. The CLI has no sleep/wake observer, so a failure here is expected and informative. |
| Software mode across sleep/wake (WRITE) | Run `record --software-mode`, sleep, wake; observe display and whether input resumes; whether the module needs `dial mode software` again (the SDK's two-second resume is host policy) | `DialSession` resume policy; mode persistence. |
| Fresh process after crash | Kill -9 a software-mode record, rerun neutral capture | Restore behavior (see 6a). |
| macOS permission prompt | Note whether/when an Input Monitoring prompt appears and what happens when denied | macOS access gate. |

Then repeat unplug/replug and sleep/wake with the desktop app: start a dry run, watch the Run panel (`waiting_for_device`, `suspended`, `reconnecting`), and confirm that an Execute run waits in `needs_confirmation` instead of resuming. This exercises [the session manager](masterhub-session-manager.md), which is unit-tested with fakes only. With **Experimental: send images to module displays** on, also confirm that the dial or keys are redrawn after wake.

## Evidence to save per module

For each of M0064, M003K, M131E, M4315, M005S, M032LE, keep the following under `captures/hardware/` (do not delete anything created by the scripts; they carry hashes):

1. Inventory directory (step 0) for the module alone and in the full stack.
2. Serial and firmware query output (step 1; dial only for firmware).
3. Descriptor `.hid` per interface and the `report_descriptor` line (step 2).
4. One idle capture and one capture per physical control and gesture (step 3), plus replay text.
5. Software-mode captures and restore observations (6a) for dial and keys.
6. Dial only: color photos, backlight observations, upload results and the orientation and pacing table (6b, 6c).
7. Lifecycle captures with exact error text (step 8).
8. Photographs of the physical layout with positions numbered, and the action log.

## Results table template

Copy to a new file in `captures/hardware/` and fill it in. Use `confirmed`, `refuted`, `partial` or `not run`; never leave a row without the evidence path.

| ID | Step | Module | Doc claim tested | Command | Result | Observed value | Evidence path | Notes |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| T0.1 | 0 | all | PID aliases (evidence table) | `collect-inventory.sh` | | | | |
| T0.2 | 0 | all | Interface count and usage pages | | | | | |
| T0.3 | 0 | all | USB serial present/unique | | | | | |
| T1.1 | 1 | all | Serial feature response framing | `get_module_serial_numbers` | | | | |
| T1.2 | 1 | dial | Feature queries `47/48/49/4c` | `dial info` | | | | |
| T2.1 | 2 | all | Descriptor readable; output ID `02` size | `record-session.sh <m> 3` | | | | |
| T3.1 | 3 | knobs | Index to placement; direction byte | | | | | |
| T3.2 | 3 | dial | Input before software mode; RIGHT/LEFT physical mapping | | | | | |
| T3.3 | 3 | dial | Press/long-press at 600 ms | | | | | |
| T3.4 | 3 | keys | Key index 0..14 to physical position | | | | | |
| T3.5 | 3 | faders | Order, range 0..100, jitter | | | | | |
| T3.6 | 3 | rollers | Up/Down mapping and order | | | | | |
| T3.7 | 3 | base | No input reports | | | | | |
| T5.1 | 5 | all | Open without privileges; permission prompt | | | | | |
| T5.2 | 5 | all | Idle timeout; deadline honored | | | | | |
| T5.3 | 5 | all | Concurrent open behavior | | | | | |
| T6.1 | 6a | dial | Software mode required for input | | | | | |
| T6.2 | 6a | keys | Software mode required for input | | | | | |
| T6.3 | 6a | dial/keys | Hardware mode restored after exit and after kill -9 | | | | | |
| T6.4 | 6b | dial | Backlight scale | | | | | |
| T6.5 | 6b | dial | Color channel order (observed colors for `ff0000`, `00ff00`, `0000ff`) | | | | | |
| T6.6 | 6c | dial | Upload accepted; header byte 6 zero | | | | | |
| T6.7 | 6c | dial | Orientation (rotation to use) | | | | | |
| T6.8 | 6c | dial | Pacing under back-to-back upload | | | | | |
| T6.9 | 6c | keys | Real output report length from descriptor | | | | | |
| T6.10 | 6d | dial | Clock, function and CPU faces appear; once-a-second redraws are stable | | | | | |
| T6.11 | 6e | knobs | Index 0 to 4 to LED (five indexes, three knobs) | `led sweep --module knob` | | | | |
| T6.12 | 6e | knobs | Per-LED colour and channel order | `led color` | | | | |
| T6.13 | 6e | faders | Index 0 to 2 to LED; colour and brightness | `led sweep --module slider` | | | | |
| T6.14 | 6e | base | Index 0 lights the edge strip; brightness | `led color ... --module base` | | | | |
| T6.15 | 6e | all | Unsaved writes do not survive a replug | | | | | |
| T7.1 | 7 | all | Service plan matches physical gestures | | | | | |
| T7.2 | 7a | knobs | `obs_scene` against a real OBS 28+ | | | | | |
| T7.3 | 7b | knobs | Timer notification and sound on a real Mac | | | | | |
| T7.4 | 7c | knobs | Media keys reach Music/Spotify; stop affects only running players and does not start one that is not running | | | | | |
| T7.5 | 7d | any | Profile key switches the managed run | | | | | |
| T7.6 | 7e | none | Real `.mhz` export imports; report accounts for every control | | | | | |
| T8.1 | 8 | all | Replug behavior and path change | | | | | |
| T8.2 | 8 | all | Sleep/wake behavior | | | | | |
| T8.3 | 8 | dial | Software mode across sleep/wake | | | | | |

After the session, update the evidence table's "Hardware status" column and the "Remaining gates" section only for rows marked `confirmed` with an evidence path. Treat `refuted` rows as protocol corrections with a fixture. Keep synthetic fixtures labelled synthetic.

## Scripts

All live in `scripts/hardware/`, use bash with `set -euo pipefail`, make no network calls, never write to hardware by default, and write only into a new directory under `captures/hardware/` (which is git-ignored). They refuse to reuse an existing output directory. They require `jq` and `shasum`; `xxd` is optional (descriptor extraction). They locate a prebuilt `masterhub-diag` (`target/release`, then `target/debug`, or the `MASTERHUB_DIAG` environment variable); they never run `cargo`.

| Script | Purpose |
| --- | --- |
| `collect-inventory.sh [OUT_DIR]` | `masterhub-diag list` to `list.json`, `summary.tsv`, host facts (`system.txt`, macOS version, architecture, git revision), `system_profiler`/`ioreg` USB views, `SHA256SUMS`. Prints `no devices found` and exits 0 if nothing is attached. |
| `record-session.sh [opts] <module> <seconds>` | Finds every interface of the module (both PID generations), records each concurrently with a finite deadline via `masterhub-diag record`, then replays each capture offline, extracts descriptors to `.hid`, and writes `meta.json`, `exit-codes.txt`, `SHA256SUMS`. Prints `no devices found` and exits 0 if the module is absent. |
| `lib.sh` | Shared helpers; not executable. |

`record-session.sh` options: `--label TEXT`, `--interface N`, `--max-reports N`, `--out-root DIR`. `--software-mode` (dial and keys only) writes to the device and is refused unless `--yes-write-hardware` is also given. Exit codes: 0 success or nothing to do, 2 usage error, 3 missing tool or binary, 4 failed output creation or recording.

Verified without hardware: `bash -n` on all scripts; both scripts report `no devices found` with exit 0; usage errors exit 2; `--software-mode` without acknowledgement is refused; a stub `masterhub-diag` exercised multi-interface recording, interface filtering, descriptor extraction and the metadata files. They have not run against real devices.
