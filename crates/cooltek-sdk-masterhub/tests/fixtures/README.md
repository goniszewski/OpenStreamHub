# Offline keypad image fixture

`keypad-72x72-baseline.jpg` is an original 72×72 black RGB image encoded as
baseline JFIF by macOS `sips` at best JPEG quality on 4 October 2026. The source
was a generated RGB PPM. Its SHA-256 is
`e19318753dcd5fd7fa5c808f6d8db8de51bc2a5249f11c220d29662cf1ebe968`
(954 bytes).

SDK tests reconstruct this file from packets with synthetic descriptors and
mutate copies to exercise format and sequence bounds. It is artwork for offline
tests, not a device capture. It does not establish firmware image acceptance,
physical key order, orientation, timing, or the actual HID output length.
