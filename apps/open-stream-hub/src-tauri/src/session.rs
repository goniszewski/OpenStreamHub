//! Sleep/wake and reconnect sessions around `masterhub-service` runs.
//!
//! A managed session wraps one `start_service` run. It remembers the original
//! request (profile snapshot, `execute`, `software_mode`) and the device
//! identity (module + serial number, never only the path, which can change
//! after a replug or wake). The session id is the first process's service id
//! and stays stable while later processes may run under other ids. It is only
//! a name, never a reservation of that path: duplicate checks use the current
//! process id and the device identity. When a new session's first service id
//! is still the name of an older session that moved to another path, the new
//! one is named `session:<uuid>` instead, so ids stay unique.
//!
//! `step` is a pure state machine (no I/O, no clock): the caller passes the
//! current time and executes the returned actions. `Driver` applies it to every
//! session and runs the actions through a `Runner` (the real one wraps
//! `service::Services`; tests use a fake).
//!
//! Safety rule: a session started with `execute = true` never resumes
//! executing on its own. After any interruption it waits in
//! `needs_confirmation` until `resume_session` (issued by the UI after an
//! explicit user confirmation). Dry-run sessions resume automatically.
//! The same holds when a focused-app rule replaces the session's profile
//! (`switch_profile`): the process restarts with the new profile, a dry run
//! at once, an executing session only after `resume_session`. Every switch
//! bumps the session's `profile_generation`; `resume_session` names the
//! generation whose profile the user confirmed and is refused once another
//! switch replaced it, so a confirmation never starts a profile the user did
//! not see.
//! A `profile` key action (next/previous) takes the same path: the service
//! only reports `profile_switch_requested`, and `Driver::profile_step`
//! resolves it against the saved profiles in a stable order and switches
//! through the focused-app path (reason `profile_action`).
//! See `docs/masterhub-session-manager.md`, `docs/masterhub-focus-rules.md`
//! and `docs/masterhub-actions.md`.

use crate::{
    display::{DisplayLaunch, DisplaySettings},
    feed::LiveDevice,
    power::PowerEvent,
    service::{self, StartRequest},
};
use cooltek_sdk_service::{Profile, ProfileDirection, ServiceModule};
use serde::Serialize;
use std::{
    collections::{BTreeSet, VecDeque},
    sync::{
        Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

pub const STATE_EVENT: &str = "session://state";
/// Wait after wake before reconnecting. The official app waits two seconds
/// after resume before re-entering software mode (static analysis of
/// MasterHUB 1.0.0.113); not verified as a hardware requirement.
pub const SETTLE_DELAY_MS: u64 = 2_000;
/// Restarts after consecutive service crashes before the session stops.
pub const MAX_ATTEMPTS: u32 = 5;
/// A service that ran this long resets the crash counter.
pub const STABLE_RUN_MS: u64 = 30_000;
/// Detail used while a serial-less device cannot be told apart.
pub const AMBIGUOUS_DEVICE: &str = "ambiguous device";
const DETAIL_CHARS: usize = 512;
/// Longest pause of an idle driver loop; timers wake it earlier.
const IDLE_WAIT: Duration = Duration::from_secs(60);
/// `start_service` error for a device that cannot be managed while the Mac
/// is going to sleep (nothing may be spawned across sleep).
pub const ASLEEP: &str = "the Mac is going to sleep; start the profile again after it wakes";
/// `start_service` error for an unidentified device when this Start itself
/// cleared a missed wake (`MISSED_WAKE_MS`): the Mac is awake, but the start
/// is still deferred for the settle delay, and an unmanaged run cannot wait.
pub const JUST_WOKE: &str = "the Mac just woke; start the profile again in a moment";
/// `start_service` error when the session driver thread is gone. The start
/// is refused rather than run unmanaged, which would ignore sleep.
pub const DRIVER_STOPPED: &str =
    "the session manager stopped; restart OpenStreamHub to start profiles";
/// Monotonic time after a `Suspend` after which a user's Start clears a sleep
/// whose wake notification never arrived. Only that user-initiated path
/// clears it: Power Nap dark wakes advance the driver clock without any
/// sleep/wake notification while timers and inventory polls keep firing, so
/// background ticks and inventory never clear `asleep` (that would restart
/// dry runs while the Mac is asleep). A user pressing Start means the Mac is
/// fully awake. Ten minutes is far beyond the pre-sleep stop (`SUSPEND_WAIT`
/// in `lib.rs`, about 10 s) and the ~30 s macOS lets a sleep observer delay
/// sleep, so a Start queued behind a slow but real sleep never trips it.
///
/// The driver clock is `CLOCK_UPTIME_RAW` on macOS, so the ten minutes count
/// awake uptime since the `Suspend`, not wall time. After a genuinely lost
/// wake every Start is deferred (identified devices get a suspended session,
/// unidentified ones are refused with `ASLEEP`) until ten minutes of uptime
/// have passed; sessions stay `suspended` meanwhile, and a second Start on a
/// device whose deferred session already exists is refused as already
/// running. The valve fires before the Start is validated, so it fires even
/// when that Start is then refused: firing only clears the asleep flag and
/// broadcasts `Resume` (settle delay first; executing sessions still wait
/// for `resume_session`).
pub const MISSED_WAKE_MS: u64 = 10 * 60_000;
/// `resume_session` error when the profile changed after the confirmation
/// was shown. The frontend matches it to ask again.
pub const PROFILE_CHANGED: &str = "the session's profile changed after the confirmation was shown; review the new profile and confirm again";
/// Profile key requests of one session closer together than this after an
/// accepted one are dropped, so a bouncing key or a held rotation switches
/// once. Requests from a process that is already stopping (the switch it
/// requested is under way) are dropped as well.
pub const PROFILE_STEP_DEBOUNCE_MS: u64 = 500;

/// Restart delay for crash attempt `attempt` (1-based): 1 s, 2 s, 4 s, 8 s, 8 s.
pub fn backoff_ms(attempt: u32) -> u64 {
    1_000 << attempt.saturating_sub(1).min(3)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Running,
    Suspended,
    WaitingForDevice,
    Reconnecting,
    NeedsConfirmation,
    Stopped,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    Sleep,
    DeviceRemoved,
    ServiceExited,
    User,
    /// A focused-app rule replaced the session's profile.
    ProfileSwitch,
    /// A `profile` key action (next/previous) replaced the session's profile.
    ProfileAction,
}

/// Payload of `session://state`.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct SessionState {
    pub session_id: String,
    /// The current process, null while none runs.
    pub service_id: Option<String>,
    pub phase: Phase,
    pub reason: Option<Reason>,
    pub attempt: u32,
    pub execute: bool,
    pub module: ServiceModule,
    pub serial_number: Option<String>,
    /// Bumped by every profile switch; `resume_session` must name it.
    pub profile_generation: u64,
    /// The saved profile the session now runs, null while it still runs the
    /// profile it was started with (whose id the backend does not know).
    pub profile_id: Option<String>,
    /// Name of the profile of `profile_generation`, so the frontend can resync
    /// the confirmation copy after a lost switch reply or a reload.
    pub profile_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_attempt_in_ms: Option<u64>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    /// The process requested by `StartService` was spawned.
    Started {
        service_id: String,
    },
    Suspend,
    Resume,
    Inventory(Vec<LiveDevice>),
    /// The current process ended (or could not be spawned).
    ServiceExited {
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    /// A scheduled timer fired; the state re-evaluates against `now_ms`.
    Tick,
    UserStop,
    UserResume,
    /// The stored request now carries another profile; the current process
    /// stops and the session restarts it under the usual safety rule.
    ProfileSwitched {
        profile_id: String,
        profile_name: String,
        /// `ProfileSwitch` (focused-app rule) or `ProfileAction` (profile key).
        reason: Reason,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum Action {
    /// SIGINT path of `Services::stop`; the service restores hardware mode.
    StopService {
        service_id: String,
    },
    /// Start the original request again on `path_hex`.
    StartService {
        path_hex: String,
        execute: bool,
    },
    Emit(SessionState),
    ScheduleTick(u64),
}

#[derive(Clone, Debug, PartialEq)]
struct Process {
    service_id: String,
    started_ms: u64,
    /// `Started` was seen; false while a requested start is in flight.
    running: bool,
    /// Stopped on purpose (sleep, user); its exit is not a crash.
    stopping: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    session_id: String,
    module: ServiceModule,
    serial_number: Option<String>,
    execute: bool,
    /// Last path the device was seen at.
    path_hex: String,
    phase: Phase,
    reason: Option<Reason>,
    attempt: u32,
    detail: Option<String>,
    service: Option<Process>,
    inventory: Vec<LiveDevice>,
    /// Between `Suspend` and `Resume`.
    asleep: bool,
    settle_until: Option<u64>,
    retry_at: Option<u64>,
    /// The user confirmed resuming an executing session.
    confirmed: bool,
    /// Number of profile switches so far.
    profile_generation: u64,
    profile_id: Option<String>,
    profile_name: String,
    last_emitted: Option<SessionState>,
}

fn blank_to_none(serial: Option<&str>) -> Option<String> {
    serial
        .map(str::trim)
        .filter(|serial| !serial.is_empty())
        .map(str::to_owned)
}

/// The last nonblank stderr line, bounded, or the exit status when empty.
fn exit_detail(exit_code: Option<i32>, stderr_tail: &str) -> String {
    stderr_tail
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(|line| line.chars().take(DETAIL_CHARS).collect())
        .unwrap_or_else(|| match exit_code {
            Some(code) => format!("exit code {code}"),
            None => "ended by a signal".into(),
        })
}

impl Session {
    /// A session named `session_id` for a service already spawned as
    /// `service_id` on `device`. `None` means the Mac is asleep and nothing
    /// was spawned: the session starts suspended and starts its process only
    /// after wake, under the usual safety rule.
    pub fn start(
        session_id: String,
        service_id: Option<String>,
        device: &LiveDevice,
        execute: bool,
        profile_name: String,
        inventory: Vec<LiveDevice>,
        now_ms: u64,
    ) -> (Self, Vec<Action>) {
        let asleep = service_id.is_none();
        let mut session = Self {
            session_id,
            module: device.module,
            serial_number: blank_to_none(device.serial_number.as_deref()),
            execute,
            path_hex: device.path_hex.clone(),
            phase: Phase::Running,
            attempt: 0,
            detail: None,
            reason: asleep.then_some(Reason::Sleep),
            service: service_id.map(|service_id| Process {
                service_id,
                started_ms: now_ms,
                running: true,
                stopping: false,
            }),
            inventory,
            asleep,
            settle_until: None,
            retry_at: None,
            confirmed: false,
            profile_generation: 0,
            profile_id: None,
            profile_name,
            last_emitted: None,
        };
        let mut actions = Vec::new();
        session.reconcile(now_ms, &mut actions);
        (session, actions)
    }

    #[cfg(test)]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The current process id, if any.
    pub fn service_id(&self) -> Option<&str> {
        self.service
            .as_ref()
            .map(|process| process.service_id.as_str())
    }

    /// Same physical device: module + nonblank serial number.
    pub fn same_identity(&self, device: &LiveDevice) -> bool {
        self.serial_number.is_some()
            && self.module == device.module
            && self.serial_number == blank_to_none(device.serial_number.as_deref())
    }

    /// A serial-less device this session will reclaim although it has no
    /// process now (e.g. it was started while asleep): same module and no
    /// serial on either side, on any path, exactly as `find_device` reclaims
    /// "the only device of that module".
    fn awaits_serial_less(&self, device: &LiveDevice) -> bool {
        !self.finished()
            && self.service.is_none()
            && self.serial_number.is_none()
            && blank_to_none(device.serial_number.as_deref()).is_none()
            && self.module == device.module
    }

    /// Ended and no process left; the driver forgets it.
    pub fn finished(&self) -> bool {
        self.phase == Phase::Stopped && self.service.is_none()
    }

    pub fn state(&self, next_attempt_in_ms: Option<u64>) -> SessionState {
        SessionState {
            session_id: self.session_id.clone(),
            service_id: self.service_id().map(str::to_owned),
            phase: self.phase,
            reason: self.reason,
            attempt: self.attempt,
            execute: self.execute,
            module: self.module,
            serial_number: self.serial_number.clone(),
            profile_generation: self.profile_generation,
            profile_id: self.profile_id.clone(),
            profile_name: self.profile_name.clone(),
            detail: self.detail.clone(),
            next_attempt_in_ms,
        }
    }

    /// Finds the device by identity: an exact serial match, or for a
    /// serial-less device the only device of that module. `Err` carries the
    /// waiting detail.
    fn find_device(&self) -> Result<String, Option<&'static str>> {
        let mut candidates = self
            .inventory
            .iter()
            .filter(|device| device.module == self.module);
        match &self.serial_number {
            Some(_) => candidates
                .find(|device| self.same_identity(device))
                .map(|device| device.path_hex.clone())
                .ok_or(None),
            None => {
                let candidates: Vec<_> = candidates.collect();
                match candidates.as_slice() {
                    [only] => Ok(only.path_hex.clone()),
                    [] => Err(None),
                    _ => Err(Some(AMBIGUOUS_DEVICE)),
                }
            }
        }
    }

    fn stop_process(&mut self, actions: &mut Vec<Action>) {
        if let Some(process) = &mut self.service
            && !process.stopping
        {
            process.stopping = true;
            actions.push(Action::StopService {
                service_id: process.service_id.clone(),
            });
        }
    }

    fn interrupt(&mut self, reason: Reason) {
        self.reason = Some(reason);
        self.detail = None;
        self.retry_at = None;
        self.confirmed = false;
    }

    fn apply(&mut self, event: Event, now_ms: u64, actions: &mut Vec<Action>) {
        let stopped = self.phase == Phase::Stopped;
        match event {
            Event::Started { service_id } => {
                if let Some(process) = &mut self.service
                    && process.service_id == service_id
                {
                    process.running = true;
                    process.started_ms = now_ms;
                }
            }
            Event::Suspend if !stopped => {
                self.asleep = true;
                self.settle_until = None;
                self.interrupt(Reason::Sleep);
            }
            Event::Resume if !stopped => {
                self.asleep = false;
                self.settle_until = Some(now_ms + SETTLE_DELAY_MS);
                self.interrupt(Reason::Sleep);
            }
            Event::Inventory(devices) => self.inventory = devices,
            Event::ServiceExited {
                exit_code,
                stderr_tail,
            } => {
                let Some(process) = self.service.take() else {
                    return;
                };
                if stopped || process.stopping {
                    return;
                }
                self.confirmed = false;
                if process.running && now_ms.saturating_sub(process.started_ms) >= STABLE_RUN_MS {
                    self.attempt = 0;
                }
                // An exit while the device is missing counts too, so a
                // flapping device backs off and hits the same limit.
                if self.find_device().is_err() {
                    self.reason = Some(Reason::DeviceRemoved);
                    self.detail = None;
                } else {
                    self.reason = Some(Reason::ServiceExited);
                    self.detail = Some(exit_detail(exit_code, &stderr_tail));
                }
                if self.attempt >= MAX_ATTEMPTS {
                    self.phase = Phase::Stopped;
                    self.retry_at = None;
                    return;
                }
                self.attempt += 1;
                self.retry_at = Some(now_ms + backoff_ms(self.attempt));
            }
            Event::UserStop if !stopped => {
                self.phase = Phase::Stopped;
                self.reason = Some(Reason::User);
                self.detail = None;
                self.asleep = false;
                self.settle_until = None;
                self.retry_at = None;
                self.stop_process(actions);
            }
            Event::UserResume if self.phase == Phase::NeedsConfirmation => {
                self.confirmed = true;
                self.retry_at = None;
            }
            // Without a live process the next start already uses the new
            // profile; nothing is interrupted.
            Event::ProfileSwitched {
                profile_id,
                profile_name,
                reason,
            } if !stopped => {
                self.profile_generation += 1;
                self.profile_id = Some(profile_id);
                self.profile_name = profile_name;
                if self
                    .service
                    .as_ref()
                    .is_some_and(|process| !process.stopping)
                {
                    self.interrupt(reason);
                    self.stop_process(actions);
                }
            }
            Event::Suspend
            | Event::Resume
            | Event::Tick
            | Event::UserStop
            | Event::UserResume
            | Event::ProfileSwitched { .. } => {}
        }
    }

    /// Derives the phase from the facts and requests what is missing.
    fn reconcile(&mut self, now_ms: u64, actions: &mut Vec<Action>) {
        let mut next = None;
        let schedule = |delay: u64, next: &mut Option<u64>, actions: &mut Vec<Action>| {
            *next = Some(delay);
            actions.push(Action::ScheduleTick(delay));
        };
        if self.phase != Phase::Stopped {
            let settling = self.settle_until.filter(|until| now_ms < *until);
            if settling.is_none() {
                self.settle_until = None;
            }
            if self.asleep || settling.is_some() {
                // Sleep restores hardware mode through the service's SIGINT
                // path; a wake seen without a prior suspend restarts cleanly.
                self.stop_process(actions);
                self.phase = Phase::Suspended;
                if let Some(until) = settling {
                    schedule(until - now_ms, &mut next, actions);
                }
            } else if let Some(process) = &self.service {
                if process.stopping {
                    // Converges once the stopped process has exited.
                } else if !process.running {
                    self.phase = Phase::Reconnecting;
                } else {
                    self.phase = Phase::Running;
                    self.reason = None;
                    self.detail = None;
                    if self.attempt > 0 {
                        let ran = now_ms.saturating_sub(process.started_ms);
                        if ran >= STABLE_RUN_MS {
                            self.attempt = 0;
                        } else {
                            actions.push(Action::ScheduleTick(STABLE_RUN_MS - ran));
                        }
                    }
                }
            } else {
                match self.find_device() {
                    Err(detail) => {
                        self.phase = Phase::WaitingForDevice;
                        if self.reason == Some(Reason::ServiceExited) {
                            self.reason = Some(Reason::DeviceRemoved);
                        }
                        self.detail = detail.map(str::to_owned);
                    }
                    Ok(path_hex) => {
                        self.path_hex = path_hex;
                        if self.detail.as_deref() == Some(AMBIGUOUS_DEVICE) {
                            self.detail = None;
                        }
                        if self.execute && !self.confirmed {
                            self.phase = Phase::NeedsConfirmation;
                        } else if let Some(at) = self.retry_at.filter(|at| now_ms < *at) {
                            self.phase = Phase::Reconnecting;
                            schedule(at - now_ms, &mut next, actions);
                        } else {
                            self.retry_at = None;
                            self.confirmed = false;
                            self.phase = Phase::Reconnecting;
                            self.service = Some(Process {
                                service_id: format!("service:{}", self.path_hex),
                                started_ms: now_ms,
                                running: false,
                                stopping: false,
                            });
                            actions.push(Action::StartService {
                                path_hex: self.path_hex.clone(),
                                execute: self.execute,
                            });
                        }
                    }
                }
            }
        }
        // Emitted on every visible change, including a new countdown.
        let state = self.state(next);
        if self.last_emitted.as_ref() != Some(&state) {
            actions.push(Action::Emit(state.clone()));
            self.last_emitted = Some(state);
        }
    }
}

/// One pure transition at `now_ms` (milliseconds of a monotonic clock).
pub fn step(mut session: Session, event: Event, now_ms: u64) -> (Session, Vec<Action>) {
    let mut actions = Vec::new();
    session.apply(event, now_ms, &mut actions);
    session.reconcile(now_ms, &mut actions);
    (session, actions)
}

/// Side effects of the driver. `start` spawns (or fails) synchronously; a
/// process's exit is reported later through `Driver::exited`.
pub trait Runner {
    /// The checks `start` runs before spawning (valid profile, supported
    /// execute mode, no process already on the path), without spawning.
    /// `Driver::start` runs them for every start, so a start while asleep
    /// fails as early as one that spawns.
    fn check(&mut self, _request: &StartRequest) -> Result<(), String> {
        Ok(())
    }
    fn start(&mut self, request: &StartRequest) -> Result<String, String>;
    /// Stops via SIGINT, then kills after the grace period.
    fn stop(&mut self, service_id: &str);
    /// Stops several processes in parallel (e.g. before sleep), so the wait
    /// is bounded by one grace period rather than their sum.
    fn stop_many(&mut self, service_ids: &[String]) {
        for service_id in service_ids {
            self.stop(service_id);
        }
    }
    /// True once app exit began; nothing may be started any more.
    fn closed(&self) -> bool {
        false
    }
    fn emit(&mut self, state: &SessionState);
    /// A fresh device inventory when the watcher has not reported the path.
    fn enumerate(&mut self) -> Option<Vec<LiveDevice>>;
    /// Every readable, valid saved profile as `(id, profile)`, in any order
    /// (`resolve_profile_step` sorts them). Read only for an accepted profile
    /// key request.
    fn saved_profiles(&mut self) -> Result<Vec<(String, Profile)>, String> {
        Ok(Vec::new())
    }
    /// The experimental display settings a session started now keeps for its
    /// whole life. Read once per user Start; off when unreadable.
    fn display_settings(&mut self) -> DisplaySettings {
        DisplaySettings::default()
    }
}

/// The saved profile a `profile` key action switches to, or `None` when
/// there is nothing to switch to (fewer than two saved profiles).
///
/// The order is stable and independent of modification times: by name
/// (ignoring case), then by exact name, then by id. The current position is
/// `current_id` (the saved profile the session last switched to, or the one
/// it was started from), otherwise, when that id is unknown or no longer
/// saved, the first saved profile equal to the running snapshot. The target
/// is never the profile named by `current_id`. A run on a profile
/// that matches none of them (an unsaved draft) goes to the first profile for
/// `next` and the last for `previous`. Both ends wrap around.
pub fn resolve_profile_step(
    mut saved: Vec<(String, Profile)>,
    current_id: Option<&str>,
    current: &Profile,
    direction: ProfileDirection,
) -> Option<(String, Profile)> {
    if saved.len() < 2 {
        return None;
    }
    saved.sort_by(|(left_id, left), (right_id, right)| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.name.cmp(&right.name))
            .then_with(|| left_id.cmp(right_id))
    });
    let position = current_id
        .and_then(|id| saved.iter().position(|(saved_id, _)| saved_id == id))
        .or_else(|| saved.iter().position(|(_, profile)| profile == current));
    let count = saved.len();
    let target = match (position, direction) {
        (Some(index), ProfileDirection::Next) => (index + 1) % count,
        (Some(index), ProfileDirection::Previous) => (index + count - 1) % count,
        (None, ProfileDirection::Next) => 0,
        (None, ProfileDirection::Previous) => count - 1,
    };
    if current_id.is_some_and(|id| saved[target].0 == id) {
        return None;
    }
    Some(saved.swap_remove(target))
}

/// The direction of a `profile_switch_requested` service record, or `None`
/// for any other record. Strict: schema 1 and a direction from the closed
/// set only; anything else is ignored rather than guessed.
pub fn profile_switch_request(record: &serde_json::Value) -> Option<ProfileDirection> {
    if record.get("type")?.as_str()? != "profile_switch_requested"
        || record.get("schema_version")?.as_u64()? != 1
    {
        return None;
    }
    serde_json::from_value(record.get("direction")?.clone()).ok()
}

/// Why `Driver::profile_step` dropped a request.
pub const STEP_ASLEEP: &str = "the Mac is asleep";
pub const STEP_NOT_RUNNING: &str = "no running session owns this service";
pub const STEP_DEBOUNCED: &str = "debounced";
pub const STEP_NO_TARGET: &str = "fewer than two saved profiles";

/// The profile a session holds for its current generation, exactly as
/// `resume_session` would restart it (`session_profile`).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HeldProfile {
    /// The saved profile it came from: the last switch's, or the one the run
    /// was started from; null for an unsaved draft.
    pub profile_id: Option<String>,
    pub profile_generation: u64,
    pub profile: Profile,
}

