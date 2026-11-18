//! Action dispatch shared by every platform, over a small set of host traits.
//!
//! [`HostDispatcher`] owns what does not depend on the operating system:
//! validation, `multi_action` sequences and their deadline, conditionals,
//! variables, OBS, Streamlabs, plugins and `run_program`. Everything that
//! talks to the system goes through a [`Host`]: audio ([`AudioHost`]), key
//! events ([`KeyInjector`]), opening apps, URLs and terminals ([`Launcher`]),
//! system commands and the focused app ([`SystemHost`]) and timer alerts
//! ([`Notifier`]). Each host operation gets the dispatcher's [`Process`], so
//! its subprocesses share one runner, clock and cancellation flag.
//!
//! [`crate::macos::MacOsHost`] is the only host today; [`crate::MacOsDispatcher`]
//! is this dispatcher over it. The traits are public only because they bound
//! the public dispatcher: [`Process`] has no public methods and the
//! dispatcher's constructor is crate-private, so a host outside this crate is
//! not supported.

pub(crate) mod process;

pub use process::Process;

use crate::{
    Action, ActionDispatcher, MacroStep, MediaCommand, Modifier, ServiceState, SystemCommand,
    TerminalApp, TimerAlert, TimerAlerter, VariableValue,
    obs::{OBS_UNAVAILABLE, ObsClient},
    plugin::{PLUGIN_UNAVAILABLE, PluginBridge},
    streamlabs::{STREAMLABS_UNAVAILABLE, StreamlabsClient},
};
use process::{ProcessRequest, ProcessRunner, Sleeper, SystemSleeper};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

/// Output and input volume and the output's mute state.
pub trait AudioHost {
    /// Moves the output volume by `delta` percentage points, clamped to 0 to 100.
    fn step_output_volume(&mut self, cx: &mut Process, delta: i16) -> Result<(), String>;

    /// Sets the output volume, in percentage points.
    fn set_output_volume(&mut self, cx: &mut Process, percent: u8) -> Result<(), String>;

    /// Sets the input (microphone) volume, in percentage points.
    fn set_input_volume(&mut self, cx: &mut Process, percent: u8) -> Result<(), String>;

    /// The output volume, in percentage points.
    fn output_volume(&mut self, cx: &mut Process) -> Result<u8, String>;

    /// Whether the output is muted.
    fn output_muted(&mut self, cx: &mut Process) -> Result<bool, String>;

    /// Mutes or unmutes the output.
    fn set_output_muted(&mut self, cx: &mut Process, muted: bool) -> Result<(), String>;
}

/// Key events sent to the focused application.
pub trait KeyInjector {
    /// Sends one validated key, which is not a media key, with its modifiers.
    fn shortcut(
        &mut self,
        cx: &mut Process,
        key: &str,
        modifiers: &[Modifier],
    ) -> Result<(), String>;

    /// Sends a media command to whichever player handles media keys.
    fn media(&mut self, cx: &mut Process, command: MediaCommand) -> Result<(), String>;

    /// Enters `text` into the focused application, then Return when `press_enter`.
    fn text(&mut self, cx: &mut Process, text: &str, press_enter: bool) -> Result<(), String>;

    /// Replays recorded key steps with their timing.
    fn macro_keys(&mut self, cx: &mut Process, steps: &[MacroStep]) -> Result<(), String>;
}

/// Opening applications, URLs and terminal windows.
pub trait Launcher {
    /// Opens the application with this validated identifier.
    fn launch_app(&mut self, cx: &mut Process, app_id: &str) -> Result<(), String>;

    /// Opens a validated `http` or `https` URL in the default browser.
    fn open_url(&mut self, cx: &mut Process, url: &str) -> Result<(), String>;

    /// Opens a new window of `app` running `command`, in `directory` when set.
    fn terminal_command(
        &mut self,
        cx: &mut Process,
        app: TerminalApp,
        command: &str,
        directory: Option<&str>,
    ) -> Result<(), String>;
}

/// System commands and what the system says about its focused app.
pub trait SystemHost {
    /// Runs one system command (undo, redo, screenshot).
    fn system(&mut self, cx: &mut Process, command: SystemCommand) -> Result<(), String>;

