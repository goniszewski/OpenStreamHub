# OpenStreamHub service session manager

The desktop app (`apps/open-stream-hub`) wraps every `masterhub-service run` that it starts in a **session**. A session keeps the run alive across system sleep, unplugging, and service crashes. The service has no reconnect logic of its own; see [masterhub-service](masterhub-service.md). The code is in `src-tauri/src/session.rs` (state machine and driver) and `src-tauri/src/power.rs` (sleep/wake events). None of it has been tested on hardware. The tests use a fake clock, a fake service runner, a fake inventory, and fake power events.

## Identity and ids

- A session stores the original `start_service` request: profile snapshot, `execute`, and `software_mode`. Every restart reuses that request and changes only `path_hex`.
- The device is identified by **module + serial number**. HID paths can change after a replug or wake, so the session never relies on the path alone. To find a device again, the session needs an exact serial match. For a device whose serial is null or blank, it accepts a candidate only if exactly one device of that module is present. If several are present, the session keeps waiting with the detail `ambiguous device`. A different serial on the same port never counts as the same device.
- The session id is the first process's service id (`service:<path_hex>`) and does not change while the session lasts. `service_id` in the state event names the current process, which may have a different id after a replug, or is `null` when no process is running. `service://status` and `service://record` keep using the current process's id.
- Each session runs at most one process at a time. Two sessions can never share a device identity: `start_service` refuses a device that another session already owns.
- When the app cannot identify the device from the latest inventory or from a fresh enumeration, it starts the service **unmanaged**, as it did before sessions existed. No session events are sent for that run.

## Phases

| Phase | Meaning |
| --- | --- |
| `running` | The service process is running. |
| `suspended` | The system is asleep, or the 2 s settle delay after wake has not ended yet (`next_attempt_in_ms` shows the remaining time). |
| `waiting_for_device` | The device identity is not in the inventory, or the match is ambiguous. |
| `reconnecting` | A restart is in progress or scheduled (crash backoff, shown in `next_attempt_in_ms`). |
| `needs_confirmation` | The device is back, but the session executes actions, so it waits for the user. |
| `stopped` | Final. `reason` is `user` (stop or app exit) or `service_exited` (too many crashes). |