/// Forwards one service process's messages to the driver: its exit, and
/// strictly parsed profile key requests. Each spawn gets its own forwarder,
/// and nothing is forwarded after that spawn's exit, so a late record of a
/// stopped process can never switch the process that replaced it under the
/// same service id. The lock keeps "exited" and every forwarded request in
/// the order the driver receives them.
pub struct SpawnForwarder {
    sessions: Sender<Input>,
    exited: Mutex<bool>,
}

impl SpawnForwarder {
    pub fn new(sessions: Sender<Input>) -> Self {
        Self {
            sessions,
            exited: Mutex::new(false),
        }
    }

    pub fn forward(&self, message: &service::ServiceMessage) {
        let mut exited = self
            .exited
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *exited {
            return;
        }
        match message {
            service::ServiceMessage::Status(status)
                if status.state == service::ServiceState::Exited =>
            {
                *exited = true;
                let _ = self.sessions.send(Input::Exited {
                    service_id: status.service_id.clone(),
                    exit_code: status.exit_code.flatten(),
                    stderr_tail: status.stderr_tail.clone().unwrap_or_default(),
                });
            }
            service::ServiceMessage::Record(payload) => {
                if let Some(direction) = profile_switch_request(&payload.record) {
                    let _ = self.sessions.send(Input::ProfileStep {
                        service_id: payload.service_id.clone(),
                        direction,
                    });
                }
            }
            service::ServiceMessage::Status(_) => {}
        }
    }
}

struct Managed {
    session: Session,
    request: StartRequest,
    /// When the last profile key request of this session passed the state
    /// checks (see `profile_step`).
    last_profile_step_ms: Option<u64>,
}

/// Owns every managed session and executes their actions.
pub struct Driver<R> {
    runner: R,
    sessions: Vec<Managed>,
    inventory: Option<Vec<LiveDevice>>,
    timers: BTreeSet<(u64, String)>,
    closed: bool,
    /// While set, `StopService` actions are collected here and then stopped
    /// together through `Runner::stop_many`.
    deferred_stops: Option<Vec<String>>,
    /// Between `Suspend` and `Resume`: nothing new is spawned.
    asleep: bool,
    /// When the last `Suspend` arrived (monotonic ms).
    suspended_ms: u64,
}

impl<R: Runner> Driver<R> {
    pub fn new(runner: R) -> Self {
        Self {
            runner,
            sessions: Vec::new(),
            inventory: None,
            timers: BTreeSet::new(),
            closed: false,
            deferred_stops: None,
            asleep: false,
            suspended_ms: 0,
        }
    }

    /// Safety valve for a lost wake notification, called only from the
    /// user's Start (never from ticks or inventory, which keep running during
    /// Power Nap dark wakes): a Start `MISSED_WAKE_MS` of monotonic time after
    /// the `Suspend` means the Mac is awake. The flag is cleared through the
    /// normal wake path: the settle delay first, and an executing session
    /// still waits for `resume_session`. True when it fired. `now_ms` is
    /// awake uptime (see `MISSED_WAKE_MS`), and it fires before the Start is
    /// validated, so a refused Start still clears the flag.
    fn missed_wake(&mut self, now_ms: u64) -> bool {
        if !self.asleep || now_ms.saturating_sub(self.suspended_ms) < MISSED_WAKE_MS {
            return false;
        }
        self.power(PowerEvent::Resume, now_ms);
        true
    }

    fn is_closed(&self) -> bool {
        self.closed || self.runner.closed()
    }

    #[cfg(test)]
    pub fn runner(&mut self) -> &mut R {
        &mut self.runner
    }

    #[cfg(test)]
    pub fn session(&self, session_id: &str) -> Option<&Session> {
        self.sessions
            .iter()
            .map(|managed| &managed.session)
            .find(|session| session.session_id == session_id)
    }

    /// The session named `id` or running process `id`. A session name may
    /// equal another session's current process id (the name's path was
    /// reused after a replug), so `by_name` picks which match wins.
    fn find(&self, id: &str, by_name: bool) -> Option<usize> {
        let named = || {
            self.sessions
                .iter()
                .position(|managed| managed.session.session_id == id)
        };
        let running = || {
            self.sessions
                .iter()
                .position(|managed| managed.session.service_id() == Some(id))
        };
        if by_name {
            named().or_else(running)
        } else {
            running().or_else(named)
        }
    }

    fn lookup(&mut self, path_hex: &str) -> Option<LiveDevice> {
        let known = |devices: &Option<Vec<LiveDevice>>| {
            devices
                .iter()
                .flatten()
                .find(|device| device.path_hex == path_hex)
                .cloned()
        };
        known(&self.inventory).or_else(|| {
            let devices = self.runner.enumerate()?;
            self.inventory = Some(devices);
            known(&self.inventory)
        })
    }

    /// Starts a run and, when its device is identified, a session for it.
    /// An unidentified device runs unmanaged, exactly as before sessions.
    /// While the Mac is asleep nothing is spawned: an identified device gets a
    /// suspended session that starts after wake (an executing one only after
    /// `resume_session`); an unidentified one is refused with `ASLEEP`. The
    /// same holds for a start that clears a missed wake (`missed_wake`): its
    /// session waits for the settle delay like every other one.
    ///
    /// Returns the session id (which may be `session:<uuid>`), or the service
    /// id of an unmanaged run, so the frontend can key the run at once.
    pub fn start(&mut self, mut request: StartRequest, now_ms: u64) -> Result<String, String> {
        if self.is_closed() {
            return Err(service::SHUTTING_DOWN.into());
        }
        let woke = self.missed_wake(now_ms);
        let deferred = self.asleep || woke;
        let service_id = service::service_id(&request.path_hex)?;
        // Only a live process holds the path; a session name does not.
        if self
            .sessions
            .iter()
            .any(|managed| managed.session.service_id() == Some(service_id.as_str()))
        {
            return Err(format!("service {service_id} is already running"));
        }
        let path_hex = service_id.trim_start_matches("service:").to_owned();
        let device = self.lookup(&path_hex);
        // A serial-less device has no identity to compare, so a process-less
        // session waiting on the same module still claims it on any path.
        if let Some(device) = &device
            && let Some(other) = self.sessions.iter().find(|managed| {
                managed.session.same_identity(device) || managed.session.awaits_serial_less(device)
            })
        {
            return Err(format!(
                "session {} already runs this device",
                other.session.session_id
            ));
        }
        // The spawning path checks again inside `start`; asleep nothing
        // spawns, so this is the only check before the session exists.
        self.runner.check(&request)?;
        // Read once, here: every later process of this session (wake,
        // reconnect, crash or profile-switch restarts) keeps these display
        // settings, so changing them never alters or restarts a running
        // session. An unidentified device runs without them.
        request.display = device.as_ref().map(|device| {
            DisplayLaunch::at_start(
                self.runner.display_settings(),
                device.module,
                request.software_mode,
                &request.profile,
            )
        });
        let id = if deferred {
            if device.is_none() {
                return Err(if woke { JUST_WOKE } else { ASLEEP }.into());
            }
            service_id
        } else {
            self.runner.start(&request)?
        };
        let Some(device) = device else {
            return Ok(id);
        };
        let inventory = self.inventory.clone().unwrap_or_default();
        let session_id = if self
            .sessions
            .iter()
            .any(|managed| managed.session.session_id == id)
        {
            format!("session:{}", uuid::Uuid::new_v4())
        } else {
            id.clone()
        };
        let (session, mut actions) = Session::start(
            session_id.clone(),
            (!deferred).then_some(id),
            &device,
            request.execute,
            request.profile.name.clone(),
            inventory,
            now_ms,
        );
        // Created after the missed wake was cleared: it settles like the rest.
        let session = if woke {
            let (session, more) = step(session, Event::Resume, now_ms);
            actions.extend(more);
            session
        } else {
            session
        };
        self.sessions.push(Managed {
            session,
            request,
            last_profile_step_ms: None,
        });
        self.execute(self.sessions.len() - 1, actions, now_ms);
        Ok(session_id)
    }