    /// The identifier of the focused application, as the `focused_app` condition compares it.
    fn frontmost_app(&mut self, cx: &mut Process) -> Result<String, String>;
}

/// Alerts for timers that ran out.
pub trait Notifier {
    /// Shows the notification and plays the sound of a validated alert.
    /// A warning is returned when the alert ran but may not have been shown.
    fn timer_alert(
        &mut self,
        cx: &mut Process,
        alert: &TimerAlert,
    ) -> Result<Option<String>, String>;
}

/// Everything a [`HostDispatcher`] needs from the operating system.
pub trait Host:
    AudioHost + KeyInjector + Launcher + SystemHost + Notifier + Send + 'static
{
    /// The platform's name in errors about actions it never runs itself.
    const NAME: &'static str;

    /// A fresh host for a timer-alert thread, without this one's state.
    fn for_alerts(&self) -> Self;
}

/// Executes validated actions through a [`Host`].
///
/// Failed or timed-out actions are returned to the caller and are never retried;
/// an action may already have taken effect before its process reports a failure.
pub struct HostDispatcher<H: Host> {
    pub(crate) process: Process,
    pub(crate) host: H,
    /// The loopback OBS client for `obs_scene`, when this run has an endpoint.
    pub(crate) obs: Option<ObsClient>,
    /// The loopback Streamlabs client for `streamlabs_*`, when this run has one.
    pub(crate) streamlabs: Option<StreamlabsClient>,
    /// The service state handed over before the current action.
    pub(crate) service_state: ServiceState,
    /// Current values of the declared variables, shared with the control channel.
    pub(crate) variables: crate::SharedVariables,
    /// The way to the app's plugin host, when the app started this run for plugins.
    pub(crate) plugins: Option<PluginBridge>,
    /// The run's cancellation flag, which also ends a wait for a plugin.
    pub(crate) cancellation: Option<Arc<AtomicBool>>,
}

impl<H: Host> HostDispatcher<H> {
    /// A dispatcher over `host` with no OBS, Streamlabs or plugins.
    pub(crate) fn with_parts(
        runner: Box<dyn ProcessRunner>,
        sleeper: Box<dyn Sleeper>,
        host: H,
        cancellation: Option<Arc<AtomicBool>>,
    ) -> Self {
        Self {
            process: Process::new(runner, sleeper),
            host,
            obs: None,
            streamlabs: None,
            service_state: ServiceState::default(),
            variables: Default::default(),
            plugins: None,
            cancellation,
        }
    }

    /// Runs `obs_scene` actions through `obs` (one loopback connection per
    /// action, bounded by [`crate::obs::DEFAULT_TIMEOUT`]). Without a client
    /// they are unavailable: [`ActionDispatcher::unavailable`] reports them and
    /// a direct dispatch fails without any I/O.
    pub fn with_obs(mut self, obs: Option<ObsClient>) -> Self {
        self.obs = obs;
        self
    }

    /// Runs `streamlabs_*` actions through `streamlabs`, like
    /// [`HostDispatcher::with_obs`]. Without a client they are unavailable
    /// ([`STREAMLABS_UNAVAILABLE`]) and a direct dispatch does no I/O.
    pub fn with_streamlabs(mut self, streamlabs: Option<StreamlabsClient>) -> Self {
        self.streamlabs = streamlabs;
        self
    }

    /// Runs `plugin` actions through `plugins`, which asks the app that
    /// started this run. Without a bridge they are unavailable
    /// ([`PLUGIN_UNAVAILABLE`]) and a direct dispatch does no I/O.
    pub fn with_plugins(mut self, plugins: Option<PluginBridge>) -> Self {
        self.plugins = plugins;
        self
    }

    /// Keeps variable values in `variables`, so the control channel can read
    /// and set them while the run goes on.
    pub fn with_variables(mut self, variables: crate::SharedVariables) -> Self {
        self.variables = variables;
        self
    }

