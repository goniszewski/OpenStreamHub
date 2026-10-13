# MasterHUB firmware images (studied offline, 12 October 2026)

The official app downloads firmware for three devices from `https://dev.pms.masterctrl.coolermaster.com` (device UUIDs in the [static analysis](masterhub-1.0.0.113-static-analysis.md)). The three stable files were downloaded once for study and read offline: strings, headers, embedded USB and HID descriptors and a Thumb disassembly. Nothing was flashed, sent to a device or run. The files themselves are not kept in this repository.

| Device | Version | Size | SHA-256 |
| --- | --- | --- | --- |
| M0064 base | 2.0.4.0 | 24,268 bytes | `504cc6ec…2751e` |
| M003K knobs | 1.0.5.0 | 23,692 bytes | `75220ed3…47ef` |
| M131E dial | 1.0.4.0 | 355,802 bytes | `edab62b9…ece` |

The app maps only these three devices to firmware. No file was found for the key module, the fader module or the rollers.

## Container

A 28-byte ASCII name (`CoolerMaster M0064 M032LE`, `CoolerMaster M003K M032LE`, `CoolerMaster M131E N9H20K1`), then a little-endian length and a checksum, then the image. The base and knobs images are Arm Cortex-M0 code for a Nuvoton M032LE; their vector table starts at file offset 0x24 (initial stack pointer `0x200031b8`, reset handler at 0xf0). The dial is an Arm9 image for a Nuvoton N9H20K1 (`ldr pc` vector table, FreeRTOS-style tasks, SPI flash, an LCD init and animation backup and recovery logic).

## Embedded USB identity (base and knobs)

Each image carries its own USB descriptors near the end: vendor ID 0x2516, product ID 0x01f6 (502) for the base and 0x0202 (514) for the knobs; manufacturer `CoolTEK` and a `HID Transfer` interface string in the image (the running base reports `COOLER MASTER` / `Base, 4x6`, so the strings are replaced at run time). The HID report descriptor is the same as the running base's 211-byte descriptor: one collection (usage page 0x0C, vendor usage 0xFF00) with output report 2 and input report 1 of 63 bytes each, and 31-byte feature reports 0x03, 0x2d to 0x32, 0x34, 0x35 and 0x36. The knobs' descriptor has the same set **without 0x35**.

## The base has an orientation sensor

Only the base image contains `G sensor communicate fail` and `G sensor write data fail`: its firmware drives an accelerometer. A poll routine averages 16 samples of three axes, subtracts stored calibration offsets and, when the vertical axis reads between about 500 and 1024 counts (about 1 g, the base lying roughly level), sets one state byte: 1 when the x axis reads 80 or more and y 50 or less, 0 when x reads 50 or less and y reads below -50. Anything between keeps the old state.