    /// `stop_service` / `stop_session`: ends the owning session from any
    /// phase. Other ids are stopped as plain services unless `sessions_only`.
    /// `stop_session` matches session names first, `stop_service` processes.
    pub fn stop(&mut self, id: &str, sessions_only: bool, now_ms: u64) {
        match self.find(id, sessions_only) {
            Some(index) => self.apply(index, Event::UserStop, now_ms),
            None if !sessions_only => self.runner.stop(id),
            None => {}
        }
        self.prune();
    }

    /// `profile_generation` is the generation of the profile the user saw
    /// when confirming. A later switch refuses the resume with
    /// `PROFILE_CHANGED` and emits the current state again.
    pub fn resume(
        &mut self,
        session_id: &str,
        profile_generation: u64,
        now_ms: u64,
    ) -> Result<(), String> {
        let index = self
            .sessions
            .iter()
            .position(|managed| managed.session.session_id == session_id)
            .ok_or_else(|| format!("unknown session {session_id}"))?;
        let session = &self.sessions[index].session;
        if session.phase != Phase::NeedsConfirmation {
            return Err(format!("session {session_id} does not need confirmation"));
        }
        if session.profile_generation != profile_generation {
            let state = session
                .last_emitted
                .clone()
                .unwrap_or_else(|| session.state(None));
            self.runner.emit(&state);
            return Err(PROFILE_CHANGED.into());
        }
        self.apply(index, Event::UserResume, now_ms);
        self.prune();
        Ok(())
    }

    /// Replaces a session's profile (a focused-app rule matched) and returns
    /// the new profile generation. Restarts follow the safety rule: an
    /// executing session waits in `needs_confirmation` and never resumes with
    /// the new profile on its own.
    pub fn switch_profile(
        &mut self,
        session_id: &str,
        profile_id: String,
        profile: Profile,
        now_ms: u64,
    ) -> Result<u64, String> {
        self.replace_profile(
            session_id,
            profile_id,
            profile,
            Reason::ProfileSwitch,
            now_ms,
        )
    }

    /// The shared switch path of focused-app rules and profile keys.
    fn replace_profile(
        &mut self,
        session_id: &str,
        profile_id: String,
        profile: Profile,
        reason: Reason,
        now_ms: u64,
    ) -> Result<u64, String> {
        let index = self
            .sessions
            .iter()
            .position(|managed| managed.session.session_id == session_id)
            .ok_or_else(|| format!("unknown session {session_id}"))?;
        if self.sessions[index].session.phase == Phase::Stopped {
            return Err(format!("session {session_id} has stopped"));
        }
        let profile_name = profile.name.clone();
        self.sessions[index].request.profile = profile;
        self.sessions[index].request.profile_id = Some(profile_id.clone());
        let event = Event::ProfileSwitched {
            profile_id,
            profile_name,
            reason,
        };
        self.apply(index, event, now_ms);
        let generation = self.sessions[index].session.profile_generation;
        self.prune();
        Ok(generation)
    }

    /// A `profile` key action of the process `service_id` asked for the next
    /// or previous saved profile. Considered only while the Mac is awake and
    /// the session that owns that process is `running` with it (not stopping
    /// for an earlier switch). The debounce window starts at every request
    /// that passes those checks, before storage is read, even when it then
    /// finds no target: a flood of presses reads the saved profiles at most
    /// once per `PROFILE_STEP_DEBOUNCE_MS`.
    /// The target comes from `resolve_profile_step`, and the switch takes the
    /// focused-app path (`replace_profile`): a new profile generation, a dry
    /// run restarts at once, an executing session waits in
    /// `needs_confirmation` and is never confirmed here. `Err` names why the
    /// request was dropped; nothing changed then.
    pub fn profile_step(
        &mut self,
        service_id: &str,
        direction: ProfileDirection,
        now_ms: u64,
    ) -> Result<u64, String> {
        if self.asleep {
            return Err(STEP_ASLEEP.into());
        }
        if self.is_closed() {
            return Err(service::SHUTTING_DOWN.into());
        }
        let index = self
            .sessions
            .iter()
            .position(|managed| {
                let session = &managed.session;
                session.phase == Phase::Running
                    && session.service.as_ref().is_some_and(|process| {
                        process.service_id == service_id && process.running && !process.stopping
                    })
            })
            .ok_or(STEP_NOT_RUNNING)?;
        if self.sessions[index]
            .last_profile_step_ms
            .is_some_and(|last| now_ms.saturating_sub(last) < PROFILE_STEP_DEBOUNCE_MS)
        {
            return Err(STEP_DEBOUNCED.into());
        }
        self.sessions[index].last_profile_step_ms = Some(now_ms);
        let saved = self.runner.saved_profiles()?;
        let managed = &self.sessions[index];
        let current_id = managed
            .session
            .profile_id
            .as_deref()
            .or(managed.request.profile_id.as_deref());
        let (profile_id, profile) =
            resolve_profile_step(saved, current_id, &managed.request.profile, direction)
                .ok_or(STEP_NO_TARGET)?;
        let session_id = managed.session.session_id.clone();
        self.replace_profile(
            &session_id,
            profile_id,
            profile,
            Reason::ProfileAction,
            now_ms,
        )
    }

    /// The profile `session_id` holds, exactly as `resume_session` would
    /// restart it, for the generation the frontend is about to show. A stale
    /// generation is refused with `PROFILE_CHANGED` (and the current state is
    /// emitted again), so the dialog never lists another profile than the
    /// one a confirmation of that generation starts.
    pub fn session_profile(
        &mut self,
        session_id: &str,
        profile_generation: u64,
    ) -> Result<HeldProfile, String> {
        let managed = self
            .sessions
            .iter()
            .find(|managed| managed.session.session_id == session_id)
            .ok_or_else(|| format!("unknown session {session_id}"))?;
        if managed.session.profile_generation != profile_generation {
            let state = managed
                .session
                .last_emitted
                .clone()
                .unwrap_or_else(|| managed.session.state(None));
            self.runner.emit(&state);
            return Err(PROFILE_CHANGED.into());
        }
        Ok(HeldProfile {
            profile_id: managed.request.profile_id.clone(),
            profile_generation,
            profile: managed.request.profile.clone(),
        })
    }

    /// Several switches at once (one focus change, several sessions): every
    /// process they interrupt is stopped in one parallel batch, like sleep.
    pub fn switch_profiles(
        &mut self,
        switches: Vec<(String, String, Profile)>,
        now_ms: u64,
    ) -> Vec<Result<u64, String>> {
        self.deferred_stops = Some(Vec::new());
        let results = switches
            .into_iter()
            .map(|(session_id, profile_id, profile)| {
                self.switch_profile(&session_id, profile_id, profile, now_ms)
            })
            .collect();
        let stops = self.deferred_stops.take().unwrap_or_default();
        if !stops.is_empty() {
            self.runner.stop_many(&stops);
        }
        results
    }

    /// Never clears `asleep`: inventory polls keep running in dark wakes.
    pub fn inventory(&mut self, devices: Vec<LiveDevice>, now_ms: u64) {
        self.inventory = Some(devices.clone());
        self.broadcast(|| Event::Inventory(devices.clone()), now_ms);
    }

    /// Every process a sleep (or wake) interrupts is stopped in parallel.
    pub fn power(&mut self, event: PowerEvent, now_ms: u64) {
        self.asleep = event == PowerEvent::Suspend;
        if self.asleep {
            self.suspended_ms = now_ms;
        }
        self.deferred_stops = Some(Vec::new());
        self.broadcast(
            || match event {
                PowerEvent::Suspend => Event::Suspend,
                PowerEvent::Resume => Event::Resume,
            },
            now_ms,
        );
        let stops = self.deferred_stops.take().unwrap_or_default();
        if !stops.is_empty() {
            self.runner.stop_many(&stops);
        }
    }

    pub fn exited(
        &mut self,
        service_id: &str,
        exit_code: Option<i32>,
        stderr_tail: String,
        now_ms: u64,
    ) {
        if let Some(index) = self
            .sessions
            .iter()
            .position(|managed| managed.session.service_id() == Some(service_id))
        {
            let event = Event::ServiceExited {
                exit_code,
                stderr_tail,
            };
            self.apply(index, event, now_ms);
        }
        self.prune();
    }

    /// Fires every timer due at `now_ms`. Never clears `asleep`: timers keep
    /// firing in dark wakes.
    pub fn tick(&mut self, now_ms: u64) {
        let due: BTreeSet<String> = self
            .timers
            .iter()
            .take_while(|(at, _)| *at <= now_ms)
            .map(|(_, id)| id.clone())
            .collect();
        self.timers.retain(|(at, _)| *at > now_ms);
        for id in due {
            if let Some(index) = self
                .sessions
                .iter()
                .position(|managed| managed.session.session_id == id)
            {
                self.apply(index, Event::Tick, now_ms);
            }
        }
        self.prune();
    }

    pub fn next_due(&self) -> Option<u64> {
        self.timers.first().map(|(at, _)| *at)
    }

    /// App exit: every session ends without restarting anything. Processes
    /// are left to `Services::stop_all`, which stops them in parallel.
    pub fn shutdown(&mut self, now_ms: u64) {
        self.closed = true;
        for index in 0..self.sessions.len() {
            let session = self.sessions[index].session.clone();
            let (session, actions) = step(session, Event::UserStop, now_ms);
            self.sessions[index].session = session;
            for action in actions {
                if let Action::Emit(state) = action {
                    self.runner.emit(&state);
                }
            }
        }
        self.sessions.clear();
        self.timers.clear();
    }

    fn broadcast(&mut self, event: impl Fn() -> Event, now_ms: u64) {
        for index in 0..self.sessions.len() {
            self.apply(index, event(), now_ms);
        }
        self.prune();
    }

    fn prune(&mut self) {
        self.sessions.retain(|managed| !managed.session.finished());
        let live: BTreeSet<&str> = self
            .sessions
            .iter()
            .map(|managed| managed.session.session_id.as_str())
            .collect();
        self.timers.retain(|(_, id)| live.contains(id.as_str()));
    }

    fn apply(&mut self, index: usize, event: Event, now_ms: u64) {
        let session = self.sessions[index].session.clone();
        let (session, actions) = step(session, event, now_ms);
        self.sessions[index].session = session;
        self.execute(index, actions, now_ms);
    }

    fn execute(&mut self, index: usize, mut actions: Vec<Action>, now_ms: u64) {
        // A start recurses into `Started`; the state it replaces is emitted
        // first so events stay in order.
        actions.sort_by_key(|action| matches!(action, Action::StartService { .. }));
        for action in actions {
            match action {
                Action::Emit(state) => self.runner.emit(&state),
                Action::ScheduleTick(delay) => {
                    let id = self.sessions[index].session.session_id.clone();
                    self.timers.insert((now_ms + delay, id));
                }
                Action::StopService { service_id } => match &mut self.deferred_stops {
                    Some(stops) => stops.push(service_id),
                    None => self.runner.stop(&service_id),
                },
                Action::StartService { .. } if self.is_closed() => {
                    // App exit began: nothing was spawned, so the session
                    // ends instead of retrying.
                    self.sessions[index].session.service = None;
                    self.apply(index, Event::UserStop, now_ms);
                }
                Action::StartService { path_hex, execute } => {
                    let service_id = format!("service:{path_hex}");
                    let busy = self.sessions.iter().enumerate().any(|(other, managed)| {
                        other != index && managed.session.service_id() == Some(service_id.as_str())
                    });
                    let started = if busy {
                        Err("the device is in use by another session".to_string())
                    } else {
                        let request = StartRequest {
                            path_hex,
                            execute,
                            ..self.sessions[index].request.clone()
                        };
                        self.runner.start(&request)
                    };
                    let event = match started {
                        Ok(service_id) => Event::Started { service_id },
                        Err(error) => Event::ServiceExited {
                            exit_code: None,
                            stderr_tail: error,
                        },
                    };
                    self.apply(index, event, now_ms);
                }
            }
        }
    }
}

/// Messages for the driver thread.
pub enum Input {
    Start(StartRequest, Sender<Result<String, String>>),
    Stop {
        id: String,
        sessions_only: bool,
        done: Sender<()>,
    },
    /// Session id and the confirmed profile generation.
    Resume(String, u64, Sender<Result<(), String>>),
    /// Session id, saved profile id and profile; answered with the new
    /// profile generation.
    SwitchProfile(String, String, Box<Profile>, Sender<Result<u64, String>>),
    /// Session id and the generation the Resume dialog is about to show.
    SessionProfile(String, u64, Sender<Result<HeldProfile, String>>),
    /// A service process reported `profile_switch_requested` (a profile key).
    ProfileStep {
        service_id: String,
        direction: ProfileDirection,
    },
    Inventory(Vec<LiveDevice>),
    /// `done` is answered once the event's actions ran (e.g. the stop before
    /// sleep).
    Power(PowerEvent, Option<Sender<()>>),
    Exited {
        service_id: String,
        exit_code: Option<i32>,
        stderr_tail: String,
    },
    Shutdown(Sender<()>),
}

impl<R: Runner> Driver<R> {
    /// Handles one input; false once shut down.
    pub fn handle(&mut self, input: Input, now_ms: u64) -> bool {
        match input {
            Input::Start(request, reply) => {
                let _ = reply.send(self.start(request, now_ms));
            }
            Input::Stop {
                id,
                sessions_only,
                done,
            } => {
                self.stop(&id, sessions_only, now_ms);
                let _ = done.send(());
            }
            Input::Resume(id, generation, reply) => {
                let _ = reply.send(self.resume(&id, generation, now_ms));
            }
            Input::SwitchProfile(id, profile_id, profile, reply) => {
                self.handle_switches(vec![(id, profile_id, profile, reply)], now_ms);
            }
            Input::SessionProfile(id, generation, reply) => {
                let _ = reply.send(self.session_profile(&id, generation));
            }
            Input::ProfileStep {
                service_id,
                direction,
            } => {
                // A dropped request changes nothing; the service log already
                // shows the request itself.
                let _ = self.profile_step(&service_id, direction, now_ms);
            }
            Input::Inventory(devices) => self.inventory(devices, now_ms),
            Input::Power(event, done) => {
                self.power(event, now_ms);
                if let Some(done) = done {
                    let _ = done.send(());
                }
            }
            Input::Exited {
                service_id,
                exit_code,
                stderr_tail,
            } => self.exited(&service_id, exit_code, stderr_tail, now_ms),
            Input::Shutdown(done) => {
                self.shutdown(now_ms);
                let _ = done.send(());
                return false;
            }
        }
        true
    }