    /// Queries current output volume and mute state through bounded subprocesses.
    ///
    /// This performs no setter operations. A fixed-volume output can return an
    /// unsupported value; malformed or unavailable state is reported as an error.
    pub fn audio_snapshot(&mut self) -> Result<AudioSnapshot, String> {
        Ok(AudioSnapshot {
            output_volume: self.host.output_volume(&mut self.process)?,
            output_muted: self.host.output_muted(&mut self.process)?,
        })
    }

    /// Attempts to restore both fields of a validated snapshot independently.
    ///
    /// A failed volume setter does not prevent the mute setter from being
    /// attempted. Errors are combined; neither setter is retried. Existing
    /// cancellation remains effective, including during restoration. A successful
    /// return reports system-tool completion, not a fresh state verification.
    pub fn restore_audio_snapshot(&mut self, snapshot: &AudioSnapshot) -> Result<(), String> {
        snapshot.validate()?;
        let volume = self
            .host
            .set_output_volume(&mut self.process, snapshot.output_volume)
            .map_err(|error| format!("restoring output volume failed: {error}"));
        let muted = self
            .host
            .set_output_muted(&mut self.process, snapshot.output_muted)
            .map_err(|error| format!("restoring output mute failed: {error}"));
        match (volume, muted) {
            (Ok(()), Ok(())) => Ok(()),
            (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
            (Err(volume), Err(muted)) => Err(format!("{volume}; {muted}")),
        }
    }

    fn execute(&mut self, action: &Action) -> Result<(), String> {
        let cx = &mut self.process;
        match action {
            Action::Volume { delta } => self.host.step_output_volume(cx, *delta)?,
            Action::SetVolume { percent } => self.host.set_output_volume(cx, *percent)?,
            Action::SetMicInputVolume { percent } => self.host.set_input_volume(cx, *percent)?,
            Action::VolumeFromPosition
            | Action::MicInputVolumeFromPosition
            | Action::ObsInputVolumeFromPosition { .. } => {
                unreachable!("unresolved templates are rejected before host operations")
            }
            Action::ToggleMute => {
                let target = !self.host.output_muted(cx)?;
                self.host.set_output_muted(cx, target)?;
            }
            Action::Shortcut { key, modifiers } => {
                if let Some(command) = crate::profile::media_key_command(key) {
                    // The validator refuses modifiers on a media key, so the
                    // event flags the host builds are exactly its own.
                    debug_assert!(
                        modifiers.is_empty(),
                        "a media key cannot combine with modifiers"
                    );
                    self.host.media(cx, command)?;
                } else {
                    self.host.shortcut(cx, key, modifiers)?;
                }
            }
            Action::LaunchApp { bundle_id } => self.host.launch_app(cx, bundle_id)?,
            Action::OpenUrl { url } => self.host.open_url(cx, url)?,
            Action::TerminalCommand {
                app,
                command,
                directory,
            } => self
                .host
                .terminal_command(cx, *app, command, directory.as_deref())?,
            Action::RunProgram { .. } => self.run_program(action, None)?,
            Action::Plugin { .. } => self.plugin_action(action, None)?,
            Action::Media { command } => self.host.media(cx, *command)?,
            Action::System { command } => self.host.system(cx, *command)?,
            Action::Text { text, press_enter } => self.host.text(cx, text, *press_enter)?,
            Action::Macro { steps } => self.host.macro_keys(cx, steps)?,
            Action::Profile { .. } => {
                // The service never owns the profile list; `process_work`
                // reports the request to a session manager instead.
                return Err(format!(
                    "profile switching is handled by the OpenStreamHub session manager, not the {} backend; no action was taken",
                    H::NAME
                ));
            }
            Action::Timer { .. } => {
                // The countdown belongs to the service worker (`process_work`),
                // which calls `timer_alert` only when it ends.
                return Err(format!(
                    "timer countdowns are run by the service worker, not the {} backend; no action was taken",
                    H::NAME
                ));
            }
            Action::ObsScene { .. }
            | Action::ObsVirtualCamera { .. }
            | Action::ObsStream { .. }
            | Action::ObsRecord { .. }
            | Action::ObsReplayBuffer { .. }
            | Action::ObsStudioMode { .. }
            | Action::ObsTransition
            | Action::ObsScreenshot
            | Action::ObsFilter { .. }
            | Action::ObsSourceVisibility { .. }
            | Action::ObsSceneCollection { .. }
            | Action::ObsProfile { .. }
            | Action::ObsInputMute { .. }
            | Action::ObsAdjustInputVolume { .. }
            | Action::ObsMediaSource { .. }
            | Action::ObsSetInputVolume { .. } => self.obs_action(action, None)?,
            Action::StreamlabsRecord
            | Action::StreamlabsStream { .. }
            | Action::StreamlabsStudioMode
            | Action::StreamlabsTransition
            | Action::StreamlabsScene { .. }
            | Action::StreamlabsSourceVisibility { .. }
            | Action::StreamlabsAudio { .. } => self.streamlabs_action(action, None)?,
            Action::MultiAction { actions } => {
                let deadline = self.process.now() + MULTI_ACTION_DEADLINE;
                self.run_steps(actions, "multi_action", deadline)?;
            }
            Action::SetVariable { name, value } => self.variables.set(name, value.clone())?,
            Action::ToggleVariable { name } => self.variables.toggle(name)?,
            Action::AddToVariable { name, amount } => self.variables.add(name, *amount)?,
            Action::Conditional { .. } => {
                let deadline = self.process.now() + MULTI_ACTION_DEADLINE;
                self.run_conditional(action, deadline)?;
            }
        }
        Ok(())
    }

    /// Reads the condition of a `conditional`, then runs the branch it picks.
    /// A nested `conditional` shares the outer one's `deadline`.
    fn run_conditional(
        &mut self,
        action: &Action,
        deadline: std::time::Instant,
    ) -> Result<(), String> {
        let Action::Conditional {
            when,
            then,
            otherwise,
        } = action
        else {
            return Err("not a conditional".into());
        };
        let holds = self.evaluate_condition(when, deadline)?;
        let (branch, label) = if holds {
            (then, "conditional then")
        } else {
            (otherwise, "conditional else")
        };
        self.run_steps(branch, label, deadline)
    }

    /// Runs `steps` in order with fixed spacing, all inside `deadline`; the
    /// first failure stops the rest. `label` names the sequence in errors.
    fn run_steps(
        &mut self,
        steps: &[Action],
        label: &str,
        deadline: std::time::Instant,
    ) -> Result<(), String> {
        let count = steps.len();
        for (index, step) in steps.iter().enumerate() {
            if index > 0 {
                self.process.pause(STEP_SPACING).map_err(|error| {
                    format!(
                        "{label} stopped before step {} of {count}: {error}",
                        index + 1
                    )
                })?;
            }
            if self.process.now() >= deadline {
                return Err(format!(
                    "{label} stopped before step {} of {count} ({}): the sequence exceeded its {} s overall deadline; earlier steps already took effect",
                    index + 1,
                    step.type_name(),
                    MULTI_ACTION_DEADLINE.as_secs()
                ));
            }
            // An OBS step gets only what is left of the sequence's deadline,
            // so a slow OBS cannot push the sequence past it.
            let result = if matches!(step, Action::Conditional { .. }) {
                self.run_conditional(step, deadline)
            } else if step.uses_obs() {
                self.obs_action(step, Some(deadline))
            } else if step.uses_streamlabs() {
                self.streamlabs_action(step, Some(deadline))
            } else if matches!(step, Action::RunProgram { .. }) {
                self.run_program(step, Some(deadline))
            } else if matches!(step, Action::Plugin { .. }) {
                self.plugin_action(step, Some(deadline))
            } else {
                self.execute(step)
            };
            result.map_err(|error| {
                format!(
                    "{label} step {} of {count} ({}) failed: {error}",
                    index + 1,
                    step.type_name()
                )
            })?;
        }
        Ok(())
    }

    /// Whether `condition` holds right now. An `all` or `any` reads every
    /// member in order, without stopping early, so a member that cannot be
    /// read fails the press whatever the others say. A failure names the
    /// member that could not be read and says no branch ran.
    pub(crate) fn evaluate_condition(
        &mut self,
        condition: &crate::profile::Condition,
        deadline: std::time::Instant,
    ) -> Result<bool, String> {
        use crate::profile::Condition;
        match condition {
            Condition::All(items) | Condition::Any(items) => {
                let mut results = Vec::with_capacity(items.len());
                for item in items {
                    results.push(self.evaluate_condition(item, deadline)?);
                }
                Ok(if matches!(condition, Condition::All(_)) {
                    results.iter().all(|holds| *holds)
                } else {
                    results.iter().any(|holds| *holds)
                })
            }
            leaf @ Condition::Plugin {
                plugin,
                condition: id,
                ..
            } => self
                .plugin_condition_holds(leaf, deadline)
                .map_err(|error| {
                    // The parameter errors (plugin_params) already start with the plugin's id, which this names.
                    let error = error
                        .strip_prefix(plugin.as_str())
                        .and_then(|rest| rest.strip_prefix(": "))
                        .unwrap_or(&error);
                    format!("conditional could not read {plugin} {id}: {error}; neither branch ran")
                }),
            leaf => if leaf.uses_obs() {
                self.condition_holds(leaf, deadline)
            } else {
                self.system_condition_holds(leaf)
            }
            .map_err(|error| {
                format!(
                    "conditional could not read {}: {error}; neither branch ran",
                    leaf.name()
                )
            }),
        }
    }

    /// Reads a condition about the default output, the timer, variables or the focused app from the host.
    fn system_condition_holds(
        &mut self,
        condition: &crate::profile::Condition,
    ) -> Result<bool, String> {
        use crate::profile::Condition;
        match condition {
            Condition::OutputMuted(expected) => {
                Ok(self.host.output_muted(&mut self.process)? == *expected)
            }
            Condition::OutputVolumeAtLeast(percent) => {
                Ok(self.host.output_volume(&mut self.process)? >= *percent)
            }
            Condition::TimerRunning(expected) => Ok(self.service_state.timer_running == *expected),
            Condition::VariableIs { name, value } => match self.variables.get(name) {
                Some(current) if current.kind() == value.kind() => Ok(current == *value),
                Some(current) => Err(format!(
                    "variable {name} is {}, not {}",
                    current.kind().name(),
                    value.kind().name()
                )),
                None => Err(format!("variable {name} is not declared")),
            },
            Condition::VariableAtLeast { name, value } => match self.variables.get(name) {
                Some(VariableValue::Number(current)) => Ok(current >= *value),
                Some(other) => Err(format!(
                    "variable {name} is {}, not number",
                    other.kind().name()
                )),
                None => Err(format!("variable {name} is not declared")),
            },
            Condition::FocusedApp(bundle_id) => Ok(self
                .host
                .frontmost_app(&mut self.process)?
                .eq_ignore_ascii_case(bundle_id)),
            _ => Err("this condition is read from OBS".into()),
        }
    }

    /// Reads a condition from OBS within what is left of `deadline`.
    fn condition_holds(
        &self,
        condition: &crate::profile::Condition,
        deadline: std::time::Instant,
    ) -> Result<bool, String> {
        let obs = self.obs.as_ref().ok_or(OBS_UNAVAILABLE)?;
        let remaining = deadline.saturating_duration_since(self.process.now());
        if remaining.is_zero() {
            return Err("the overall deadline has passed; OBS was not contacted".into());
        }
        obs.clone()
            .with_timeout(remaining)
            .condition_holds(condition)
    }

    /// Runs an OBS action within `deadline` (on the sleeper's clock) when
    /// given, and never beyond the client's own 3 s limit. A deadline that has
    /// already passed fails without connecting.
    pub(crate) fn obs_action(
        &self,
        action: &Action,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), String> {
        let obs = self.obs.as_ref().ok_or(OBS_UNAVAILABLE)?;
        let Some(deadline) = deadline else {
            return obs.run(action);
        };
        let remaining = deadline.saturating_duration_since(self.process.now());
        if remaining.is_zero() {
            return Err(format!(
                "the multi_action's {} s overall deadline has passed; OBS was not contacted",
                MULTI_ACTION_DEADLINE.as_secs()
            ));
        }
        obs.clone().with_timeout(remaining).run(action)
    }

    /// Runs a Streamlabs action like [`HostDispatcher::obs_action`] runs an
    /// OBS one: within `deadline` when given, never beyond the client's 3 s.
    fn streamlabs_action(
        &self,
        action: &Action,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), String> {
        let streamlabs = self.streamlabs.as_ref().ok_or(STREAMLABS_UNAVAILABLE)?;
        let Some(deadline) = deadline else {
            return streamlabs.run(action);
        };
        let remaining = deadline.saturating_duration_since(self.process.now());
        if remaining.is_zero() {
            return Err(format!(
                "the multi_action's {} s overall deadline has passed; Streamlabs was not contacted",
                MULTI_ACTION_DEADLINE.as_secs()
            ));
        }
        streamlabs.clone().with_timeout(remaining).run(action)
    }

    /// Asks the app's plugin host to run a `plugin` action, with the
    /// variables of its listed text parameters filled in, and waits for the
    /// plugin's answer within [`PLUGIN_TIMEOUT`] or what is left of
    /// `deadline`.
    fn plugin_action(
        &mut self,
        action: &Action,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), String> {
        let Action::Plugin {
            plugin,
            min_version,
            action: name,
            params,
            variables,
        } = action
        else {
            return Err("not a plugin action".into());
        };
        let bridge = self.plugins.as_ref().ok_or(PLUGIN_UNAVAILABLE)?;
        let params = self.plugin_params(plugin, params, variables)?;
        let timeout = match deadline {
            None => PLUGIN_TIMEOUT,
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(self.process.now());
                if remaining.is_zero() {
                    return Err(format!(
                        "the sequence's {} s overall deadline has passed; the plugin was not asked",
                        MULTI_ACTION_DEADLINE.as_secs()
                    ));
                }
                remaining.min(PLUGIN_TIMEOUT)
            }
        };
        bridge.invoke(
            plugin,
            min_version,
            name,
            &params,
            variables,
            self.service_state.input.as_ref(),
            timeout,
            self.cancellation.as_deref(),
        )
    }