- The state is returned by **feature report 0x35** (byte 2 of the report; bytes 0 and 1 are the report ID and 0).
- When it changes the base sends an **input report** `01 2b 01 00 <state>` (report ID 1, event 0x2b).
- The official app's vertical/horizontal base setting is a plausible consumer of this, and the base is printed with "VERTICAL" and "HORIZONTAL" marks. Which value means which is not in the firmware's strings; it needs one read per position on the module (see the hardware results).
- The sensor distinguishes two states, not four rotations.
- **On a real base the byte did not move.** It read 1 lying flat in landscape and stayed 1 for 50 seconds of moving the base through lying flat, a quarter turn on the desk, and standing in both turns. The routine above is therefore not a working orientation reading on this unit. The poll routine returns at once when a status byte (set by the sensor's start-up check, which logs the `G sensor communicate fail` strings) is not zero; whether the sensor started here is not known.

## How the app uses it

The desktop app does not read feature report 0x35 (it once did, behind an experimental switch, and the byte did not move on the base tested; the SDK function `base_orientation::read_orientation_state` remains). The input report `01 2b …` is not listened for.

## Why the orientation byte may not move (second pass, base 2.0.4.0)

Read from the firmware only; none of this is confirmed on hardware yet.

- **The sensor is an I²C accelerometer at address 0x18.** The start-up routine (file offset 0x1c10, called once from the entry wrapper at 0x405c) reads register 0 and expects 0x87, then writes six configuration registers (0x0f, 0x10, 0x34, 0x17, 0x1a, 0x20 and 0x21). That register map and chip ID match a Sensortek STK8BA5x-class part; the part number itself is not in the image. Samples are read from register 2 on, six bytes, with a "new data" flag in bit 0 of each low byte and a 12-bit signed value (`>> 4`).
- **Its result is kept in a RAM block at 0x200002c0.** Byte 0 is the orientation state, byte 1 the start-up status (0 = started, -1 = ID mismatch or no answer, 1 = a configuration write failed), byte 2 "the last sample read failed", byte 3 "calibration mode", byte 4 the calibration sample counter. The two `G sensor …` strings are printed through `adr` references from the start-up routine (they are not unused): a non-zero status is logged and the poll routine (0x3c04) then returns at once, so **the orientation state is never updated when the sensor does not answer or is not configured.** A base without the part fitted, or with it unreachable, would behave exactly like the one tested.
- **The poll runs from a timer interrupt** (0x1b5c, timer flag in the block at 0x40004000), and only when a global flag (0x20000010) is set and a small counter test on bytes at 0x2000000c, 0x20000220 and 0x20000221 passes. What sets those is not yet traced, so "the poll never runs" is not excluded either.
- **A third state exists:** while a calibration is running the byte is 2 (set at 0x3c9c). Calibration averages 15 samples per axis, writes the offsets to flash (Nuvoton ISP unlock 0x59, 0x16, 0x88), and expects the base lying flat (z near +1 g; the code tests 1024 ± 500 counts).
- **The input report is sent from the poll, not only on change.** After a threshold branch (state set to 1 or 0) the routine at 0x5430 builds `01 2b 01 00 <state>` in the transmit buffer unless the sample read failed; it is skipped while the previous report is still pending.
- **Two ways to settle it on a real base** (both read-only): listen for input reports with first byte 0x01 and second 0x2b while moving the base (an event proves the sensor runs and the app's read is stale), and read feature 0x35 right after replugging. If no event ever arrives, the sensor is most likely not populated or not starting on this unit.

## Command dispatch (second pass)

The routine at 0x41d4 handles a 64-byte host buffer and switches on byte 1. A jump table at 0x41fc covers 0x2a to 0x30; 0x31, 0x32, 0x33, 0x49 and 0x4a are compared separately. Command 0x2a carries a 16-bit offset, a 16-bit length, a flag byte, an end flag and a 0x0e-byte header, and copies the data into a RAM staging area then computes a checksum and flash-writes it: it is the firmware **update** transfer, so the handler as a whole is the updater and not the lighting path. The other commands in the table (0x2b to 0x33, 0x49, 0x4a) are not decoded yet, so calling them update steps is an inference from 0x2a only. This part has **not** been mapped to the lighting commands the SDK uses (0x2c to 0x2e in report 2); that remains open, and nothing here is to be sent to a device.

## Output-report commands (third pass; read from firmware, base 2.0.4.0 and knobs 1.0.5.0)

The handler at 0x41d4 (base; the knobs image has the same switch at 0x3ec0) reads byte 1 of the 64-byte report. Arguments follow byte 1, with two address bytes first (`[2]`, `[3]`): the command acts on the device itself only when both are 0; otherwise the base does not apply it here (the SDK sends other values to address a module). The commands the SDK already sends match this table, which confirms their layout:

| Command | Firmware behaviour | SDK |
| --- | --- | --- |
| 0x2a | Firmware-image transfer: 16-bit offset, 16-bit length, flags, data from byte 14, checksum and flash write (the update path) | not used, never to be |
| 0x2b | Sets a USB string (length in `[2]`, ASCII from `[3]`, checked by a validator, stored with a length and saved to flash); the serial-number write | not used |
| 0x2c | Mode: value in `[4]`, saved to flash when `[5]` is non-zero | `MODE_COMMAND` |
| 0x2d | Colour: three bytes `[4..6]`, saved when `[7]` is non-zero | `COLOR_COMMAND` |
| 0x2e | Brightness: value in `[4]`, scaled through a lookup | `BRIGHTNESS_COMMAND` |
| 0x2f | Sets the sensor's calibration-mode flag (base: starts an orientation calibration, which writes offsets to flash) | not used |
| 0x30 | Output level: takes a value 0 to 100, uses 100 minus it, scales to 0 to 255 in software floating point (constants 100.0 and 255.0 are in the image) and stores the result as the level of two LED outputs (an inverted duty cycle); which LEDs those are is not known | not used |
| 0x31 | Saves the in-memory configuration to the **backup** flash block, then reads it back (no restart) | not used |
| 0x32, 0x33 | Reload the in-memory configuration from flash: 0x32 from the primary block, 0x33 from the backup block; each sets a one-byte flag naming the source | not used |
| 0x49, 0x4a | **Enter the loader**: clear a validity word in the saved configuration (0x49 in the structured config, 0x4a in the raw primary block, then write it back), wait for the flash controller, point the vector map at the loader ROM (address 0x100000) and issue a system reset (write 0x05FA0004 to the Cortex-M reset register). After this the device runs the loader, not the application, until an update completes | not used |

Anything else is ignored. These are read from code, not exercised.

**Flash use (base, 2.0.4.0).** The commands share a small set of helpers: system-register unlock (the usual 0x59, 0x16, 0x88 key sequence), flash-controller enable and disable, erase in 512-byte pages, word read and word program. Two 2 KB blocks hold configuration: a primary block at 0x14000 and a backup block at 0x14800, each a 176-byte record (0xb0) with its length at +0x24 and a CRC at +0x20; the 0x36 feature report returns those two fields. The loader ROM is at 0x100000. Commands 0x2a, 0x2b, 0x2c, 0x2d, 0x2e (when saving), 0x2f, 0x31 and 0x49 or 0x4a write flash, and 0x49 and 0x4a restart the device into the loader, so none of them is to be sent by this repository to a real unit beyond the mode, colour and brightness commands it already uses. The knobs image carries the same switch (compares at 0x3ec2 to 0x3eec), so the table is expected to hold there too; only the base was traced to the end.

## Feature reports (third pass)

A jump table at 0x1e97 maps IDs 0x2d to 0x37 (0x37 and anything else fall through to the `no match` log). Byte 1 of a reply is a length or tag, the rest the payload:

| ID | Reply (read from code) |
| --- | --- |
| 0x2d | tag 0x0f, 4 bytes from device block +0x2c, 11 bytes from +8 (a name) |
| 0x2e | the same layout from +0x34 and a fixed 11-byte string |
| 0x2f | the same layout from +0x5c and +0x40 |
| 0x30 | four version-like bytes (one from each of four words at +0x30, +0x34, +0x38, +0x3c of the second block) and 11 bytes from +0x90 (the string set by command 0x2b, i.e. the serial) |
| 0x31 | the four version bytes only |
| 0x32 | a computed LED status byte per output |
| 0x34 | the saved mode value (the byte the 0x2c command stored) |
| 0x35 | the base's orientation byte (base only) |
| 0x36 | tag 0x10: the stored image's checksum and length (header fields at +0x20 and +0x24) of two images, 16 bytes |

All are read-only; there is no feature SET handler in these images except what the output commands cover.

## Timings and limits (third pass)

- **USB.** Both images declare one interface with an interrupt IN endpoint 0x81 and an interrupt OUT endpoint 0x02, 64 bytes each, **bInterval 3**: the host polls every 3 ms at full speed, so at most about 333 reports per second each way (about 21 kB/s of 63-byte payloads). The configuration is bus powered, 100 mA (descriptor value 0x32).
- **Orientation sensor.** Samples are averaged over 16 readings; the thresholds are about 80 and 50 counts (see above); the retry count when reading the sensor is 10.
- **Debounce, long-press and LED effect timing** were not isolated; they sit in tick-counter code that needs the dial-side or timer setup traced. Left open.

## Dial display path (third pass)

- The dial file is a 127,256-byte image followed by an **animation of 25 baseline JPEG frames, 240 × 240 pixels, 5.4 to 12 KB each** (about 228 KB in total, from file offset 0x1f14d). The default animation is therefore a stack of ordinary JPEGs, which supports the SDK's JPEG-per-frame approach and the 240 × 240 screen size.
- The runtime has three tasks (`timer_task`, `process_event_task`, `update_screen_task`), a frame counter and a content-over-buffer guard. A frame that does not fit the buffer logs `ERR: content over buffer size!!` and is dropped.
- At a 3 ms poll interval a 12 KB frame is about 190 reports of 63 bytes, so roughly 0.6 s of USB time at best; this is the lower bound for the speed-test results.

## Other modules and the updater (third pass)

- **Updater.** The container (name, checksum, length, image, plus the dial's block table) and the 0x2a transfer above match the official app's three downloads; the full command sequence is not reconstructed and this repository will not implement it.
- **Other modules.** No firmware for the key module, faders or rollers has been seen in the update service (the official app maps only the base, knobs and dial), so their descriptors are known from the hardware only.

## Debounce, saved configuration, bad images and effect curve (fourth pass; read from firmware)

- **Knob button debounce.** The knobs' pin-change interrupt (0x391c) clears the flag, waits through a busy-loop delay called with the argument 10, then re-reads the three pins. The loop runs 9,500 iterations per unit, about 1 ms per unit at a 48 MHz core clock (the clock setting was not verified), so **debounce is roughly 10 ms**. A new input report is sent only when the three sampled states differ from the previous ones. The report is `01 2a 05 00` then `03 03` and three state bytes: tag 0x2a, five payload bytes. There is no long-press logic in the firmware; a long press is the host's to measure.
- **Same wait loop on the base** (0x4024, same constant), used as a millisecond delay in the sensor and start-up code.
- **Saved configuration.** Mode, colour and brightness are written to flash only when the command's save flag is set (the `[5]` or `[7]` byte above); otherwise they change in RAM and are lost at power-off. The base also stores sensor calibration offsets in flash. A configuration block carries the magic value 0xA55AAA55. At start-up the firmware logs `detect change new config` (and keeps a backup copy), `config invalid, check backup block...`, and then either restores from the backup (`backup config valid`, `Config: refresh config from backup`) or, when the backup is also invalid, `start factory reset` and `FW: copy backup to reserve`. So a corrupted configuration falls back to a backup and then to factory defaults, not to a stuck device.
- **Bad images on the dial.** The dial checks the runtime and backup animation separately (`animation valid`, `animation invalid`, `animation null`, `backup animation invalid`), logs `recovery fails, backup animation broken` when neither is usable, and otherwise recovers the animation from the backup copy. Display frames sent from the host are separate from this stored animation; a host frame larger than the buffer is dropped with the `content over buffer size` message.
- **Update checks.** The knobs image refuses an update when the identify string (`CoolerMaster M003K M032LE`) does not match or when the CRC-32 from flash differs from the CRC in the image (`CRC32 don't match`), and updates its loader (`LD`) separately.
- **Breathing effect curve.** The knobs image holds a 75-entry brightness table that rises from 28 to 100 and back (a smooth sine-like curve with a flat top of eight entries at 100). It is the breathing effect's waveform; the step rate was not isolated, so the effect's period is unknown.

## What else the images show

- **Header fields.** After the 28-byte name: a little-endian length (24,232 for the base, 23,656 for the knobs; the dial's field reads 127,256 although its file is 355,802 bytes, so the dial file holds more than one block) and a four-byte word that is probably a checksum; the algorithm is not worked out.
- **Report handling.** The base and knobs images switch on feature report IDs 0x2d to 0x36 through a jump table; an unknown ID logs `no match feature report %d`. Report 0x2d returns four bytes at offset 0x2c of a device block and 0x2e to 0x32 return similar fixed fields (version-like data), which matches the 4-byte versions read from the rollers and knobs earlier; this was not decoded further. `SET_IDLE` is logged when the host sends that request.
- **Version strings.** `__APVER_02.00.04.00` (base), `__APVER_01.00.05.00` (knobs) and, for the dial, `__APVER_01.00.04.00` plus `__LDVER_` (a separate loader version).
- **Product strings.** The dial image carries `COOLER MASTER` and `1xDial w/ IPS, 2x2`, and checks the serial (`Invalid SN!`); the running base reports `Base, 4x6`.
- **Knobs.** Their image logs `In Hard Fault Handler`, so a crash in the module's code is reported over its debug output.
- **Dial runtime.** Tasks named `tmr_task`, `process_event_task` and `update_screen_task` with an event queue (`EventQue`, `procEvt`, `frame_cnt`) and a content-over-buffer-size guard (`ERR: content over buffer size!!`), an LCD initialisation, SPI flash identification and a jump to SRAM. A frame counter and event queue suggest the display is drawn from queued events, which fits the rate limits seen on hardware.

## Dial update flow (from strings)

The dial image keeps a runtime copy and a backup of its firmware, its configuration and its animation, each with a length and CRC (`runtime animation crc`, `backup animation len`, `save backup fw...`, `Recovery AP...`, `Recovery animation...`, `Recovery config...`). A bad runtime copy is restored from the backup. This is the layout the official updater relies on; this repository does not implement firmware updates.

## Not established

What the base's two DisplayPort connectors do (the firmware has no display code; the DisplayPorts are most likely passthrough), whether the pogo pads carry anything beyond power and USB, and any update command sequence. None of that is in these images.