    fn handle_switches(&mut self, switches: Vec<Switch>, now_ms: u64) {
        let (requests, replies): (Vec<_>, Vec<_>) = switches
            .into_iter()
            .map(|(id, profile_id, profile, reply)| ((id, profile_id, *profile), reply))
            .unzip();
        for (result, reply) in self
            .switch_profiles(requests, now_ms)
            .into_iter()
            .zip(replies)
        {
            let _ = reply.send(result);
        }
    }
}

type Switch = (String, String, Box<Profile>, Sender<Result<u64, String>>);

/// What the driver handles next from the queued inputs.
enum Next {
    One(Input),
    Switches(Vec<Switch>),
}

/// Picks the next queued input. Power events go first, so a sleep never
/// waits behind other queued work and a dry run a switch just stopped is not
/// restarted right before sleep stops it again. A start queued behind a
/// suspend therefore runs while the driver is asleep and spawns nothing (see
/// `Driver::start`). A profile switch takes every
/// other queued switch with it, so their stops run in one parallel batch.
fn take_next(queue: &mut VecDeque<Input>) -> Option<Next> {
    let index = queue
        .iter()
        .position(|input| matches!(input, Input::Power(..)))
        .unwrap_or(0);
    match queue.remove(index)? {
        Input::SwitchProfile(id, profile_id, profile, reply) => {
            let mut switches = vec![(id, profile_id, profile, reply)];
            let mut rest = VecDeque::with_capacity(queue.len());
            for input in queue.drain(..) {
                match input {
                    Input::SwitchProfile(id, profile_id, profile, reply) => {
                        switches.push((id, profile_id, profile, reply))
                    }
                    other => rest.push_back(other),
                }
            }
            *queue = rest;
            Some(Next::Switches(switches))
        }
        input => Some(Next::One(input)),
    }
}