    /// Asks the app's plugin host whether a plugin condition holds, with the
    /// variables of its listed text parameters filled in, within
    /// [`PLUGIN_CHECK_TIMEOUT`] or what is left of `deadline`, and compares the
    /// answer with the condition's `value`.
    fn plugin_condition_holds(
        &self,
        condition: &crate::profile::Condition,
        deadline: std::time::Instant,
    ) -> Result<bool, String> {
        let crate::profile::Condition::Plugin {
            plugin,
            min_version,
            condition: name,
            params,
            variables,
            value,
        } = condition
        else {
            return Err("not a plugin condition".into());
        };
        let bridge = self.plugins.as_ref().ok_or(PLUGIN_UNAVAILABLE)?;
        let params = self.plugin_params(plugin, params, variables)?;
        let remaining = deadline.saturating_duration_since(self.process.now());
        if remaining.is_zero() {
            return Err(format!(
                "the sequence's {} s overall deadline has passed; the plugin was not asked",
                MULTI_ACTION_DEADLINE.as_secs()
            ));
        }
        let holds = bridge.check(
            plugin,
            min_version,
            name,
            &params,
            variables,
            self.service_state.input.as_ref(),
            remaining.min(PLUGIN_CHECK_TIMEOUT),
            self.cancellation.as_deref(),
        )?;
        Ok(holds == *value)
    }