`reason` gives the most recent interruption: `sleep`, `device_removed`, `service_exited`, `user`, or `profile_switch` (a [focused-app rule](masterhub-focus-rules.md) replaced the profile). It is `null` while the session is `running`. `detail` holds the last nonblank stderr line of a crashed service (at most 512 characters), `exit code N` or `ended by a signal` when stderr is empty, or `ambiguous device`. `profile_generation` starts at 0 and is bumped by every focused-app profile switch ([focused-app rules](masterhub-focus-rules.md#confirmation-is-tied-to-the-profile-shown)). `profile_id` and `profile_name` describe the profile of that generation: `profile_id` is `null` until a switch put a saved profile in place (the backend does not know the id of the profile a run started with), and `profile_name` is its name. The frontend resyncs its Resume dialog from them. A `session://state` event is sent whenever any field changes, including the countdown, and once as `running` right after a managed start.

## Safety rule

**Actions never resume executing on their own.** A session started with `execute: true` that is interrupted by sleep, unplugging, or a crash moves to `needs_confirmation` once its device is back. Only `resume_session(session_id, profile_generation)` restarts it, and the UI sends that command only after the user explicitly confirms. `profile_generation` names the profile the confirmation showed; if a profile switch replaced it in the meantime, the command is refused and the user is asked again. Time passing, inventory changes, and wake events cannot restart it. `resume_session` returns an error in any other phase. It restarts the session with its stored request (the original snapshot, or the saved profile a focused-app rule switched to) and its original `execute` value. Dry-run sessions (`execute: false`) restart automatically.

## Transitions

- **Sleep.** On suspend, the session stops the service through the existing SIGINT path, so the service restores hardware mode, as the official app does on suspend. The phase becomes `suspended`. After wake, the session waits `SETTLE_DELAY_MS` (2000 ms), then for the device identity to appear in the inventory. A dry run then restarts; an executing session moves to `needs_confirmation`. If a wake arrives before the pre-sleep stop has finished, the session waits for the old process to exit and then continues the same way.
- **Unplug, or a service exit the user did not request, with the device gone:** `waiting_for_device`. When the same identity reappears, a dry run restarts on the new path and an executing session moves to `needs_confirmation`. Removing the device from the inventory does not stop a running service by itself; the service exits when its reads fail. Rapid remove/add sequences cannot start a second process, because a restart happens only when no process exists.
- **Crash with the device present:** a bounded retry with backoff of 1 s, 2 s, 4 s, 8 s, 8 s (`MAX_ATTEMPTS` = 5). The next crash after that moves the session to `stopped` with reason `service_exited`. The attempt counter resets after a process has run for 30 s. For an executing session, every crash also moves it to `needs_confirmation`, and those crashes count toward the same limit. A start failure, such as a missing binary or the device being in use, counts as a crash. The app finds the binary through `OPEN_STREAM_HUB_SERVICE` or the bundled sidecar next to its executable; see [how the desktop app finds the binary](masterhub-service.md#how-the-desktop-app-finds-the-binary).
- **User stop.** `stop_session(session_id)`, or `stop_service` with either the session id or the current process id, ends the session from any phase, including `suspended`, `waiting_for_device`, and `needs_confirmation` where no process exists. Unknown ids are ignored. `stop_session` never stops a service that is not managed.
- **App exit.** All sessions end (`stopped`, reason `user`) before `Services::stop_all` stops the processes. Nothing is restarted.

## Power events

- **macOS (used):** native `NSWorkspaceWillSleepNotification` and `NSWorkspaceDidWakeNotification` observers, registered on the main thread at startup. The suspend handler waits up to `STOP_GRACE` + 1 s for the services to stop. Whether macOS actually delays sleep for that long has not been verified.
- **Ordering.** The driver handles queued sleep/wake events before any other queued input, so a sleep never waits behind queued work. Profile switches queued at the same time are handled together, and every process they interrupt is stopped in one parallel batch, as for sleep.
- **Starting while asleep.** Between a suspend and the next wake the driver spawns nothing, including a `start_service` that was queued behind the suspend. For an identified device it creates the session already `suspended` (reason `sleep`, no process); after wake and the settle delay it starts through the normal wake path, so a dry run starts and an executing session waits in `needs_confirmation`. An unidentified device cannot be managed, so its start is refused with `session::ASLEEP` ("the Mac is going to sleep; start the profile again after it wakes").
- **Other platforms (fallback):** a clock-gap detector. Once per second it compares `Instant` (which does not advance during sleep on macOS or Linux) with `SystemTime`. If the wall clock moved at least 10 s further than the monotonic clock, the detector reports a wake. It can report only `Resume`, never a suspend beforehand, so the OS will already have interrupted the service without a clean hardware-mode restore. The session stops whatever is still running and restarts cleanly after the settle delay. A forward wall-clock jump of 10 s or more, for example an NTP correction, is also read as a wake. On Windows, it is not known whether `Instant` stops during sleep.

Dependencies: `block2`, `objc2-foundation`, and `objc2-app-kit` are now direct macOS-only dependencies. The same versions were already in `Cargo.lock` as Tauri dependencies, so nothing new is downloaded. The only newly enabled features are `NSOperation` on `objc2-foundation` and `NSWorkspace` on `objc2-app-kit`.

## Comparison with the official app (unverified)

Static analysis of MasterHUB 1.0.0.113 ([research notes](research/masterhub-1.0.0.113-static-analysis.md#startup-and-macos-transport-implications)) shows that on suspend the official app sets hardware mode. On resume, it waits 2 s, re-enters software mode, queries firmware, and requests a display refresh. OpenStreamHub does the following:

- It restores hardware mode by stopping the service, which already does that on SIGINT when it enabled software mode.
- It re-enters software mode by restarting the service with the same `--software-mode` flag.
- It does not query firmware or refresh the display; the service has neither feature.

The 2 s settle delay is copied from the official app and has not been checked against real hardware. None of the following has been checked on hardware either:

- How long a device takes to re-enumerate after wake.
- Whether paths or serial numbers stay stable.
- Whether a pre-sleep SIGINT finishes before the system sleeps.