/// Runs the driver until shutdown, waking for inputs and due timers.
pub fn run_driver<R: Runner>(
    driver: &mut Driver<R>,
    inputs: &Receiver<Input>,
    clock: impl Fn() -> u64,
) {
    let mut queue = VecDeque::new();
    loop {
        if queue.is_empty() {
            let now = clock();
            let wait = driver.next_due().map_or(IDLE_WAIT, |due| {
                Duration::from_millis(due.saturating_sub(now))
            });
            match inputs.recv_timeout(wait) {
                Ok(input) => queue.push_back(input),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
        queue.extend(inputs.try_iter());
        match take_next(&mut queue) {
            Some(Next::One(input)) => {
                if !driver.handle(input, clock()) {
                    return;
                }
            }
            Some(Next::Switches(switches)) => driver.handle_switches(switches, clock()),
            None => {}
        }
        driver.tick(clock());
    }
}

/// Handle to the driver thread held in the app state.
pub struct Manager {
    inputs: Sender<Input>,
    receiver: Mutex<Option<Receiver<Input>>>,
    running: AtomicBool,
    /// Set once app exit began; starts are refused from then on.
    closed: AtomicBool,
}

impl Default for Manager {
    fn default() -> Self {
        let (inputs, receiver) = mpsc::channel();
        Self {
            inputs,
            receiver: Mutex::new(Some(receiver)),
            running: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }
}

/// How long a command waits for the driver; it may be inside a bounded stop.
const REPLY_WAIT: Duration = Duration::from_secs(30);

impl Manager {
    pub fn sender(&self) -> Sender<Input> {
        self.inputs.clone()
    }

    /// Starts the driver thread once.
    pub fn spawn<R: Runner + Send + 'static>(&self, runner: R) -> Result<(), String> {
        let receiver = self
            .receiver
            .lock()
            .map_err(|_| "session manager lock is unavailable".to_string())?
            .take()
            .ok_or("the session manager is already running")?;
        thread::Builder::new()
            .name("service sessions".into())
            .spawn(move || {
                let origin = Instant::now();
                let clock = || u64::try_from(origin.elapsed().as_millis()).unwrap_or(u64::MAX);
                run_driver(&mut Driver::new(runner), &receiver, clock);
            })
            .map_err(|error| format!("could not start the session manager: {error}"))?;
        self.running.store(true, Ordering::SeqCst);
        Ok(())
    }

    /// `None` only when no driver was started (or it was shut down), so the
    /// caller falls back to plain services. A driver thread that died is
    /// `Some(Err(DRIVER_STOPPED))`, never a fallback: an unmanaged start would
    /// ignore sleep.
    fn request<T>(&self, make: impl FnOnce(Sender<T>) -> Input) -> Option<Result<T, String>> {
        if !self.running.load(Ordering::SeqCst) {
            return None;
        }
        let (reply, answer) = mpsc::channel();
        if self.inputs.send(make(reply)).is_err() {
            return Some(Err(DRIVER_STOPPED.into()));
        }
        Some(
            answer
                .recv_timeout(REPLY_WAIT)
                .map_err(|error| match error {
                    RecvTimeoutError::Disconnected => DRIVER_STOPPED.into(),
                    RecvTimeoutError::Timeout => "the session manager did not respond".to_string(),
                }),
        )
    }

    /// True once `shutdown` was called.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// `None` only when no driver ever ran; refused once shut down.
    pub fn start(&self, request: StartRequest) -> Option<Result<String, String>> {
        if self.is_closed() {
            return Some(Err(service::SHUTTING_DOWN.into()));
        }
        self.request(|reply| Input::Start(request, reply))
            .map(|answer| answer.and_then(|started| started))
    }

    /// `Ok(true)` once the driver handled the stop, `Ok(false)` without one.
    /// A dead driver is `Ok(false)` too when the plain process may be stopped
    /// instead (that spawns nothing, so the fallback stays safe). A
    /// sessions-only stop has no such fallback, so it reports `DRIVER_STOPPED`.
    pub fn stop(&self, id: &str, sessions_only: bool) -> Result<bool, String> {
        match self.request(|done| Input::Stop {
            id: id.into(),
            sessions_only,
            done,
        }) {
            None => Ok(false),
            Some(Err(error)) if error == DRIVER_STOPPED && !sessions_only => Ok(false),
            Some(answer) => answer.map(|()| true),
        }
    }

    pub fn resume(&self, session_id: &str, profile_generation: u64) -> Result<(), String> {
        self.request(|reply| Input::Resume(session_id.into(), profile_generation, reply))
            .unwrap_or_else(|| Err(format!("unknown session {session_id}")))
            .and_then(|resumed| resumed)
    }

    pub fn switch_profile(
        &self,
        session_id: &str,
        profile_id: &str,
        profile: Profile,
    ) -> Result<u64, String> {
        self.request(|reply| {
            Input::SwitchProfile(
                session_id.into(),
                profile_id.into(),
                Box::new(profile),
                reply,
            )
        })
        .unwrap_or_else(|| Err(format!("unknown session {session_id}")))
        .and_then(|switched| switched)
    }

    pub fn session_profile(
        &self,
        session_id: &str,
        profile_generation: u64,
    ) -> Result<HeldProfile, String> {
        self.request(|reply| Input::SessionProfile(session_id.into(), profile_generation, reply))
            .unwrap_or_else(|| Err(format!("unknown session {session_id}")))
            .and_then(|held| held)
    }

    /// App exit: ends every session; waits at most `wait`.
    pub fn shutdown(&self, wait: Duration) {
        self.closed.store(true, Ordering::SeqCst);
        if !self.running.swap(false, Ordering::SeqCst) {
            return;
        }
        let (done, finished) = mpsc::channel();
        if self.inputs.send(Input::Shutdown(done)).is_ok() {
            let _ = finished.recv_timeout(wait);
        }
    }
}

/// Fakes shared by the session tests and the contract fixtures.
#[cfg(test)]
pub mod fake {
    use super::*;
    use crate::test_profiles;
    use cooltek_sdk_cli::capture::encode_hex;

    #[derive(Default)]
    pub struct FakeRunner {
        pub starts: Vec<StartRequest>,
        pub stops: Vec<String>,
        pub emitted: Vec<SessionState>,
        /// Processes that were started and have not exited yet.
        pub alive: BTreeSet<String>,
        pub fail_next: Option<String>,
        pub devices: Option<Vec<LiveDevice>>,
        /// Each `stop_many` call, in order.
        pub batches: Vec<Vec<String>>,
        pub closed: bool,
        /// What `saved_profiles` returns.
        pub saved: Vec<(String, Profile)>,
        /// Number of `saved_profiles` reads.
        pub saved_reads: usize,
        /// What `display_settings` returns.
        pub display: DisplaySettings,
    }

    impl Runner for FakeRunner {
        fn check(&mut self, request: &StartRequest) -> Result<(), String> {
            let id = service::service_id(&request.path_hex)?;
            if self.alive.contains(&id) {
                return Err(format!("service {id} is already running"));
            }
            service::check_request(request)
        }

        fn start(&mut self, request: &StartRequest) -> Result<String, String> {
            if let Some(error) = self.fail_next.take() {
                return Err(error);
            }
            let id = service::service_id(&request.path_hex)?;
            assert!(self.alive.insert(id.clone()), "{id} started twice");
            self.starts.push(request.clone());
            Ok(id)
        }

        fn stop(&mut self, service_id: &str) {
            self.stops.push(service_id.into());
        }

        fn stop_many(&mut self, service_ids: &[String]) {
            self.batches.push(service_ids.to_vec());
            self.stops.extend_from_slice(service_ids);
        }

        fn closed(&self) -> bool {
            self.closed
        }

        fn emit(&mut self, state: &SessionState) {
            self.emitted.push(state.clone());
        }

        fn enumerate(&mut self) -> Option<Vec<LiveDevice>> {
            self.devices.clone()
        }

        fn saved_profiles(&mut self) -> Result<Vec<(String, Profile)>, String> {
            self.saved_reads += 1;
            Ok(self.saved.clone())
        }

        fn display_settings(&mut self) -> DisplaySettings {
            self.display
        }
    }

    pub fn device(module: ServiceModule, serial: Option<&str>, path: &str) -> LiveDevice {
        LiveDevice {
            path_hex: encode_hex(path.as_bytes()),
            module,
            serial_number: serial.map(str::to_owned),
        }
    }

    pub fn request(device: &LiveDevice, execute: bool) -> StartRequest {
        StartRequest {
            profile: test_profiles::folders(),
            path_hex: device.path_hex.to_uppercase(),
            execute,
            software_mode: true,
            profile_id: None,
            display: None,
        }
    }

    /// Ends a fake process and reports it, like the supervisor's `exited`.
    pub fn exit(driver: &mut Driver<FakeRunner>, service_id: &str, stderr: &str, now: u64) {
        assert!(
            driver.runner().alive.remove(service_id),
            "{service_id} not alive"
        );
        driver.exited(service_id, Some(1), stderr.into(), now);
    }
}

#[cfg(test)]
mod tests {
    use super::fake::*;
    use super::*;

    const DIAL: ServiceModule = ServiceModule::M131E;

    fn dial(path: &str) -> LiveDevice {
        device(DIAL, Some("SER-1"), path)
    }

    /// A driver with one session running on `dial("/a")`.
    fn running(execute: bool) -> (Driver<FakeRunner>, String) {
        let mut driver = Driver::new(FakeRunner::default());
        driver.inventory(vec![dial("/a")], 0);
        let id = driver.start(request(&dial("/a"), execute), 0).unwrap();
        (driver, id)
    }

    fn last(driver: &mut Driver<FakeRunner>) -> SessionState {
        driver.runner().emitted.last().unwrap().clone()
    }

    fn phase(driver: &mut Driver<FakeRunner>) -> Phase {
        last(driver).phase
    }

    fn starts(driver: &mut Driver<FakeRunner>) -> usize {
        driver.runner().starts.len()
    }

    fn sid(path: &str) -> String {
        format!("service:{}", dial(path).path_hex)
    }

    #[test]
    fn managed_start_emits_running_with_a_stable_session_id() {
        let (mut driver, id) = running(false);
        assert_eq!(id, sid("/a"));
        assert_eq!(
            driver.runner().emitted,
            [SessionState {
                session_id: id.clone(),
                service_id: Some(id.clone()),
                phase: Phase::Running,
                reason: None,
                attempt: 0,
                execute: false,
                module: DIAL,
                serial_number: Some("SER-1".into()),
                profile_generation: 0,
                profile_id: None,
                profile_name: crate::test_profiles::folders().name,
                detail: None,
                next_attempt_in_ms: None,
            }]
        );
        // Starting the same device again is refused, even under another path.
        assert!(driver.start(request(&dial("/a"), false), 1).is_err());
        driver.inventory(vec![dial("/a"), dial("/b")], 1);
        let error = driver.start(request(&dial("/b"), false), 1).unwrap_err();
        assert!(error.contains("already runs this device"), "{error}");
        assert_eq!(starts(&mut driver), 1);
    }

    #[test]
    fn unidentified_devices_run_unmanaged() {
        let mut driver = Driver::new(FakeRunner::default());
        let id = driver.start(request(&dial("/x"), false), 0).unwrap();
        assert!(driver.runner().emitted.is_empty());
        assert!(driver.session(&id).is_none());
        // The enumerate fallback identifies devices the watcher has not seen.
        driver.runner().alive.clear();
        driver.runner().devices = Some(vec![dial("/y")]);
        let id = driver.start(request(&dial("/y"), false), 0).unwrap();
        assert_eq!(driver.session(&id).unwrap().phase(), Phase::Running);
    }

    #[test]
    fn sleep_then_wake_restarts_a_dry_run_after_the_settle_delay() {
        let (mut driver, id) = running(false);
        driver.power(PowerEvent::Suspend, 100);
        assert_eq!(driver.runner().stops, std::slice::from_ref(&id));
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason),
            (Phase::Suspended, Some(Reason::Sleep))
        );
        exit(&mut driver, &id, "", 150);
        assert_eq!(last(&mut driver).service_id, None);

        driver.power(PowerEvent::Resume, 10_000);
        let state = last(&mut driver);
        assert_eq!(state.phase, Phase::Suspended);
        assert_eq!(state.next_attempt_in_ms, Some(SETTLE_DELAY_MS));
        assert_eq!(driver.next_due(), Some(10_000 + SETTLE_DELAY_MS));
        driver.tick(11_999);
        assert_eq!(starts(&mut driver), 1, "restarted before the settle delay");
        driver.tick(12_000);
        assert_eq!(starts(&mut driver), 2);
        let restarted = &driver.runner().starts[1];
        assert!(!restarted.execute && restarted.software_mode);
        assert_eq!(restarted.profile, crate::test_profiles::folders());
        let state = last(&mut driver);
        assert_eq!((state.phase, state.reason), (Phase::Running, None));
        assert_eq!(state.session_id, id);
    }

    #[test]
    fn an_executing_session_waits_for_confirmation_after_wake() {
        let (mut driver, id) = running(true);
        assert!(
            driver.resume(&id, 0, 0).is_err(),
            "running needs no confirmation"
        );
        driver.power(PowerEvent::Suspend, 0);
        exit(&mut driver, &id, "", 10);
        driver.power(PowerEvent::Resume, 100);
        driver.tick(100 + SETTLE_DELAY_MS);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason),
            (Phase::NeedsConfirmation, Some(Reason::Sleep))
        );
        // Time alone never restarts it.
        driver.tick(1_000_000);
        driver.inventory(vec![dial("/a")], 1_000_001);
        assert_eq!(starts(&mut driver), 1);

        driver.resume(&id, 0, 1_000_002).unwrap();
        assert_eq!(starts(&mut driver), 2);
        let restarted = driver.runner().starts[1].clone();
        assert!(restarted.execute);
        assert_eq!(restarted.profile, crate::test_profiles::folders());
        assert_eq!(phase(&mut driver), Phase::Running);
        assert!(driver.resume("service:unknown", 0, 0).is_err());
    }

    #[test]
    fn unplug_and_replug_restarts_on_the_new_path() {
        let (mut driver, id) = running(false);
        // A removal alone does not stop a running service.
        driver.inventory(vec![], 10);
        assert_eq!(phase(&mut driver), Phase::Running);
        exit(
            &mut driver,
            &id,
            "masterhub-service: input read failed\n",
            20,
        );
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason, state.service_id),
            (Phase::WaitingForDevice, Some(Reason::DeviceRemoved), None)
        );
        driver.inventory(vec![dial("/b")], 5_000);
        assert_eq!(driver.runner().starts[1].path_hex, dial("/b").path_hex);
        let state = last(&mut driver);
        assert_eq!(state.phase, Phase::Running);
        assert_eq!(state.service_id, Some(sid("/b")));
        assert_eq!(state.session_id, id);
        // stop_service with the new process id ends the session.
        driver.stop(&sid("/b"), false, 6_000);
        assert_eq!(driver.runner().stops, [sid("/b")]);
        assert_eq!(last(&mut driver).phase, Phase::Stopped);
    }

    #[test]
    fn replugging_an_executing_session_needs_confirmation() {
        let (mut driver, id) = running(true);
        driver.inventory(vec![], 1);
        exit(&mut driver, &id, "", 2);
        driver.inventory(vec![dial("/b")], 3);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason),
            (Phase::NeedsConfirmation, Some(Reason::DeviceRemoved))
        );
        assert_eq!(starts(&mut driver), 1);
        driver.resume(&id, 0, 4).unwrap();
        assert_eq!(driver.runner().starts[1].path_hex, dial("/b").path_hex);
        assert!(driver.runner().starts[1].execute);
    }

    #[test]
    fn a_different_serial_does_not_resume() {
        let (mut driver, id) = running(false);
        driver.inventory(vec![], 1);
        exit(&mut driver, &id, "", 2);
        driver.inventory(vec![device(DIAL, Some("SER-2"), "/a")], 3);
        driver.inventory(vec![device(ServiceModule::M003K, Some("SER-1"), "/a")], 4);
        assert_eq!(phase(&mut driver), Phase::WaitingForDevice);
        assert_eq!(starts(&mut driver), 1);
    }

    #[test]
    fn a_serial_less_device_resumes_only_when_unambiguous() {
        let mut driver = Driver::new(FakeRunner::default());
        let first = device(DIAL, None, "/a");
        driver.inventory(vec![first.clone()], 0);
        let id = driver.start(request(&first, false), 0).unwrap();
        assert_eq!(last(&mut driver).serial_number, None);
        driver.inventory(vec![], 1);
        exit(&mut driver, &id, "", 2);
        driver.inventory(
            vec![device(DIAL, None, "/b"), device(DIAL, Some(" "), "/c")],
            3,
        );
        let state = last(&mut driver);
        assert_eq!(state.phase, Phase::WaitingForDevice);
        assert_eq!(state.detail.as_deref(), Some(AMBIGUOUS_DEVICE));
        assert_eq!(starts(&mut driver), 1);
        // The exit while the device was missing counts, so the restart
        // waits for its 1 s backoff.
        driver.inventory(vec![device(DIAL, None, "/c")], 1_002);
        let state = last(&mut driver);
        assert_eq!((state.phase, state.detail), (Phase::Running, None));
        assert_eq!(driver.runner().starts[1].path_hex, encode("/c"));
    }

    fn encode(path: &str) -> String {
        cooltek_sdk_cli::capture::encode_hex(path.as_bytes())
    }

    #[test]
    fn crashes_back_off_then_stop_after_max_attempts() {
        let (mut driver, id) = running(false);
        let mut now = 1_000;
        for (attempt, delay) in [(1, 1_000), (2, 2_000), (3, 4_000), (4, 8_000), (5, 8_000)] {
            exit(&mut driver, &id, "first\nmasterhub-service: boom\n\n", now);
            let state = last(&mut driver);
            assert_eq!(state.phase, Phase::Reconnecting);
            assert_eq!(state.reason, Some(Reason::ServiceExited));
            assert_eq!(state.attempt, attempt);
            assert_eq!(state.next_attempt_in_ms, Some(delay));
            assert_eq!(state.detail.as_deref(), Some("masterhub-service: boom"));
            driver.tick(now + delay - 1);
            assert_eq!(starts(&mut driver), attempt as usize);
            now += delay;
            driver.tick(now);
            assert_eq!(starts(&mut driver), attempt as usize + 1);
            assert_eq!(phase(&mut driver), Phase::Running);
            now += 10;
        }
        exit(&mut driver, &id, "", now);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason, state.attempt),
            (Phase::Stopped, Some(Reason::ServiceExited), 5)
        );
        assert_eq!(state.detail.as_deref(), Some("exit code 1"));
        assert!(driver.session(&id).is_none());
        assert_eq!(driver.next_due(), None);
    }

    #[test]
    fn the_attempt_counter_resets_after_a_stable_run() {
        let (mut driver, id) = running(false);
        exit(&mut driver, &id, "", 0);
        driver.tick(1_000);
        exit(&mut driver, &id, "", 1_100);
        driver.tick(3_100);
        assert_eq!(last(&mut driver).attempt, 2);
        driver.tick(3_100 + STABLE_RUN_MS - 1);
        assert_eq!(last(&mut driver).attempt, 2);
        driver.tick(3_100 + STABLE_RUN_MS);
        let state = last(&mut driver);
        assert_eq!((state.phase, state.attempt), (Phase::Running, 0));
        exit(&mut driver, &id, "", 40_000);
        let state = last(&mut driver);
        assert_eq!((state.attempt, state.next_attempt_in_ms), (1, Some(1_000)));
    }

    #[test]
    fn executing_crashes_need_confirmation_and_failed_starts_count() {
        let (mut driver, id) = running(true);
        exit(&mut driver, &id, "boom", 10);
        let state = last(&mut driver);
        assert_eq!((state.phase, state.attempt), (Phase::NeedsConfirmation, 1));
        driver.tick(100_000);
        assert_eq!(starts(&mut driver), 1);
        driver.runner().fail_next = Some("could not start".into());
        driver.resume(&id, 0, 100_001).unwrap();
        let state = last(&mut driver);
        assert_eq!((state.phase, state.attempt), (Phase::NeedsConfirmation, 2));
        assert_eq!(state.detail.as_deref(), Some("could not start"));
    }

    #[test]
    fn inventory_flaps_never_start_two_services() {
        let (mut driver, id) = running(false);
        driver.inventory(vec![], 1);
        exit(&mut driver, &id, "", 2);
        for now in 3..20 {
            let devices = if now % 2 == 0 {
                vec![]
            } else {
                vec![dial("/a")]
            };
            driver.inventory(devices, now);
        }
        // The exit counted as a crash: nothing restarts before its backoff.
        assert_eq!(starts(&mut driver), 1);
        driver.inventory(vec![dial("/a")], 21);
        driver.tick(1_002);
        // The FakeRunner asserts that no id is started while still alive.
        assert_eq!(starts(&mut driver), 2);
        assert_eq!(driver.runner().alive.len(), 1);
        assert_eq!(phase(&mut driver), Phase::Running);
    }

    #[test]
    fn wake_before_the_stop_finished_still_converges() {
        let (mut driver, id) = running(false);
        driver.power(PowerEvent::Suspend, 0);
        driver.power(PowerEvent::Resume, 10);
        driver.tick(10 + SETTLE_DELAY_MS);
        // The old process is still stopping: nothing new starts yet.
        assert_eq!(starts(&mut driver), 1);
        assert_eq!(driver.runner().stops, std::slice::from_ref(&id));
        exit(&mut driver, &id, "", 5_000);
        assert_eq!(starts(&mut driver), 2);
        assert_eq!(phase(&mut driver), Phase::Running);
    }

    #[test]
    fn a_wake_without_suspend_restarts_a_running_service() {
        // The clock-gap fallback reports only `Resume`.
        let (mut driver, id) = running(false);
        driver.power(PowerEvent::Resume, 50);
        assert_eq!(driver.runner().stops, std::slice::from_ref(&id));
        exit(&mut driver, &id, "", 60);
        driver.tick(50 + SETTLE_DELAY_MS);
        assert_eq!(starts(&mut driver), 2);
    }

    #[test]
    fn the_device_missing_after_wake_waits_for_it() {
        let (mut driver, id) = running(false);
        driver.power(PowerEvent::Suspend, 0);
        exit(&mut driver, &id, "", 1);
        driver.inventory(vec![], 2);
        driver.power(PowerEvent::Resume, 3);
        driver.tick(3 + SETTLE_DELAY_MS);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason),
            (Phase::WaitingForDevice, Some(Reason::Sleep))
        );
        driver.inventory(vec![dial("/z")], 9_000);
        assert_eq!(phase(&mut driver), Phase::Running);
    }

    /// Drives a fresh session into `target` and returns it with its id.
    fn in_phase(target: Phase, execute: bool) -> (Driver<FakeRunner>, String) {
        let (mut driver, id) = running(execute);
        match target {
            Phase::Running => {}
            Phase::Suspended => driver.power(PowerEvent::Suspend, 1),
            Phase::WaitingForDevice => {
                driver.inventory(vec![], 1);
                exit(&mut driver, &id, "", 2);
            }
            Phase::Reconnecting => exit(&mut driver, &id, "", 2),
            Phase::NeedsConfirmation => exit(&mut driver, &id, "", 2),
            Phase::Stopped => unreachable!(),
        }
        assert_eq!(phase(&mut driver), target);
        (driver, id)
    }

    #[test]
    fn a_user_stop_ends_the_session_from_every_phase() {
        for (target, execute) in [
            (Phase::Running, false),
            (Phase::Suspended, false),
            (Phase::WaitingForDevice, false),
            (Phase::Reconnecting, false),
            (Phase::NeedsConfirmation, true),
        ] {
            let (mut driver, id) = in_phase(target, execute);
            let alive = driver.runner().alive.iter().next().cloned();
            driver.stop(&id, true, 3);
            let state = last(&mut driver);
            assert_eq!(
                (state.phase, state.reason),
                (Phase::Stopped, Some(Reason::User)),
                "{target:?}"
            );
            if let Some(alive) = alive {
                if target != Phase::Suspended {
                    assert_eq!(driver.runner().stops, std::slice::from_ref(&alive));
                }
                exit(&mut driver, &alive, "", 4);
                assert_eq!(last(&mut driver).service_id, None);
            }
            assert!(driver.session(&id).is_none(), "{target:?}");
            // Nothing restarts afterwards.
            driver.inventory(vec![dial("/a")], 5);
            driver.power(PowerEvent::Resume, 6);
            driver.tick(1_000_000);
            assert_eq!(starts(&mut driver), 1, "{target:?}");
            assert!(driver.resume(&id, 0, 7).is_err());
            driver.stop(&id, true, 8);
            driver.stop("service:unknown", true, 8);
        }
    }

    #[test]
    fn stop_session_never_stops_unmanaged_services_but_stop_service_does() {
        let mut driver = Driver::new(FakeRunner::default());
        driver.stop("service:41", true, 0);
        assert!(driver.runner().stops.is_empty());
        driver.stop("service:41", false, 0);
        assert_eq!(driver.runner().stops, ["service:41"]);
    }

    #[test]
    fn app_exit_stops_every_session_without_restarting() {
        let (mut driver, id) = running(false);
        driver.handle(
            Input::Inventory(vec![
                dial("/a"),
                device(ServiceModule::M003K, Some("K"), "/k"),
            ]),
            1,
        );
        driver
            .start(
                request(&device(ServiceModule::M003K, Some("K"), "/k"), true),
                1,
            )
            .unwrap();
        let (done, finished) = mpsc::channel();
        assert!(!driver.handle(Input::Shutdown(done), 2));
        assert!(finished.try_recv().is_ok());
        let stopped: Vec<_> = driver
            .runner()
            .emitted
            .iter()
            .filter(|state| state.phase == Phase::Stopped)
            .map(|state| (state.session_id.clone(), state.reason))
            .collect();
        assert_eq!(stopped.len(), 2);
        assert!(stopped.contains(&(id.clone(), Some(Reason::User))));
        // Processes are left to `Services::stop_all`.
        assert!(driver.runner().stops.is_empty());
        exit(&mut driver, &id, "", 3);
        driver.power(PowerEvent::Resume, 4);
        driver.tick(1_000_000);
        assert_eq!(starts(&mut driver), 2);
        assert!(driver.start(request(&dial("/a"), false), 5).is_err());
    }

    #[test]
    fn the_pure_step_never_requests_a_second_process() {
        let (session, _) = Session::start(
            sid("/a"),
            Some(sid("/a")),
            &dial("/a"),
            false,
            "Folders".into(),
            vec![dial("/a")],
            0,
        );
        let events = [
            Event::ServiceExited {
                exit_code: Some(1),
                stderr_tail: String::new(),
            },
            Event::Tick,
            Event::Inventory(vec![dial("/b")]),
            Event::Inventory(vec![dial("/a")]),
            Event::Resume,
            Event::Tick,
            Event::Suspend,
            Event::Resume,
            Event::Inventory(vec![]),
            Event::Inventory(vec![dial("/a")]),
        ];
        let mut session = session;
        let mut alive = 1;
        let mut now = 0;
        for event in events {
            now += 1_500;
            if matches!(event, Event::ServiceExited { .. }) {
                alive -= 1;
            }
            let (next, actions) = step(session, event, now);
            session = next;
            for action in actions {
                if matches!(action, Action::StartService { .. }) {
                    alive += 1;
                    let service_id = session_service(&session);
                    let (next, _) = step(session, Event::Started { service_id }, now);
                    session = next;
                }
                assert!(alive <= 1, "two processes requested");
            }
        }
    }

    fn session_service(session: &Session) -> String {
        session.service_id().unwrap().to_owned()
    }

    #[test]
    fn backoff_schedule() {
        let delays: Vec<_> = (1..=MAX_ATTEMPTS).map(backoff_ms).collect();
        assert_eq!(delays, [1_000, 2_000, 4_000, 8_000, 8_000]);
    }

    #[test]
    fn the_driver_thread_answers_commands_and_fires_timers() {
        use std::sync::{Arc, Mutex as StdMutex};
        struct Shared(Arc<StdMutex<FakeRunner>>);
        impl Runner for Shared {
            fn start(&mut self, request: &StartRequest) -> Result<String, String> {
                self.0.lock().unwrap().start(request)
            }
            fn stop(&mut self, service_id: &str) {
                self.0.lock().unwrap().stop(service_id)
            }
            fn emit(&mut self, state: &SessionState) {
                self.0.lock().unwrap().emit(state)
            }
            fn enumerate(&mut self) -> Option<Vec<LiveDevice>> {
                Some(vec![dial("/a")])
            }
        }
        let fake = Arc::new(StdMutex::new(FakeRunner::default()));
        let manager = Manager::default();
        assert!(
            manager
                .stop("service:41", false)
                .is_ok_and(|handled| !handled)
        );
        manager.spawn(Shared(Arc::clone(&fake))).unwrap();
        assert!(manager.spawn(Shared(Arc::clone(&fake))).is_err());
        let id = manager.start(request(&dial("/a"), false)).unwrap().unwrap();
        // A crash schedules a 1 s restart that the thread fires on its own.
        fake.lock().unwrap().alive.clear();
        manager
            .sender()
            .send(Input::Exited {
                service_id: id.clone(),
                exit_code: Some(1),
                stderr_tail: String::new(),
            })
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while fake.lock().unwrap().starts.len() < 2 {
            assert!(Instant::now() < deadline, "the restart timer never fired");
            thread::sleep(Duration::from_millis(20));
        }
        assert!(manager.resume(&id, 0).is_err());
        assert!(manager.stop(&id, true).unwrap());
        manager.shutdown(Duration::from_secs(1));
        // Refused rather than falling back to an unmanaged run.
        assert!(matches!(
            manager.start(request(&dial("/a"), false)),
            Some(Err(error)) if error == service::SHUTTING_DOWN
        ));
        let idle = Manager::default();
        assert!(idle.start(request(&dial("/a"), false)).is_none());
        idle.shutdown(Duration::from_millis(10));
        assert!(
            idle.start(request(&dial("/a"), false))
                .is_some_and(|r| r.is_err())
        );
    }

    #[test]
    fn a_replugged_session_no_longer_reserves_its_original_path() {
        let (mut driver, id) = running(false);
        driver.inventory(vec![], 1);
        exit(&mut driver, &id, "", 2);
        driver.inventory(vec![dial("/b")], 5_000);
        assert_eq!(last(&mut driver).service_id, Some(sid("/b")));
        // Another device now sits at the old path; it can be started.
        let other = device(DIAL, Some("SER-2"), "/a");
        driver.inventory(vec![dial("/b"), other.clone()], 5_001);
        let started = driver.start(request(&other, false), 5_002).unwrap();
        let state = last(&mut driver);
        assert_eq!(state.service_id, Some(sid("/a")));
        assert_eq!(state.serial_number.as_deref(), Some("SER-2"));
        // Its name cannot be the old session's, which is still in use; the
        // start returns that name so the frontend keys the run by it.
        assert!(state.session_id.starts_with("session:"), "{state:?}");
        assert_ne!(state.session_id, id);
        assert_eq!(started, state.session_id);
        let second = state.session_id.clone();
        // stop_service with the old path's id stops the process running there.
        driver.stop(&sid("/a"), false, 5_003);
        assert_eq!(driver.runner().stops, [sid("/a")]);
        assert_eq!(last(&mut driver).session_id, second);
        assert_eq!(
            driver.session(&id).unwrap().phase(),
            Phase::Running,
            "the moved session is untouched"
        );
        // The first session is still reachable by its name.
        driver.stop(&id, true, 5_004);
        assert_eq!(driver.runner().stops, [sid("/a"), sid("/b")]);
    }

    #[test]
    fn crashes_while_the_device_is_missing_back_off_and_stop() {
        let (mut driver, id) = running(false);
        let mut now = 100;
        for attempt in 1..=MAX_ATTEMPTS {
            driver.inventory(vec![], now);
            exit(&mut driver, &id, "", now + 1);
            let state = last(&mut driver);
            assert_eq!(
                (state.phase, state.reason, state.attempt),
                (
                    Phase::WaitingForDevice,
                    Some(Reason::DeviceRemoved),
                    attempt
                )
            );
            // A replug before the backoff waits for it.
            driver.inventory(vec![dial("/a")], now + 2);
            let state = last(&mut driver);
            assert_eq!(state.phase, Phase::Reconnecting);
            assert_eq!(state.next_attempt_in_ms, Some(backoff_ms(attempt) - 1));
            assert_eq!(starts(&mut driver), attempt as usize);
            now += 1 + backoff_ms(attempt);
            driver.tick(now);
            assert_eq!(starts(&mut driver), attempt as usize + 1);
            now += 10;
        }
        driver.inventory(vec![], now);
        exit(&mut driver, &id, "", now + 1);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason, state.attempt),
            (Phase::Stopped, Some(Reason::DeviceRemoved), MAX_ATTEMPTS)
        );
        assert!(driver.session(&id).is_none());
    }

    #[test]
    fn sleep_stops_every_session_in_one_parallel_batch() {
        let (mut driver, first) = running(false);
        let keys = device(ServiceModule::M003K, Some("K"), "/k");
        driver.inventory(vec![dial("/a"), keys.clone()], 1);
        let second = driver.start(request(&keys, false), 1).unwrap();
        driver.power(PowerEvent::Suspend, 2);
        assert_eq!(driver.runner().batches, [vec![first, second]]);
        // A wake with nothing running stops nothing.
        driver.power(PowerEvent::Resume, 3);
        assert_eq!(driver.runner().batches.len(), 1);
    }

    #[test]
    fn nothing_restarts_once_the_runner_is_closed() {
        let (mut driver, id) = running(false);
        exit(&mut driver, &id, "", 10);
        driver.runner().closed = true;
        driver.tick(10 + backoff_ms(1));
        assert_eq!(starts(&mut driver), 1);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason, state.service_id),
            (Phase::Stopped, Some(Reason::User), None)
        );
        assert!(driver.session(&id).is_none());
        let error = driver.start(request(&dial("/a"), false), 20).unwrap_err();
        assert_eq!(error, service::SHUTTING_DOWN);
    }

    fn renamed(name: &str) -> Profile {
        let mut profile = crate::test_profiles::folders();
        profile.name = name.into();
        profile
    }

    #[test]
    fn a_profile_switch_restarts_a_dry_run_with_the_new_profile() {
        let (mut driver, id) = running(false);
        driver
            .switch_profile(&id, "Browser".into(), renamed("Browser"), 100)
            .unwrap();
        assert_eq!(driver.runner().stops, std::slice::from_ref(&id));
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason),
            (Phase::Running, Some(Reason::ProfileSwitch))
        );
        // A second switch while stopping only replaces the profile again.
        driver
            .switch_profile(&id, "Editor".into(), renamed("Editor"), 110)
            .unwrap();
        assert_eq!(driver.runner().stops.len(), 1);
        exit(&mut driver, &id, "", 150);
        assert_eq!(starts(&mut driver), 2);
        let restarted = driver.runner().starts[1].clone();
        assert!(!restarted.execute);
        assert!(restarted.software_mode);
        assert_eq!(restarted.profile.name, "Editor");
        let state = last(&mut driver);
        assert_eq!((state.phase, state.reason), (Phase::Running, None));
    }

    #[test]
    fn a_profile_switch_never_resumes_an_executing_session_on_its_own() {
        let (mut driver, id) = running(true);
        driver
            .switch_profile(&id, "Browser".into(), renamed("Browser"), 100)
            .unwrap();
        exit(&mut driver, &id, "", 150);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason, state.service_id),
            (Phase::NeedsConfirmation, Some(Reason::ProfileSwitch), None)
        );
        driver.tick(1_000_000);
        driver.inventory(vec![dial("/a")], 1_000_001);
        assert_eq!(state.profile_generation, 1);
        // Another switch while paused starts nothing either.
        let generation = driver
            .switch_profile(&id, "Editor".into(), renamed("Editor"), 1_000_002)
            .unwrap();
        assert_eq!(generation, 2);
        assert_eq!(last(&mut driver).profile_generation, 2);
        assert_eq!(starts(&mut driver), 1);
        assert_eq!(phase(&mut driver), Phase::NeedsConfirmation);

        // A confirmation of the profile shown before the second switch is
        // refused, and the current state is emitted again.
        let emitted = driver.runner().emitted.len();
        let error = driver.resume(&id, 1, 1_000_003).unwrap_err();
        assert_eq!(error, PROFILE_CHANGED);
        assert_eq!(starts(&mut driver), 1);
        assert_eq!(driver.runner().emitted.len(), emitted + 1);
        let refreshed = last(&mut driver);
        assert_eq!(
            (refreshed.phase, refreshed.profile_generation),
            (Phase::NeedsConfirmation, 2)
        );

        driver.resume(&id, 2, 1_000_004).unwrap();
        let restarted = driver.runner().starts[1].clone();
        assert!(restarted.execute);
        assert_eq!(restarted.profile.name, "Editor");
        assert_eq!(phase(&mut driver), Phase::Running);
    }

    #[test]
    fn switches_for_several_sessions_stop_in_one_parallel_batch() {
        let (mut driver, first) = running(false);
        let keys = device(ServiceModule::M003K, Some("K"), "/k");
        driver.inventory(vec![dial("/a"), keys.clone()], 1);
        let second = driver.start(request(&keys, true), 1).unwrap();
        let results = driver.switch_profiles(
            vec![
                (first.clone(), "Browser".into(), renamed("Browser")),
                (second.clone(), "Browser".into(), renamed("Browser")),
                (
                    "service:unknown".into(),
                    "Browser".into(),
                    renamed("Browser"),
                ),
            ],
            2,
        );
        assert_eq!(results[0], Ok(1));
        assert_eq!(results[1], Ok(1));
        assert!(results[2].is_err());
        assert_eq!(driver.runner().batches, [vec![first, second]]);
        assert_eq!(driver.runner().stops.len(), 2);
    }

    fn switch_input(id: &str) -> (Input, Receiver<Result<u64, String>>) {
        let (reply, answer) = mpsc::channel();
        let input = Input::SwitchProfile(
            id.into(),
            "Browser".into(),
            Box::new(renamed("Browser")),
            reply,
        );
        (input, answer)
    }

    #[test]
    fn queued_power_events_go_first_and_queued_switches_batch() {
        let (first, _a) = switch_input("s1");
        let (second, _b) = switch_input("s2");
        let mut queue = VecDeque::from([
            Input::Exited {
                service_id: "service:41".into(),
                exit_code: Some(0),
                stderr_tail: String::new(),
            },
            first,
            Input::Inventory(vec![]),
            Input::Power(PowerEvent::Suspend, None),
            second,
            Input::Power(PowerEvent::Resume, None),
        ]);
        assert!(matches!(
            take_next(&mut queue),
            Some(Next::One(Input::Power(PowerEvent::Suspend, None)))
        ));
        assert!(matches!(
            take_next(&mut queue),
            Some(Next::One(Input::Power(PowerEvent::Resume, None)))
        ));
        assert!(matches!(
            take_next(&mut queue),
            Some(Next::One(Input::Exited { .. }))
        ));
        match take_next(&mut queue) {
            Some(Next::Switches(switches)) => {
                let ids: Vec<_> = switches.iter().map(|(id, ..)| id.as_str()).collect();
                assert_eq!(ids, ["s1", "s2"]);
            }
            _ => panic!("expected the queued switches together"),
        }
        assert!(matches!(
            take_next(&mut queue),
            Some(Next::One(Input::Inventory(_)))
        ));
        assert!(take_next(&mut queue).is_none());
    }

    #[test]
    fn a_dry_run_switched_just_before_sleep_is_not_restarted() {
        let (mut driver, id) = running(false);
        driver
            .switch_profile(&id, "Browser".into(), renamed("Browser"), 100)
            .unwrap();
        // Sleep is handled before the queued exit of the switched process.
        driver.power(PowerEvent::Suspend, 110);
        assert_eq!(driver.runner().stops, std::slice::from_ref(&id));
        exit(&mut driver, &id, "", 120);
        assert_eq!(starts(&mut driver), 1);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason, state.service_id),
            (Phase::Suspended, Some(Reason::Sleep), None)
        );
        driver.power(PowerEvent::Resume, 10_000);
        driver.tick(10_000 + SETTLE_DELAY_MS);
        assert_eq!(starts(&mut driver), 2);
        assert_eq!(driver.runner().starts[1].profile.name, "Browser");
    }

    #[test]
    fn the_driver_thread_batches_and_answers_queued_switches() {
        let mut driver = Driver::new(FakeRunner::default());
        driver.inventory(vec![dial("/a")], 0);
        let id = driver.start(request(&dial("/a"), false), 0).unwrap();
        let (inputs, receiver) = mpsc::channel();
        let (first, first_answer) = switch_input(&id);
        let (second, second_answer) = switch_input("service:unknown");
        let (done, finished) = mpsc::channel();
        inputs.send(first).unwrap();
        inputs.send(second).unwrap();
        inputs.send(Input::Shutdown(done)).unwrap();
        run_driver(&mut driver, &receiver, || 1);
        assert_eq!(first_answer.try_recv().unwrap(), Ok(1));
        assert!(second_answer.try_recv().unwrap().is_err());
        assert!(finished.try_recv().is_ok());
        assert_eq!(driver.runner().batches, [vec![id]]);
    }

    fn saved(names: &[(&str, &str)]) -> Vec<(String, Profile)> {
        names
            .iter()
            .map(|(id, name)| ((*id).into(), renamed(name)))
            .collect()
    }

    fn step_target(
        names: &[(&str, &str)],
        current_id: Option<&str>,
        current: &str,
        direction: ProfileDirection,
    ) -> Option<String> {
        resolve_profile_step(saved(names), current_id, &renamed(current), direction)
            .map(|(id, _)| id)
    }

    #[test]
    fn profile_steps_follow_a_stable_name_order_and_wrap_around() {
        use ProfileDirection::{Next, Previous};
        // Listed in storage order (most recently modified first); the
        // switch order is by name, ignoring case, then by id.
        let names = [
            ("p3", "streaming"),
            ("p1", "Browser"),
            ("p2", "Editor"),
            ("p0", "Editor"),
        ];
        assert_eq!(
            step_target(&names, Some("p1"), "x", Next).as_deref(),
            Some("p0")
        );
        assert_eq!(
            step_target(&names, Some("p0"), "x", Next).as_deref(),
            Some("p2")
        );
        assert_eq!(
            step_target(&names, Some("p3"), "x", Next).as_deref(),
            Some("p1")
        );
        assert_eq!(
            step_target(&names, Some("p1"), "x", Previous).as_deref(),
            Some("p3")
        );
        assert_eq!(
            step_target(&names, Some("p2"), "x", Previous).as_deref(),
            Some("p0")
        );
        // Without a switched id the running snapshot is matched by content.
        assert_eq!(
            step_target(&names, None, "Browser", Next).as_deref(),
            Some("p0")
        );
        // A deleted switched profile falls back to the content match.
        assert_eq!(
            step_target(&names, Some("gone"), "streaming", Next).as_deref(),
            Some("p1")
        );
        // An unsaved draft matches nothing: next is first, previous is last.
        assert_eq!(
            step_target(&names, None, "Draft", Next).as_deref(),
            Some("p1")
        );
        assert_eq!(
            step_target(&names, None, "Draft", Previous).as_deref(),
            Some("p3")
        );
        // Nothing to switch between with fewer than two saved profiles.
        assert_eq!(step_target(&names[..1], None, "Draft", Next), None);
        assert_eq!(
            step_target(&names[..1], Some("p3"), "streaming", Previous),
            None
        );
        assert_eq!(step_target(&[], None, "Draft", Next), None);
    }

    #[test]
    fn profile_switch_requests_are_parsed_strictly() {
        use serde_json::json;
        let record = |value: serde_json::Value| profile_switch_request(&value);
        assert_eq!(
            record(
                json!({"type":"profile_switch_requested","schema_version":1,"direction":"next","event":{}})
            ),
            Some(ProfileDirection::Next)
        );
        assert_eq!(
            record(
                json!({"type":"profile_switch_requested","schema_version":1,"direction":"previous"})
            ),
            Some(ProfileDirection::Previous)
        );
        for other in [
            json!({"type":"profile_switch_requested","schema_version":1,"direction":"first"}),
            json!({"type":"profile_switch_requested","schema_version":1,"direction":"Next"}),
            json!({"type":"profile_switch_requested","schema_version":2,"direction":"next"}),
            json!({"type":"profile_switch_requested","direction":"next"}),
            json!({"type":"profile_switch_requested","schema_version":1}),
            json!({"type":"profile_switch_requested","schema_version":1,"direction":{"name":"Desk"}}),
            json!({"type":"action_unavailable","schema_version":1,"direction":"next"}),
            json!({"type":"raw","line":"profile_switch_requested next"}),
            json!(["profile_switch_requested"]),
        ] {
            assert_eq!(record(other.clone()), None, "{other}");
        }
    }

    /// A running dry or executing session on `dial("/a")` whose snapshot is
    /// the saved "Folders" profile, with two other saved profiles.
    fn keyed(execute: bool) -> (Driver<FakeRunner>, String) {
        let (mut driver, id) = running(execute);
        driver.runner().saved = vec![
            ("id-c".into(), renamed("Streaming")),
            ("id-a".into(), renamed("Browser")),
            ("id-b".into(), crate::test_profiles::folders()),
        ];
        (driver, id)
    }

    #[test]
    fn a_profile_key_switches_a_dry_run_through_the_focused_app_path() {
        let (mut driver, id) = keyed(false);
        assert_eq!(
            driver.profile_step(&id, ProfileDirection::Next, 1_000),
            Ok(1)
        );
        assert_eq!(driver.runner().stops, std::slice::from_ref(&id));
        let state = last(&mut driver);
        assert_eq!(
            (
                state.reason,
                state.profile_generation,
                state.profile_id.as_deref()
            ),
            (Some(Reason::ProfileAction), 1, Some("id-c"))
        );
        assert_eq!(state.profile_name, "Streaming");
        // More requests from the stopping process are dropped (coalesced).
        assert_eq!(
            driver.profile_step(&id, ProfileDirection::Next, 2_000),
            Err(STEP_NOT_RUNNING.into())
        );
        exit(&mut driver, &id, "", 2_100);
        assert_eq!(starts(&mut driver), 2);
        assert_eq!(driver.runner().starts[1].profile.name, "Streaming");
        assert!(!driver.runner().starts[1].execute);
        assert_eq!(
            (phase(&mut driver), last(&mut driver).reason),
            (Phase::Running, None)
        );
        // The restarted process: wraps around from the last profile.
        assert_eq!(
            driver.profile_step(&id, ProfileDirection::Next, 2_200),
            Ok(2)
        );
        assert_eq!(last(&mut driver).profile_id.as_deref(), Some("id-a"));
        exit(&mut driver, &id, "", 2_300);
        // Debounced: within PROFILE_STEP_DEBOUNCE_MS of the accepted one.
        let reads = driver.runner().saved_reads;
        assert_eq!(
            driver.profile_step(
                &id,
                ProfileDirection::Previous,
                2_200 + PROFILE_STEP_DEBOUNCE_MS - 1
            ),
            Err(STEP_DEBOUNCED.into())
        );
        assert_eq!(
            driver.runner().saved_reads,
            reads,
            "dropped before storage is read"
        );
        assert_eq!(
            driver.profile_step(
                &id,
                ProfileDirection::Previous,
                2_200 + PROFILE_STEP_DEBOUNCE_MS
            ),
            Ok(3)
        );
        assert_eq!(last(&mut driver).profile_id.as_deref(), Some("id-c"));
    }

    #[test]
    fn a_profile_key_never_resumes_an_executing_session_on_its_own() {
        let (mut driver, id) = keyed(true);
        assert_eq!(
            driver.profile_step(&id, ProfileDirection::Previous, 1_000),
            Ok(1)
        );
        exit(&mut driver, &id, "", 1_100);
        let state = last(&mut driver);
        assert_eq!(
            (state.phase, state.reason, state.service_id.as_deref()),
            (Phase::NeedsConfirmation, Some(Reason::ProfileAction), None)
        );
        assert_eq!(state.profile_name, "Browser");
        // Nothing runs, so later presses cannot switch or confirm anything.
        assert_eq!(
            driver.profile_step(&id, ProfileDirection::Next, 5_000),
            Err(STEP_NOT_RUNNING.into())
        );
        driver.tick(1_000_000);
        driver.inventory(vec![dial("/a")], 1_000_001);
        assert_eq!(starts(&mut driver), 1);
        assert_eq!(phase(&mut driver), Phase::NeedsConfirmation);
        assert_eq!(
            driver.resume(&id, 0, 1_000_002),
            Err(PROFILE_CHANGED.into())
        );
        driver.resume(&id, 1, 1_000_003).unwrap();
        let restarted = driver.runner().starts[1].clone();
        assert!(restarted.execute);
        assert_eq!(restarted.profile.name, "Browser");
    }

    #[test]
    fn a_profile_key_does_nothing_asleep_alone_or_for_unknown_processes() {
        let (mut driver, id) = running(false);
        driver.runner().saved = saved(&[("only", "Browser")]);
        assert_eq!(
            driver.profile_step(&id, ProfileDirection::Next, 1_000),
            Err(STEP_NO_TARGET.into())
        );
        assert_eq!(
            driver.profile_step("service:unknown", ProfileDirection::Next, 5_000),
            Err(STEP_NOT_RUNNING.into())
        );
        driver.runner().saved = saved(&[("a", "Browser"), ("b", "Editor")]);
        driver.power(PowerEvent::Suspend, 6_000);
        assert_eq!(
            driver.profile_step(&id, ProfileDirection::Next, 6_001),
            Err(STEP_ASLEEP.into())
        );
        assert!(driver.runner().stops.iter().all(|stop| stop == &id));
        assert_eq!(driver.runner().stops.len(), 1, "only the sleep stopped it");
        assert!(
            driver
                .runner()
                .emitted
                .iter()
                .all(|state| state.profile_generation == 0)
        );
    }

    #[test]
    fn a_profile_key_starts_from_the_saved_profile_the_run_was_started_from() {
        // The run started from saved "Folders" unchanged (the frontend sends
        // its id), and the saved file was edited afterwards, so the running
        // copy no longer equals any saved profile. The id still places it:
        // next is "Streaming", not the first profile.
        let mut driver = Driver::new(FakeRunner::default());
        driver.inventory(vec![dial("/a")], 0);
        let mut start = request(&dial("/a"), false);
        start.profile_id = Some("00000000-0000-4000-8000-0000000000b2".into());
        let id = driver.start(start, 0).unwrap();
        driver.runner().saved = vec![
            (
                "00000000-0000-4000-8000-0000000000b1".into(),
                renamed("Browser"),
            ),
            (
                "00000000-0000-4000-8000-0000000000b2".into(),
                renamed("Folders v2"),
            ),
            (
                "00000000-0000-4000-8000-0000000000b3".into(),
                renamed("Streaming"),
            ),
        ];
        driver
            .profile_step(&id, ProfileDirection::Next, 1_000)
            .unwrap();
        assert_eq!(last(&mut driver).profile_name, "Streaming");
        // Previous from a known id never lands on that same id.
        assert_eq!(
            step_target(
                &[("a", "A"), ("b", "B")],
                Some("a"),
                "A",
                ProfileDirection::Previous
            )
            .as_deref(),
            Some("b")
        );
    }

    #[test]
    fn session_profile_returns_the_held_profile_of_the_shown_generation_only() {
        let (mut driver, id) = keyed(true);
        driver
            .profile_step(&id, ProfileDirection::Next, 1_000)
            .unwrap();
        exit(&mut driver, &id, "", 1_100);
        let held = driver.session_profile(&id, 1).unwrap();
        assert_eq!(held.profile_id.as_deref(), Some("id-c"));
        assert_eq!(held.profile_generation, 1);
        assert_eq!(held.profile.name, "Streaming");
        // A later edit of the saved profile does not change what resume runs.
        driver.runner().saved[0].1.name = "Streaming v2".into();
        assert_eq!(
            driver.session_profile(&id, 1).unwrap().profile.name,
            "Streaming"
        );
        let emitted = driver.runner().emitted.len();
        assert_eq!(driver.session_profile(&id, 0), Err(PROFILE_CHANGED.into()));
        assert_eq!(
            driver.runner().emitted.len(),
            emitted + 1,
            "the current state is sent again"
        );
        assert!(driver.session_profile("service:unknown", 1).is_err());
        driver.resume(&id, 1, 2_000).unwrap();
        assert_eq!(driver.runner().starts[1].profile.name, "Streaming");
    }

    #[test]
    fn a_spawn_forwards_nothing_after_its_exit() {
        use serde_json::json;
        let (sender, receiver) = mpsc::channel();
        let forwarder = SpawnForwarder::new(sender);
        let record = |direction: &str| {
            service::ServiceMessage::Record(service::RecordPayload {
                service_id: "service:41".into(),
                record: json!({"type":"profile_switch_requested","schema_version":1,"direction":direction}),
            })
        };
        forwarder.forward(&record("next"));
        forwarder.forward(&service::ServiceMessage::Record(service::RecordPayload {
            service_id: "service:41".into(),
            record: json!({"type":"event","status":"planned"}),
        }));
        forwarder.forward(&service::ServiceMessage::Status(
            service::StatusPayload::exited("service:41", false, Some(0), String::new()),
        ));
        // A late record of the stopped process: its successor shares the id.
        forwarder.forward(&record("previous"));
        let inputs: Vec<_> = receiver.try_iter().collect();
        assert_eq!(inputs.len(), 2);
        assert!(matches!(
            &inputs[0],
            Input::ProfileStep { service_id, direction: ProfileDirection::Next } if service_id == "service:41"
        ));
        assert!(matches!(
            &inputs[1],
            Input::Exited {
                exit_code: Some(0),
                ..
            }
        ));
    }

    #[test]
    fn the_driver_thread_applies_queued_profile_steps() {
        let (mut driver, id) = keyed(false);
        let (inputs, receiver) = mpsc::channel();
        for _ in 0..3 {
            inputs
                .send(Input::ProfileStep {
                    service_id: id.clone(),
                    direction: ProfileDirection::Next,
                })
                .unwrap();
        }
        let (done, finished) = mpsc::channel();
        inputs.send(Input::Shutdown(done)).unwrap();
        run_driver(&mut driver, &receiver, || 1_000);
        assert!(finished.try_recv().is_ok());
        // Three queued presses switch once: the process is stopping after the first.
        assert_eq!(driver.runner().stops, std::slice::from_ref(&id));
        assert_eq!(driver.runner().saved_reads, 1);
    }

    #[test]
    fn a_profile_switch_needs_a_live_session() {
        let (mut driver, id) = running(false);
        let error = driver
            .switch_profile("service:unknown", "Browser".into(), renamed("Browser"), 1)
            .unwrap_err();
        assert!(error.contains("unknown session"), "{error}");
        driver.stop(&id, true, 2);
        exit(&mut driver, &id, "", 3);
        assert!(
            driver
                .switch_profile(&id, "Browser".into(), renamed("Browser"), 4)
                .is_err()
        );
        assert_eq!(starts(&mut driver), 1);
    }

    #[test]
    fn a_start_while_asleep_spawns_nothing_until_wake() {
        for execute in [false, true] {
            let mut driver = Driver::new(FakeRunner::default());
            driver.inventory(vec![dial("/a")], 0);
            driver.power(PowerEvent::Suspend, 10);
            let id = driver.start(request(&dial("/a"), execute), 20).unwrap();
            assert_eq!(id, sid("/a"));
            assert_eq!(starts(&mut driver), 0, "spawned across sleep");
            let state = last(&mut driver);
            assert_eq!(
                (
                    state.session_id.as_str(),
                    state.phase,
                    state.reason,
                    state.service_id
                ),
                (id.as_str(), Phase::Suspended, Some(Reason::Sleep), None)
            );
            // Ticks and inventory never clear the sleep, however late.
            driver.tick(10 + 100 * MISSED_WAKE_MS);
            driver.inventory(vec![dial("/a")], 10 + 100 * MISSED_WAKE_MS);
            assert_eq!(starts(&mut driver), 0);
            assert_eq!(phase(&mut driver), Phase::Suspended);

            driver.power(PowerEvent::Resume, 2_000_000);
            driver.tick(2_000_000 + SETTLE_DELAY_MS - 1);
            assert_eq!(starts(&mut driver), 0, "started before the settle delay");
            driver.tick(2_000_000 + SETTLE_DELAY_MS);
            if execute {
                let state = last(&mut driver);
                assert_eq!(
                    (state.phase, state.reason),
                    (Phase::NeedsConfirmation, Some(Reason::Sleep))
                );
                assert_eq!(starts(&mut driver), 0, "executed without confirmation");
                driver.resume(&id, 0, 2_100_000).unwrap();
            }
            assert_eq!(starts(&mut driver), 1);
            assert_eq!(driver.runner().starts[0].execute, execute);
            assert_eq!(phase(&mut driver), Phase::Running);
        }
    }

    #[test]
    fn an_unidentified_device_is_refused_while_asleep() {
        let mut driver = Driver::new(FakeRunner::default());
        driver.power(PowerEvent::Suspend, 0);
        let error = driver.start(request(&dial("/x"), false), 1).unwrap_err();
        assert_eq!(error, ASLEEP);
        assert_eq!(starts(&mut driver), 0);
        driver.power(PowerEvent::Resume, 2);
        assert!(driver.start(request(&dial("/x"), false), 3).is_ok());
    }

    #[test]
    fn a_start_queued_behind_suspend_waits_for_wake() {
        let mut driver = Driver::new(FakeRunner::default());
        driver.inventory(vec![dial("/a")], 0);
        let (inputs, receiver) = mpsc::channel();
        let (reply, answer) = mpsc::channel();
        let (done, finished) = mpsc::channel();
        // Queued in this order; the power event is handled first.
        inputs
            .send(Input::Start(request(&dial("/a"), true), reply))
            .unwrap();
        inputs
            .send(Input::Power(PowerEvent::Suspend, None))
            .unwrap();
        inputs.send(Input::Shutdown(done)).unwrap();
        run_driver(&mut driver, &receiver, || 1);
        assert_eq!(answer.try_recv().unwrap(), Ok(sid("/a")));
        assert!(finished.try_recv().is_ok());
        assert_eq!(starts(&mut driver), 0, "spawned after the sleep stop");
        assert!(
            driver
                .runner()
                .emitted
                .iter()
                .all(|state| state.service_id.is_none())
        );
    }

    #[test]
    fn session_state_carries_the_current_profile_for_resync() {
        let (mut driver, id) = running(true);
        let state = last(&mut driver);
        assert_eq!(
            (
                state.profile_id,
                state.profile_name,
                state.profile_generation
            ),
            (None, crate::test_profiles::folders().name, 0)
        );
        driver
            .switch_profile(&id, "p-browser".into(), renamed("Browser"), 100)
            .unwrap();
        exit(&mut driver, &id, "", 150);
        let state = last(&mut driver);
        assert_eq!(state.phase, Phase::NeedsConfirmation);
        assert_eq!(
            (state.profile_id.as_deref(), state.profile_name.as_str()),
            (Some("p-browser"), "Browser")
        );
        assert_eq!(state.profile_generation, 1);
    }

    #[test]
    fn a_start_while_asleep_on_a_path_named_by_a_moved_session_returns_its_session_id() {
        let (mut driver, id) = running(false);
        driver.inventory(vec![], 1);
        exit(&mut driver, &id, "", 2);
        driver.inventory(vec![dial("/b")], 5_000);
        let other = device(DIAL, Some("SER-2"), "/a");
        driver.inventory(vec![dial("/b"), other.clone()], 5_001);
        driver.power(PowerEvent::Suspend, 5_002);
        exit(&mut driver, &sid("/b"), "", 5_003);
        let started = driver.start(request(&other, false), 5_004).unwrap();
        assert!(started.starts_with("session:"), "{started}");
        let state = last(&mut driver);
        assert_eq!(
            (state.session_id.as_str(), state.service_id, state.phase),
            (started.as_str(), None, Phase::Suspended)
        );
        assert_eq!(starts(&mut driver), 2, "spawned across sleep");
    }

    #[test]
    fn ticks_and_inventory_never_clear_a_sleep_without_a_wake() {
        // Power Nap dark wakes advance the clock and keep timers and inventory
        // polls running without any wake notification.
        for execute in [false, true] {
            let (mut driver, id) = running(execute);
            // A crash backoff timer is pending when the Mac goes to sleep.
            exit(&mut driver, &id, "boom", 50);
            driver.power(PowerEvent::Suspend, 100);
            let late = 100 + 100 * MISSED_WAKE_MS;
            for now in [100 + MISSED_WAKE_MS, late, late + SETTLE_DELAY_MS] {
                driver.tick(now);
                driver.inventory(vec![dial("/a")], now);
            }
            assert_eq!(starts(&mut driver), 1, "spawned while asleep");
            assert_eq!(phase(&mut driver), Phase::Suspended);
            assert_eq!(driver.next_due(), None);
            assert!(driver.resume(&id, 0, late + SETTLE_DELAY_MS + 1).is_err());
            assert_eq!(starts(&mut driver), 1);
        }
    }

    #[test]
    fn other_late_inputs_never_clear_a_sleep_without_a_wake() {
        for execute in [false, true] {
            let (mut driver, id) = running(execute);
            driver.power(PowerEvent::Suspend, 100);
            exit(&mut driver, &id, "", 101);
            let late = 100 + 100 * MISSED_WAKE_MS;
            // An unmanaged process exiting, a focused-app switch and a stop of
            // an unknown id, all long after the suspend.
            driver.exited("service:unmanaged", Some(1), String::new(), late);
            driver
                .switch_profile(&id, "p-browser".into(), renamed("Browser"), late + 1)
                .unwrap();
            driver.stop("service:unknown", false, late + 2);
            driver.stop("service:unknown", true, late + 3);
            driver.tick(late + SETTLE_DELAY_MS);
            driver.inventory(vec![dial("/a")], late + SETTLE_DELAY_MS);
            driver.tick(late + 2 * SETTLE_DELAY_MS);
            assert_eq!(starts(&mut driver), 1, "spawned while asleep");
            assert_eq!(phase(&mut driver), Phase::Suspended);
            assert_eq!(driver.next_due(), None);
        }
    }

    #[test]
    fn an_unidentified_start_that_clears_a_missed_wake_says_the_mac_just_woke() {
        let mut driver = Driver::new(FakeRunner::default());
        driver.power(PowerEvent::Suspend, 0);
        let error = driver
            .start(request(&dial("/x"), false), MISSED_WAKE_MS)
            .unwrap_err();
        assert_eq!(error, JUST_WOKE);
        assert_eq!(starts(&mut driver), 0);
        // The valve fired: after the settle delay the next start spawns.
        driver.tick(MISSED_WAKE_MS + SETTLE_DELAY_MS);
        assert!(
            driver
                .start(
                    request(&dial("/x"), false),
                    MISSED_WAKE_MS + SETTLE_DELAY_MS + 1
                )
                .is_ok()
        );
        assert_eq!(starts(&mut driver), 1);
    }

    #[test]
    fn a_dead_driver_refuses_starts_instead_of_running_unmanaged() {
        let manager = Manager::default();
        // A driver that ran and died: its receiver is gone.
        drop(manager.receiver.lock().unwrap().take());
        manager.running.store(true, Ordering::SeqCst);
        let device = dial("/a");
        assert_eq!(
            manager.start(request(&device, false)),
            Some(Err(DRIVER_STOPPED.to_string()))
        );
        // Stopping the plain process stays possible.
        assert_eq!(manager.stop("service:aa", false), Ok(false));
        // A sessions-only stop has no plain fallback: it reports the dead driver.
        assert_eq!(
            manager.stop("service:aa", true),
            Err(DRIVER_STOPPED.to_string())
        );
        // No driver configured: the caller falls back to plain services.
        let idle = Manager::default();
        assert_eq!(idle.start(request(&device, false)), None);
    }

    #[test]
    fn a_start_long_after_suspend_clears_a_missed_wake_but_still_settles() {
        for execute in [false, true] {
            let mut driver = Driver::new(FakeRunner::default());
            driver.inventory(vec![dial("/a")], 0);
            driver.power(PowerEvent::Suspend, 0);
            let id = driver
                .start(request(&dial("/a"), execute), MISSED_WAKE_MS)
                .unwrap();
            assert_eq!(starts(&mut driver), 0, "spawned without the settle delay");
            let state = last(&mut driver);
            assert_eq!(
                (state.phase, state.next_attempt_in_ms),
                (Phase::Suspended, Some(SETTLE_DELAY_MS))
            );
            driver.tick(MISSED_WAKE_MS + SETTLE_DELAY_MS);
            if execute {
                assert_eq!(phase(&mut driver), Phase::NeedsConfirmation);
                assert_eq!(starts(&mut driver), 0, "executed without confirmation");
                driver
                    .resume(&id, 0, MISSED_WAKE_MS + SETTLE_DELAY_MS + 1)
                    .unwrap();
            }
            assert_eq!(starts(&mut driver), 1);
            assert_eq!(phase(&mut driver), Phase::Running);
            // The driver is awake again: the next start spawns at once.
            let keys = device(ServiceModule::M003K, Some("K"), "/k");
            driver.inventory(vec![dial("/a"), keys.clone()], MISSED_WAKE_MS + 5_000);
            driver
                .start(request(&keys, false), MISSED_WAKE_MS + 5_001)
                .unwrap();
            assert_eq!(starts(&mut driver), 2);
        }
    }

    #[test]
    fn a_serial_less_device_gets_one_session_while_asleep() {
        let mut driver = Driver::new(FakeRunner::default());
        let bare = device(DIAL, None, "/a");
        driver.inventory(vec![bare.clone()], 0);
        driver.power(PowerEvent::Suspend, 1);
        let id = driver.start(request(&bare, false), 2).unwrap();
        let error = driver.start(request(&bare, false), 3).unwrap_err();
        assert!(error.contains("already runs this device"), "{error}");
        assert!(error.contains(&id), "{error}");
        // The session reclaims the only device of its module on any path, so
        // the same module on another path is refused too.
        let moved = device(DIAL, None, "/b");
        driver.inventory(vec![moved.clone()], 4);
        let error = driver.start(request(&moved, false), 5).unwrap_err();
        assert!(error.contains(&id), "{error}");
        driver.inventory(vec![bare.clone()], 6);
        driver.power(PowerEvent::Resume, 10);
        driver.tick(10 + SETTLE_DELAY_MS);
        assert_eq!(starts(&mut driver), 1);
        // Another serial-less module of another kind is not claimed.
        let keys = device(ServiceModule::M003K, None, "/k");
        driver.inventory(vec![bare.clone(), keys.clone()], 20);
        assert!(driver.start(request(&keys, false), 21).is_ok());
    }

    #[test]
    fn a_stopped_or_stopping_serial_less_session_does_not_claim_the_module() {
        let mut driver = Driver::new(FakeRunner::default());
        let bare = device(DIAL, None, "/a");
        let moved = device(DIAL, None, "/b");
        driver.inventory(vec![bare.clone()], 0);
        let id = driver.start(request(&bare, false), 0).unwrap();
        // Still stopping: its process is alive and it never restarts.
        driver.stop(&id, true, 1);
        driver.inventory(vec![moved.clone()], 2);
        let other = driver.start(request(&moved, false), 3).unwrap();
        assert_eq!(starts(&mut driver), 2);
        // Finished: forgotten once its process exited.
        exit(&mut driver, &id, "", 4);
        assert!(driver.session(&id).is_none());
        // A process-less session waiting to restart does claim it.
        exit(&mut driver, &other, "boom", 5);
        driver.inventory(vec![bare.clone()], 6);
        let error = driver.start(request(&bare, false), 7).unwrap_err();
        assert!(error.contains(&other), "{error}");
    }

    #[test]
    fn a_start_while_asleep_is_validated_like_one_that_spawns() {
        let mut driver = Driver::new(FakeRunner::default());
        driver.inventory(vec![dial("/a"), dial("/u")], 0);
        driver.power(PowerEvent::Suspend, 1);
        let mut invalid = request(&dial("/a"), false);
        invalid.profile.schema_version = 999;
        assert!(driver.start(invalid.clone(), 2).is_err());
        assert!(driver.session(&sid("/a")).is_none());
        assert!(driver.runner().emitted.is_empty());
        // The same profile is refused awake too, before anything spawns.
        driver.power(PowerEvent::Resume, 3);
        assert!(driver.start(invalid, 4).is_err());
        assert_eq!(starts(&mut driver), 0);
        // An unmanaged process already on the path refuses a start asleep.
        driver.power(PowerEvent::Suspend, 5);
        driver.runner().alive.insert(sid("/u"));
        let error = driver.start(request(&dial("/u"), false), 6).unwrap_err();
        assert!(error.contains("already running"), "{error}");
        assert!(driver.session(&sid("/u")).is_none());
    }

    const DISPLAY_ON: DisplaySettings = DisplaySettings {
        enabled: true,
        dial_content: crate::display::DialContent::Clock,
    };

    fn launched(driver: &mut Driver<FakeRunner>, index: usize) -> Option<DisplayLaunch> {
        driver.runner().starts[index].display
    }

    #[test]
    fn display_settings_are_read_once_per_start_and_kept_through_restarts() {
        let mut driver = Driver::new(FakeRunner {
            display: DISPLAY_ON,
            ..FakeRunner::default()
        });
        driver.inventory(vec![dial("/a")], 0);
        let id = driver.start(request(&dial("/a"), false), 0).unwrap();
        let expected = Some(DisplayLaunch {
            settings: DISPLAY_ON,
            module: DIAL,
            active_at_start: true,
        });
        assert_eq!(launched(&mut driver, 0), expected);
        // Turning the setting off mid-run changes nothing and restarts nothing.
        driver.runner().display = DisplaySettings::default();
        driver.tick(1_000_000);
        assert_eq!(starts(&mut driver), 1);
        assert!(driver.runner().stops.is_empty());
        // Wake, crash and profile-switch restarts keep the starting value.
        driver.power(PowerEvent::Suspend, 10);
        exit(&mut driver, &id, "", 11);
        driver.power(PowerEvent::Resume, 20);
        driver.tick(20 + SETTLE_DELAY_MS);
        assert_eq!(starts(&mut driver), 2);
        assert_eq!(launched(&mut driver, 1), expected);
        let current = last(&mut driver).service_id.unwrap();
        exit(&mut driver, &current, "boom", 3_000);
        driver.tick(3_000 + backoff_ms(1));
        assert_eq!(starts(&mut driver), 3);
        assert_eq!(launched(&mut driver, 2), expected);
        driver
            .switch_profile(&id, "Browser".into(), renamed("Browser"), 5_000)
            .unwrap();
        let current = driver.runner().stops.last().unwrap().clone();
        exit(&mut driver, &current, "", 5_001);
        assert_eq!(starts(&mut driver), 4);
        assert_eq!(launched(&mut driver, 3), expected);
        assert_eq!(driver.runner().starts[3].profile.name, "Browser");

        // The next Start reads the setting again.
        driver.stop(&id, true, 6_000);
        let current = driver.runner().stops.last().unwrap().clone();
        exit(&mut driver, &current, "", 6_001);
        driver.start(request(&dial("/a"), false), 7_000).unwrap();
        let off = Some(DisplayLaunch {
            settings: DisplaySettings::default(),
            module: DIAL,
            active_at_start: false,
        });
        assert_eq!(launched(&mut driver, 4), off);
    }

    #[test]
    fn a_display_change_never_resumes_an_executing_session() {
        let mut driver = Driver::new(FakeRunner {
            display: DISPLAY_ON,
            ..FakeRunner::default()
        });
        driver.inventory(vec![dial("/a")], 0);
        let id = driver.start(request(&dial("/a"), true), 0).unwrap();
        driver.power(PowerEvent::Suspend, 1);
        exit(&mut driver, &id, "", 2);
        driver.runner().display = DisplaySettings::default();
        driver.power(PowerEvent::Resume, 3);
        driver.tick(3 + SETTLE_DELAY_MS);
        assert_eq!(phase(&mut driver), Phase::NeedsConfirmation);
        driver.tick(1_000_000);
        assert_eq!(starts(&mut driver), 1, "resumed without confirmation");
        driver.resume(&id, 0, 1_000_001).unwrap();
        assert_eq!(starts(&mut driver), 2);
        assert!(driver.runner().starts[1].execute);
        // Confirmation restarts with the value the session started with.
        assert_eq!(launched(&mut driver, 1).unwrap().settings, DISPLAY_ON);
    }

    #[test]
    fn a_start_while_asleep_snapshots_display_settings_and_spawns_only_after_wake() {
        let mut driver = Driver::new(FakeRunner {
            display: DISPLAY_ON,
            ..FakeRunner::default()
        });
        driver.inventory(vec![dial("/a")], 0);
        driver.power(PowerEvent::Suspend, 1);
        driver.start(request(&dial("/a"), false), 2).unwrap();
        driver.runner().display = DisplaySettings::default();
        assert_eq!(starts(&mut driver), 0, "spawned across sleep");
        driver.power(PowerEvent::Resume, 10);
        driver.tick(10 + SETTLE_DELAY_MS);
        assert_eq!(starts(&mut driver), 1);
        assert_eq!(launched(&mut driver, 0).unwrap().settings, DISPLAY_ON);
    }

    #[test]
    fn a_switch_into_keypad_folders_never_starts_display_writes_mid_session() {
        let keypad = |path: &str| device(ServiceModule::M4315, Some("KEY-1"), path);
        let mut driver = Driver::new(FakeRunner {
            display: DISPLAY_ON,
            ..FakeRunner::default()
        });
        driver.inventory(vec![keypad("/k")], 0);
        let mut flat = crate::test_profiles::folders();
        flat.schema_version = 1;
        flat.keypad = None;
        flat.name = "Flat".into();
        let id = driver
            .start(
                StartRequest {
                    profile: flat,
                    ..request(&keypad("/k"), false)
                },
                0,
            )
            .unwrap();
        let args = |driver: &mut Driver<FakeRunner>, index: usize| {
            let started = driver.runner().starts[index].clone();
            crate::display::display_args(
                started.display.as_ref(),
                started.software_mode,
                &started.profile,
            )
        };
        assert!(args(&mut driver, 0).is_empty());
        // A focused-app rule switches the dry run to a keypad-folder profile.
        driver
            .switch_profile(&id, "Folders".into(), crate::test_profiles::folders(), 10)
            .unwrap();
        let current = driver.runner().stops.last().unwrap().clone();
        exit(&mut driver, &current, "", 11);
        assert_eq!(starts(&mut driver), 2);
        assert!(driver.runner().starts[1].profile.keypad.is_some());
        // No display flags: the service refuses the folders, as with the setting off.
        assert!(args(&mut driver, 1).is_empty());
    }

    #[test]
    fn an_unidentified_device_runs_without_display_settings() {
        let mut driver = Driver::new(FakeRunner {
            display: DISPLAY_ON,
            ..FakeRunner::default()
        });
        driver.start(request(&dial("/x"), false), 0).unwrap();
        assert_eq!(launched(&mut driver, 0), None);
    }
}