    /// The parameters of `plugin` with the variables listed in `variables` filled in, refused when the profile that
    /// planned the press was swapped out meanwhile.
    fn plugin_params(
        &self,
        plugin: &str,
        params: &std::collections::BTreeMap<String, crate::PluginValue>,
        variables: &[String],
    ) -> Result<std::collections::BTreeMap<String, crate::PluginValue>, String> {
        if variables.is_empty() {
            return Ok(params.clone());
        }
        // As for run_program: values are kept by name across a profile
        // swap, so a press the previous profile planned would fill in what
        // the new profile means by that name.
        if self.variables.swapped_away_from(self.service_state.profile) {
            return Err(format!(
                "{plugin}: the profile changed after this key was pressed; the plugin was not asked"
            ));
        }
        self.variables
            .read(|lookup| crate::plugin::fill_in_plugin_params(params, variables, lookup))
            .map_err(|error| format!("{plugin}: {error}; the plugin was not asked"))
    }

    /// Runs a `run_program` action: the executable directly, with its
    /// arguments as they are, in its directory (the home folder when none),
    /// within [`RUN_PROGRAM_TIMEOUT`] or what is left of `deadline`. The
    /// program's output is discarded and a failure never names the path or the
    /// arguments, which can carry tokens.
    fn run_program(
        &mut self,
        action: &Action,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), String> {
        let Action::RunProgram {
            program,
            args,
            directory,
            variables,
        } = action
        else {
            unreachable!("run_program is called only for a run_program action");
        };
        // Filled in first, so a value that breaks the argument rules stops the
        // press before anything else is looked at, from one snapshot of the
        // values so a write meanwhile cannot mix old and new ones.
        let args = if *variables {
            // Values are kept by name across a profile swap, so a press the
            // previous profile planned would fill in what the new profile
            // means by that name.
            if self.variables.swapped_away_from(self.service_state.profile) {
                return Err(format!(
                    "{RUN_PROGRAM}: the profile changed after this key was pressed; the program was not started"
                ));
            }
            self.variables
                .read(|lookup| crate::profile::fill_in_run_program_args(args, lookup))
                .map_err(|error| format!("{RUN_PROGRAM}: {error}; the program was not started"))?
        } else {
            args.clone()
        };
        let timeout = match deadline {
            None => RUN_PROGRAM_TIMEOUT,
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(self.process.now());
                if remaining.is_zero() {
                    return Err(format!(
                        "the sequence's {} s overall deadline has passed; the program was not started",
                        MULTI_ACTION_DEADLINE.as_secs()
                    ));
                }
                remaining.min(RUN_PROGRAM_TIMEOUT)
            }
        };
        let home = std::env::var("HOME")
            .ok()
            .filter(|home| home.starts_with('/'))
            .ok_or("the program was not started: the home folder is unknown")?;
        let current_dir = expand_home(directory.as_deref().unwrap_or("~"), &home);
        // Checked here so a renamed folder is not reported as a missing program.
        if self.process.checks_paths() && !std::path::Path::new(&current_dir).is_dir() {
            return Err("the program was not started: its start folder does not exist".into());
        }
        // No fixed environment: the program sees the service's own, locale
        // included. The shell's exit codes for a missing or unrunnable file
        // (an `env` interpreter not on the app's PATH, say) get a hint.
        let output = self.process.run_bounded(ProcessRequest {
            program: RUN_PROGRAM,
            path: Some(expand_home(program, &home)),
            args,
            current_dir: Some(current_dir),
            isolated: true,
            timeout,
            ..ProcessRequest::default()
        })?;
        if output.success {
            return Ok(());
        }
        let hint = if output.status == "exit status: 127" {
            " (the program or its interpreter was not found; the app's PATH is only the system folders, so use full paths)"
        } else if output.status == "exit status: 126" {
            " (the program or its interpreter could not be run; check that it is executable)"
        } else {
            ""
        };
        Err(format!("{RUN_PROGRAM}: {}{hint}", output.status))
    }
}

