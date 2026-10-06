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
//! See `docs/masterhub-session-manager.md`.

use crate::{
    feed::LiveDevice,
    power::PowerEvent,
    service::{self, StartRequest},
};
use cooltek_sdk_service::ServiceModule;
use serde::Serialize;
use std::{
    collections::BTreeSet,
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
    /// `service_id` on `device`.
    pub fn start(
        session_id: String,
        service_id: String,
        device: &LiveDevice,
        execute: bool,
        inventory: Vec<LiveDevice>,
        now_ms: u64,
    ) -> (Self, Vec<Action>) {
        let mut session = Self {
            session_id,
            module: device.module,
            serial_number: blank_to_none(device.serial_number.as_deref()),
            execute,
            path_hex: device.path_hex.clone(),
            phase: Phase::Running,
            reason: None,
            attempt: 0,
            detail: None,
            service: Some(Process {
                service_id,
                started_ms: now_ms,
                running: true,
                stopping: false,
            }),
            inventory,
            asleep: false,
            settle_until: None,
            retry_at: None,
            confirmed: false,
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
            Event::Suspend | Event::Resume | Event::Tick | Event::UserStop | Event::UserResume => {}
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
}

struct Managed {
    session: Session,
    request: StartRequest,
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
        }
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
    pub fn start(&mut self, request: StartRequest, now_ms: u64) -> Result<String, String> {
        if self.is_closed() {
            return Err(service::SHUTTING_DOWN.into());
        }
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
        if let Some(device) = &device
            && let Some(other) = self
                .sessions
                .iter()
                .find(|managed| managed.session.same_identity(device))
        {
            return Err(format!(
                "session {} already runs this device",
                other.session.session_id
            ));
        }
        let id = self.runner.start(&request)?;
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
        let (session, actions) = Session::start(
            session_id,
            id.clone(),
            &device,
            request.execute,
            inventory,
            now_ms,
        );
        self.sessions.push(Managed { session, request });
        self.execute(self.sessions.len() - 1, actions, now_ms);
        Ok(id)
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

    pub fn resume(&mut self, session_id: &str, now_ms: u64) -> Result<(), String> {
        let index = self
            .sessions
            .iter()
            .position(|managed| managed.session.session_id == session_id)
            .ok_or_else(|| format!("unknown session {session_id}"))?;
        if self.sessions[index].session.phase != Phase::NeedsConfirmation {
            return Err(format!("session {session_id} does not need confirmation"));
        }
        self.apply(index, Event::UserResume, now_ms);
        self.prune();
        Ok(())
    }

    pub fn inventory(&mut self, devices: Vec<LiveDevice>, now_ms: u64) {
        self.inventory = Some(devices.clone());
        self.broadcast(|| Event::Inventory(devices.clone()), now_ms);
    }

    /// Every process a sleep (or wake) interrupts is stopped in parallel.
    pub fn power(&mut self, event: PowerEvent, now_ms: u64) {
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

    /// Fires every timer due at `now_ms`.
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
    Resume(String, Sender<Result<(), String>>),
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
            Input::Resume(id, reply) => {
                let _ = reply.send(self.resume(&id, now_ms));
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
}

/// Runs the driver until shutdown, waking for inputs and due timers.
pub fn run_driver<R: Runner>(
    driver: &mut Driver<R>,
    inputs: &Receiver<Input>,
    clock: impl Fn() -> u64,
) {
    loop {
        let now = clock();
        let wait = driver.next_due().map_or(IDLE_WAIT, |due| {
            Duration::from_millis(due.saturating_sub(now))
        });
        match inputs.recv_timeout(wait) {
            Ok(input) => {
                if !driver.handle(input, clock()) {
                    return;
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
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

    /// `None` when no driver runs, so the caller falls back to plain services.
    fn request<T>(&self, make: impl FnOnce(Sender<T>) -> Input) -> Option<Result<T, String>> {
        if !self.running.load(Ordering::SeqCst) {
            return None;
        }
        let (reply, answer) = mpsc::channel();
        if self.inputs.send(make(reply)).is_err() {
            return None;
        }
        Some(
            answer
                .recv_timeout(REPLY_WAIT)
                .map_err(|_| "the session manager did not respond".to_string()),
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
    pub fn stop(&self, id: &str, sessions_only: bool) -> Result<bool, String> {
        self.request(|done| Input::Stop {
            id: id.into(),
            sessions_only,
            done,
        })
        .map_or(Ok(false), |answer| answer.map(|()| true))
    }

    pub fn resume(&self, session_id: &str) -> Result<(), String> {
        self.request(|reply| Input::Resume(session_id.into(), reply))
            .unwrap_or_else(|| Err(format!("unknown session {session_id}")))
            .and_then(|resumed| resumed)
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
    }

    impl Runner for FakeRunner {
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
            driver.resume(&id, 0).is_err(),
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

        driver.resume(&id, 1_000_002).unwrap();
        assert_eq!(starts(&mut driver), 2);
        let restarted = driver.runner().starts[1].clone();
        assert!(restarted.execute);
        assert_eq!(restarted.profile, crate::test_profiles::folders());
        assert_eq!(phase(&mut driver), Phase::Running);
        assert!(driver.resume("service:unknown", 0).is_err());
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
        driver.resume(&id, 4).unwrap();
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
        driver.resume(&id, 100_001).unwrap();
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
            assert!(driver.resume(&id, 7).is_err());
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
            sid("/a"),
            &dial("/a"),
            false,
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
        assert!(manager.resume(&id).is_err());
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
        assert_eq!(started, sid("/a"));
        let state = last(&mut driver);
        assert_eq!(state.service_id, Some(sid("/a")));
        assert_eq!(state.serial_number.as_deref(), Some("SER-2"));
        // Its name cannot be the old session's, which is still in use.
        assert!(state.session_id.starts_with("session:"), "{state:?}");
        assert_ne!(state.session_id, id);
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
}
