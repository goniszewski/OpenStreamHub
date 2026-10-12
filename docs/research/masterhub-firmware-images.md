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

## Dial update flow (from strings)

The dial image keeps a runtime copy and a backup of its firmware, its configuration and its animation, each with a length and CRC (`runtime animation crc`, `backup animation len`, `save backup fw...`, `Recovery AP...`, `Recovery animation...`, `Recovery config...`). A bad runtime copy is restored from the backup. This is the layout the official updater relies on; this repository does not implement firmware updates.

## Not established

What the base's two DisplayPort connectors do (the firmware has no display code; the DisplayPorts are most likely passthrough), whether the pogo pads carry anything beyond power and USB, and any update command sequence. None of that is in these images.