impl<H: Host> ActionDispatcher for HostDispatcher<H> {
    fn set_service_state(&mut self, state: ServiceState) {
        // A variable keeps its value while its name and kind stay; anything
        // else starts at the declared value, and an undeclared one is dropped.
        self.variables.declare(&state.variables);
        self.service_state = state;
    }

    fn dispatch(&mut self, action: &Action) -> Result<(), String> {
        // Validate even when the caller did not use the profile loader. This
        // also validates every multi_action step before the first one runs.
        action.validate_executable()?;
        self.execute(action)
    }

    fn unavailable(&self, action: &Action) -> Option<&'static str> {
        (self.obs.is_none() && action.uses_obs())
            .then_some(OBS_UNAVAILABLE)
            .or_else(|| {
                (self.streamlabs.is_none() && action.uses_streamlabs())
                    .then_some(STREAMLABS_UNAVAILABLE)
            })
            .or_else(|| {
                (self.plugins.is_none() && action.uses_plugins()).then_some(PLUGIN_UNAVAILABLE)
            })
    }

    /// Fills in the variables of the owning action's parameters and writes the event; an action whose variables
    /// cannot be filled in, or a record that cannot be written, sends nothing.
    fn plugin_event(&mut self, action: &Action, input: &crate::plugin::PluginInput) {
        let Action::Plugin {
            plugin,
            min_version,
            action: name,
            params,
            variables,
        } = action
        else {
            return;
        };
        let Some(bridge) = self.plugins.as_ref() else {
            return;
        };
        if let Ok(params) = self.plugin_params(plugin, params, variables) {
            let _ = bridge.event(plugin, min_version, name, &params, variables, input);
        }
    }

    /// Validates the alert, then hands it to the host's [`Notifier`].
    fn timer_alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        alert.validate()?;
        self.host.timer_alert(&mut self.process, alert)
    }

    /// A second dispatcher cancelled by `cancel`, for an alert thread. It only
    /// raises alerts, so it gets no OBS, Streamlabs or plugin client.
    fn timer_alerter(&mut self, cancel: Arc<AtomicBool>) -> Option<Box<dyn TimerAlerter>> {
        Some(Box::new(Self::with_parts(
            self.process.runner.fork(cancel)?,
            Box::new(SystemSleeper),
            self.host.for_alerts(),
            None,
        )))
    }
}

