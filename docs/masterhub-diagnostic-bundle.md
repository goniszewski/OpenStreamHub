# OpenStreamHub diagnostic bundle

The desktop app can write a single local JSON file that describes the attached MasterHUB hardware, the app build and recent UI activity. It exists for the first hardware-verification sessions: one file to keep alongside your notes instead of copying CLI output by hand.

**The bundle is never uploaded.** The app has no upload, telemetry or network code for it. The file stays on your machine until you decide to share it yourself.

## Where it is written

`<app data>/diagnostics/openstreamhub-diagnostics-<UTC yyyymmdd-hhmmss>.json`, for example on macOS `~/Library/Application Support/<app identifier>/diagnostics/`. The command returns the full path.

- The folder is created with mode `0700` and each file with mode `0600` (Unix).
- Files are written to a private temporary file and then linked to their final name, so a bundle is either complete or absent. An existing file is never overwritten; two bundles in the same second fail with "already exists; wait a second and try again".
- Only the newest 10 files matching exactly `openstreamhub-diagnostics-<8 digits>-<6 digits>.json` are kept; older ones are deleted. Any other file in the folder is left alone.

## Request

`create_diagnostics(request)` takes every field explicitly (there are no defaults):

| Field | Meaning |
| --- | --- |
| `redact_serials` | Redact serial numbers and device paths (see below). Recommended before sharing. |
| `include_capture` | Run a short passive input capture on each picked live interface. |
| `capture_seconds` | 1–5 when `include_capture` is true (other values are rejected); ignored otherwise. |
| `include_profile` | A profile object to embed, or `null` for none. |
| `client` | `{ activity: [...], notes: string \| null }` from the UI: recent feed/service status, records and stderr tails. Extra keys are kept as given. |

Limits: the whole request may be at most 256 KiB of JSON (larger requests are rejected); `notes` is truncated to 2 KiB; only the last 500 `activity` entries are kept. The bundle records `client_notes_truncated` and `client_activity_dropped`.

The result is `{ path, bytes, summary: { devices, descriptors_read, descriptor_errors, capture_reports, redacted } }`.

## What is collected

- **app**: package name and version, debug/release build, target OS and architecture, Tauri version, and the resolved `masterhub-service` path or the lookup error. The service binary is not executed.
- **environment**: OS, OS family, architecture and the OS version string (macOS: read from `SystemVersion.plist`; Linux: `PRETTY_NAME` from `/etc/os-release`; otherwise `null`). No program is run.
- **devices**: every enumerated HID interface with the Cooler Master vendor id `0x2516`, including the hub base and unknown products: path (or label), VID/PID, model name or `null`, whether the app supports its input, serial, manufacturer/product strings, release number, interface number, usage page and usage, and `picked_for_live` (the interface `list_live_devices` would select).
- **descriptor** per interface: the raw HID report descriptor as hex plus the output and feature report lengths (per report id, including the id byte) parsed by the SDK's `descriptor` module, or the first parse error. If the interface cannot be opened or read, the error string is recorded and the remaining interfaces are still probed. Interfaces held by a running feed or service are **not opened**; they are recorded as `skipped: "in use by feed/service"`.
- **capture** (only with `include_capture`): for each picked interface not in use, up to `capture_seconds`, at most 256 reports and at most 64 KiB of raw input per interface, using 100 ms reads. Each report has `timestamp_us` (since that interface's capture started) and raw `hex`. `ended` is `deadline`, `report_cap`, `byte_cap` or `read_error`. Interfaces are captured one after another, so a capture of several modules takes several times `capture_seconds`.
- **profile**: the profile given in the request, or `null`.
- **runtime**: ids of running feeds and services, and the `client` object.

### Device access is read-only

Probing enumerates, opens an interface, reads its report descriptor and, for a capture, reads input reports. It never sends feature or output reports, never queries feature reports, and never enables or disables software mode. Whether an interface is in use is checked right before it is opened; a feed or service started during the probe may still find the interface briefly open (opens are non-exclusive on macOS).

## Redaction (`redact_serials: true`)

- Every serial becomes `…` followed by its last four characters, or just `…` when it is shorter than five characters. This applies to device serials, every string under a `serial_number` key in the profile, and every string under a `serial_number` or `serial` key in `client`.
- Device paths can embed serials (for example Windows `\\?\HID#VID_2516&PID_…#<serial>` paths), so the real path is removed from the file entirely. Each device gets `path_label: "path-xxxxxxxx"`, the 32-bit FNV-1a hash of the raw path bytes in hex; it is stable for the same path, so entries can be matched across a bundle and across bundles from the same machine, but the path is not stored. `live:<hex>` and `service:<hex>` ids become `live:path-…` and `service:path-…`.
- Afterwards every text value (error messages, stderr tails, ids in activity) is scrubbed: occurrences of an enumerated device's path hex (either case), its path text (six characters or longer) and its serial (five characters or longer) are replaced with the label or redacted serial. Raw descriptor and report hex are never rewritten. Serials shorter than five characters are only redacted where they appear under a serial key.

- Action payloads are redacted at any depth of the included profile and client data (bindings, `multi_action` steps, keypad tiles, `dial_functions`, activity): `text.text` and `obs_scene.scene` become `<N chars>` and `open_url.url` is reduced to `scheme://host[:port]`, dropping user information, path, query and fragment. Raw snippets and URLs of 8 or more characters are also scrubbed from free text. See [Diagnostics bundle redaction](masterhub-actions.md#diagnostics-bundle-redaction).

Without redaction the bundle contains real `path_hex` values and serials.

The OBS WebSocket password is never in a bundle, with or without redaction: the app keeps it in memory only and never writes it to any file (`<app data>/obs-settings.json` holds only the on/off switch, host and port, and the bundle does not read that file).

Independently of redaction, the home directory prefix is replaced with `~` in every stored string, including the profile and client data.

## What is not collected

- No environment variable dump, user or host names, home paths, network information, or process list.
- No profiles other than the one passed explicitly; no stored profile files or app settings.
- No output from `masterhub-service` other than what the UI already passed in `client`.
- No vendor feature-report data (serial queries, firmware info): those require sending requests to the device.
- Nothing about other HID devices: only vendor id `0x2516` is enumerated.

Redaction is best effort. With redaction on, snippet text and URLs are covered by the rule above; other profile action fields (commands, file paths), notes you typed, and activity text are included as given apart from the rules above. Open the file and read it before sharing it.

## Attaching it to a report

If you choose to share a bundle:

1. Create it with **Redact serials** enabled unless the recipient needs exact serials.
2. Open the file in a text editor and check it; remove anything you do not want to share.
3. Attach it manually, for example by dragging it into a GitHub issue or an email. The app never does this for you.

The CLI equivalents for deeper investigation remain `masterhub-diag list` and `masterhub-diag record`; see [MasterHUB diagnostic tools](masterhub-diagnostics.md).
