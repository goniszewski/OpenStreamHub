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

The desktop app reads feature report 0x35 of the base every two seconds (`read_base_orientation`, one read-only GET FEATURE) and, with the experimental "Follow sensor" switch on (off by default), turns the connected base layout when the state changes, taking 1 as lying down and 0 as standing. Because the byte did not move on the base tested, that switch is for trying other bases. The input report `01 2b …` is not listened for.

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