impl<H: Host> TimerAlerter for HostDispatcher<H> {
    fn alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        self.timer_alert(alert)
    }
}

/// Fixed delay between `multi_action` steps, matching the official app.
pub(crate) const STEP_SPACING: Duration = Duration::from_millis(280);
/// Upper bound on the wall-clock time of a whole `multi_action`. It is checked
/// before each step starts; a step that is already running is bounded only by
/// its own subprocess timeouts.
pub(crate) const MULTI_ACTION_DEADLINE: Duration = Duration::from_secs(15);

/// Bound for a `run_program` that is a key's own action; as a step it gets
/// what is left of the sequence's [`MULTI_ACTION_DEADLINE`], never more than this.
pub(crate) const RUN_PROGRAM_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a `plugin` action waits for the plugin's answer as a key's own
/// action; as a step it gets what is left of the sequence's deadline.
pub(crate) const PLUGIN_TIMEOUT: Duration = Duration::from_secs(15);
/// Most a plugin condition waits for its answer, the OBS client's limit; it
/// never gets more than what is left of the sequence's deadline.
pub(crate) const PLUGIN_CHECK_TIMEOUT: Duration = Duration::from_secs(3);
/// The name a `run_program` request carries in errors and logs, in place of
/// the profile's path (which can name the user).
pub(crate) const RUN_PROGRAM: &str = "run_program";

/// Expands a leading `~` (alone or before `/`) of a validated `run_program`
/// path to `home`; any other path is returned as it is.
pub(crate) fn expand_home(path: &str, home: &str) -> String {
    if path == "~" {
        home.to_owned()
    } else if let Some(rest) = path.strip_prefix("~/") {
        format!("{}/{rest}", home.trim_end_matches('/'))
    } else {
        path.to_owned()
    }
}

/// The system output's volume and mute state, suitable for exact restoration.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AudioSnapshot {
    /// Output volume in percentage points, between 0 and 100 inclusive.
    pub output_volume: u8,
    /// Whether the system output is muted.
    pub output_muted: bool,
}

impl AudioSnapshot {
    /// Checks the volume before a caller-provided snapshot reaches system tools.
    pub fn validate(&self) -> Result<(), String> {
        if self.output_volume > 100 {
            Err("audio snapshot output volume must be between 0 and 100".into())
        } else {
            Ok(())
        }
    }
}
