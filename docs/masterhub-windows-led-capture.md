# Windows capture: knob, fader and base lighting

Goal: see which USB reports the official MasterHUB app sends to the knob module (M003K) and the fader module (M005S) when their lighting changes. On the Mac, the base (M0064) obeyed `02 2c`/`02 2d` output reports, but the knobs and faders ignored every write, so something differs on the wire. See the [hardware results](masterhub-hardware-results.md#knob-fader-and-base-leds-11-october-2026) and the [general procedure](masterhub-macos-plan.md#windows-capture-procedure).

Reference, what the Mac sent and the base accepted (output report ID 2, 64 bytes, zero padded):

```
02 2c 00 00 01 …      base, mode 1 (Static)
02 2d 00 00 ff 00 00 …  base, colour red
```

For knobs the same commands used type `01`, for faders `02`. Compare what Windows sends with these.

## What you need

- A Windows PC with the official MasterHUB app (1.0.0.113 or newer; note the version) and the whole kit connected, ideally the same unit as on the Mac.
- [Wireshark](https://www.wireshark.org/) with **USBPcap** (it is an option in the Wireshark installer; reboot afterwards).
- A text file for a timestamped action log (a phone stopwatch is enough).

## Setup

1. Open the app once and note, per module, the firmware version shown (Settings or the module page). Knobs were `01.00.05.00` and the base `02.00.04.00` on the Mac; write down the Windows values, and whether the app offers a firmware update (do not apply one; note it).
2. Start Wireshark as administrator. In the capture list pick the **USBPcap** interface whose root hub carries the kit. If unsure: start one and unplug the kit; the interface that stops showing traffic is wrong. Use the one that shows the kit's devices (vendor ID `0x2516`).
3. Close the official app before the first capture, so enumeration is recorded too.

## Captures

Make one capture file per step, saved as `pcapng`, named as below. Stop and save after each one. Write the wall-clock time of every action in the log.

| File | What to do |
| --- | --- |
| `01-startup.pcapng` | Start capture, launch the official app, wait until the modules appear, wait 10 s idle, stop. |
| `02-knob-colour.pcapng` | App running, Lighting tab, **Single module**, pick the **knob module**, effect **Static**. Start capture. Change the colour once to pure red (`ff0000`), wait 5 s, then to pure green (`00ff00`), wait 5 s, then pure blue (`0000ff`), wait 5 s. Stop. |
| `03-knob-effect-brightness.pcapng` | Same module. Start capture. Switch the effect to **Breathing**, wait 5 s, to **Static**, wait 5 s, move brightness to about 20 %, wait 5 s, back to 100 %. Stop. |
| `04-fader-colour.pcapng` | Select the **fader module**, Static, then red, green, blue as in 02, 5 s apart. Stop. |
| `05-base-colour.pcapng` | Select the **base**, Static, red, then green, 5 s apart. This is the module that works on the Mac, so it is the control. Stop. |
| `06-whole-kit.pcapng` | Lighting tab, **Whole kit**, effect Static, colour red, wait 5 s. Stop. |
| `07-replug-knob.pcapng` | Start capture with the app running and the knob module set to red. Unplug and replug the knob module, wait 10 s. Stop. This shows whether the app re-sends lighting after a reconnect. |

If any step does not visibly change the lighting on Windows either, say so in the log: that would mean the knobs only respond in some other state.

Also note, per knob colour change: did all three knobs change together, or can the app set them separately? The app may offer per-knob colour somewhere (a knob selector in the Lighting tab). If it does, add `08-knob-per-index.pcapng`: set knob 1 red, knob 2 green, knob 3 blue, one at a time, 5 s apart.

## Reading the capture

In Wireshark use these display filters (apply one at a time):

```
usb.transfer_type == 0x01 && usb.endpoint_address.direction == 0
usbhid.setup.ReportType == 3
```

The first shows interrupt OUT transfers, which is how the Mac sent output report 2. The second shows SET_REPORT control transfers for feature reports. Also try `usb.bmRequestType == 0x21` (any host-to-interface class request) and `usb.transfer_type == 0x02` (control).

For each lighting change, find the transfer closest in time to the log entry, and look at **Leftover Capture Data** (or *HID Data*). Check:

1. **Which device** it goes to (`usb.device_address`, and the one whose serial or product is the knob module, not the base).
2. **Transfer kind**: interrupt OUT or control SET_REPORT, and for control the report ID and report type (feature or output).
3. **The bytes**. Compare with the Mac's `02 2d 01 ff RR GG BB 00`: same layout? Different command bytes or order? Any extra bytes or a different length? One transfer per LED index or a single one?
4. **Anything sent before it** on the same device after the app starts (file 01): feature reports `03 xx`, or an output report that is not a lighting command. A one-off setup report that the Mac never sends is the most likely missing piece.

## What to send back

Put these in one folder and share it (the Mac side will turn them into fixtures):

- all `pcapng` files, unmodified;
- the action log with times;
- the app version, and firmware versions of each module as the app shows them;
- a plain-text export of just the relevant transfers, from a terminal in Wireshark's folder:

```
tshark -r 02-knob-colour.pcapng -Y "usb.transfer_type == 0x01 || usb.bmRequestType == 0x21" -T fields -e frame.time_relative -e usb.device_address -e usb.endpoint_address -e usb.bmRequestType -e usbhid.setup.ReportType -e usb.capdata -e usbhid.data > 02-knob-colour.txt
```

  (repeat per file; skip it if `tshark` is not available, the `pcapng` files are enough).

Captures include the modules' USB serial numbers. They are not secret in the way a password is, but share the folder only with whoever is analysing it.
