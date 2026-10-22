//! Explicit macOS actions through system tools. Constructing a dispatcher performs no actions.

use crate::{
    Action, ActionDispatcher, MacroStep, MediaCommand, Modifier, SystemCommand, TimerAlert,
    TimerAlerter,
    obs::{OBS_UNAVAILABLE, ObsClient},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

const OSASCRIPT: &str = "/usr/bin/osascript";
const OPEN: &str = "/usr/bin/open";
const PBCOPY: &str = "/usr/bin/pbcopy";
const PBPASTE: &str = "/usr/bin/pbpaste";
const SCREENCAPTURE: &str = "/usr/sbin/screencapture";
const AFPLAY: &str = "/usr/bin/afplay";
const PROCESS_TIMEOUT: Duration = Duration::from_secs(5);
/// Bound for the timer notification (`osascript` returns at once once the
/// notification is posted).
const TIMER_NOTIFICATION_TIMEOUT: Duration = Duration::from_secs(3);
/// Bound for the timer sound. The longest closed-set file (`Funk.aiff`) lasts
/// 2.2 s and `afplay` takes about 2.9 s of wall time for it, so 6 s never cuts
/// a sound short. Alerts run off the action worker, so this never delays actions.
const TIMER_SOUND_TIMEOUT: Duration = Duration::from_secs(6);
/// Fixed notification title of an expired timer.
const TIMER_TITLE: &str = "Timer finished";
/// Fixed AppleScript of the timer notification. Title and message arrive as
/// `argv` items after `--`, so no profile text is ever part of the script.
const TIMER_NOTIFICATION_SCRIPT: [&str; 3] = [
    "on run argv",
    "display notification (item 2 of argv) with title (item 1 of argv)",
    "end run",
];
/// `NX_KEYTYPE_PLAY`, `NX_KEYTYPE_NEXT` and `NX_KEYTYPE_PREVIOUS` from IOKit's
/// `ev_keymap.h`: the key types a keyboard's media keys report.
const NX_KEYTYPE_PLAY: u8 = 16;
const NX_KEYTYPE_NEXT: u8 = 17;
const NX_KEYTYPE_PREVIOUS: u8 = 18;
/// Fixed JavaScript for Automation script that posts one hardware media key,
/// as a keyboard's media key does, so it reaches whichever app currently owns
/// media-key handling. The key type is its only input: one argv item from
/// the closed set above, which the script checks again.
///
/// Each event is an `NX_SYSDEFINED` (14) event of subtype 8
/// (`NX_SUBTYPE_AUX_CONTROL_BUTTONS`) whose `data1` is
/// `key type << 16 | key state << 8`, state 0xA down and 0xB up, with the same
/// state in the flags' second byte. The usual recipe builds it with
/// `NSEvent.otherEventWithType…` and takes its `CGEvent`, but in `osascript`
/// that `CGEvent` reference comes back unusable (`CGEventGetType` reads 0;
/// checked on macOS 27 without posting). So the event is built with public
/// CoreGraphics setters on undocumented field numbers: 83 subtype, 149
/// `data1`, 150 `data2`. A Swift comparison found the result byte-identical to
/// the `NSEvent` one apart from an unused location. Before anything is posted,
/// both events are read back through `NSEvent.eventWithCGEvent` and must show
/// exactly type 14, subtype 8 and the expected `data1`; if a future macOS
/// moved those fields, the script fails and posts nothing.
///
/// Posting needs the Accessibility (post event) permission of the process
/// macOS holds responsible. Without it `CGEventPost` drops events silently,
/// so the script checks first and fails. Its second argv item is `request` or
/// `check`: only `request` also asks macOS to show its permission prompt, and
/// a dispatcher passes it on its first media key only, so a knob turned while
/// access is denied cannot pile up prompts.
const MEDIA_KEY_SCRIPT: &[&str] = &[
    "ObjC.import('AppKit');",
    "ObjC.import('CoreGraphics');",
    "ObjC.bindFunction('CGPreflightPostEventAccess', ['bool', []]);",
    "ObjC.bindFunction('CGRequestPostEventAccess', ['bool', []]);",
    "function run(argv) {",
    "if (argv.length !== 2 || ['16', '17', '18'].indexOf(argv[0]) < 0 || ['request', 'check'].indexOf(argv[1]) < 0) throw new Error('media-key: unknown key type');",
    "var key = Number(argv[0]);",
    "if (!$.CGPreflightPostEventAccess()) {",
    "if (argv[1] === 'request') $.CGRequestPostEventAccess();",
    "throw new Error('media-key: not allowed to post events');",
    "}",
    "var events = [0xA, 0xB].map(function (state) {",
    "var data1 = (key << 16) | (state << 8);",
    "var event = $.CGEventCreate(null);",
    "$.CGEventSetType(event, 14);",
    "$.CGEventSetFlags(event, state << 8);",
    "$.CGEventSetIntegerValueField(event, 83, 8);",
    "$.CGEventSetIntegerValueField(event, 149, data1);",
    "$.CGEventSetIntegerValueField(event, 150, -1);",
    "var check = $.NSEvent.eventWithCGEvent(event);",
    "if (check.isNil() || Number(check.type) !== 14 || Number(check.subtype) !== 8 || Number(check.data1) !== data1) throw new Error('media-key: event check failed');",
    "return event;",
    "});",
    "events.forEach(function (event) { $.CGEventPost(0, event); });",
    "}",
];
/// Marker the media-key script throws when posting events is not allowed.
const MEDIA_KEY_DENIED: &str = "media-key: not allowed to post events";
/// Marker the media-key script throws when the built event reads back wrong.
const MEDIA_KEY_CHECK_FAILED: &str = "media-key: event check failed";
/// macOS has no stop media key, so `stop` asks Apple Music (`stop`) and
/// Spotify (`pause`) directly, and only copies that are already running, found
/// with `NSRunningApplication` (which starts nothing). Immediately before each
/// command the script checks again that the process id still belongs to that
/// player, and skips it otherwise.
///
/// Music gets its `stop` event (`hook`/`Stop` in its sdef) as an Apple event
/// addressed to the process id. Such an event cannot start a process: sent to
/// a process that has quit it just fails (checked with TextEdit, which stayed
/// closed). Its error cannot be read safely in `osascript` (reading the
/// `NSError` reference crashed it), so a failure is reported as `no reply`.
/// Spotify's event codes cannot be checked here, so it keeps
/// `Application(pid).pause()`. JXA may resolve that by name, so if Spotify
/// quits in the instant between the re-check and the command, macOS may
/// relaunch it; that does not start playback.
///
/// A failure for a player that has meanwhile quit is not reported. Nothing
/// running is a successful no-op that prints `nothing to stop`. The fixed
/// script takes no input. Commands need the Automation permission for each
/// player.
const MEDIA_STOP_SCRIPT: &[&str] = &[
    "ObjC.import('AppKit');",
    "function running(pid, bundle) {",
    "var app = $.NSRunningApplication.runningApplicationWithProcessIdentifier(pid);",
    "return !app.isNil() && ObjC.unwrap(app.bundleIdentifier) === bundle;",
    "}",
    "function stopMusic(pid) {",
    "var target = $.NSAppleEventDescriptor.descriptorWithProcessIdentifier(pid);",
    "var event = $.NSAppleEventDescriptor.appleEventWithEventClassEventIDTargetDescriptorReturnIDTransactionID(0x686F6F6B, 0x53746F70, target, -1, 0);",
    "if (event.sendEventWithOptionsTimeoutError(3, 2, null).isNil()) throw new Error('no reply');",
    "}",
    "function pauseSpotify(pid) { Application(pid).pause(); }",
    "function run(argv) {",
    "var players = [['com.apple.Music', 'Music', stopMusic], ['com.spotify.client', 'Spotify', pauseSpotify]];",
    "var stopped = [];",
    "var failed = [];",
    "players.forEach(function (player) {",
    "var found = $.NSRunningApplication.runningApplicationsWithBundleIdentifier(player[0]);",
    "for (var index = 0; index < found.count; index++) {",
    "var pid = found.objectAtIndex(index).processIdentifier;",
    "if (!running(pid, player[0])) continue;",
    "try { player[2](pid); stopped.push(player[1]); }",
    "catch (error) { if (running(pid, player[0])) failed.push('media-stop: ' + player[1] + ' failed (' + (error.errorNumber ? Number(error.errorNumber) : 'no reply') + ')'); }",
    "}",
    "});",
    "if (failed.length) throw new Error(failed.join('; '));",
    "return stopped.length ? 'stopped ' + stopped.join(', ') : 'nothing to stop';",
    "}",
];
/// The players `stop` addresses, as named in the stop script's failures.
const MEDIA_STOP_PLAYERS: [&str; 2] = ["Music", "Spotify"];
const OUTPUT_LIMIT: usize = 16 * 1024;
/// Largest stdin payload. It is below the 16 KiB initial macOS pipe buffer, so
/// writing it before monitoring the child cannot block on a child that never reads.
const STDIN_LIMIT: usize = 8 * 1024;
/// Fixed delay between `multi_action` steps, matching the official app.
const STEP_SPACING: Duration = Duration::from_millis(280);
/// Delay before restoring the clipboard, so the target application can read
/// the pasted snippet first. A slower application can still paste the restored text.
const CLIPBOARD_RESTORE_DELAY: Duration = Duration::from_millis(500);
/// Pasteboard tools choose their text encoding from the locale; a bundled app
/// can start without one, so clipboard tools always run with UTF-8.
const UTF8_ENV: &[(&str, &str)] = &[("LANG", "en_US.UTF-8"), ("LC_ALL", "en_US.UTF-8")];
/// Upper bound on the wall-clock time of a whole `multi_action`. It is checked
/// before each step starts; a step that is already running is bounded only by
/// its own subprocess timeouts.
const MULTI_ACTION_DEADLINE: Duration = Duration::from_secs(15);
/// Clipboard type names that `clipboard info` reports for plain text only.
const PLAIN_TEXT_TYPES: &[&str] = &["«class utf8»", "«class ut16»", "string", "Unicode text"];

/// Fixed JavaScript for Automation script that replays a `macro`: it reads the
/// step list from stdin as a JSON array, validates every field against the
/// same bounds the profile validator enforces, checks the Accessibility
/// (post event) permission, and then posts each keystroke as a paired
/// `CGEventCreateKeyboardEvent` down and up with the recorded modifiers and
/// the waits and holds between them.
///
/// The step list never touches `argv`: every step is a validated integer
/// (virtual key code, modifier flags, wait, hold) written to stdin, which is
/// bounded well below the 16 KiB pipe buffer. The script re-checks every
/// bound, so a future caller bug posts nothing rather than something
/// unexpected. Before anything is posted, every event is read back through
/// `NSEvent.eventWithCGEvent` and must show exactly the expected type and key
/// code; if a future macOS moved those fields, the script fails and posts
/// nothing.
///
/// Posting needs the Accessibility (post event) permission of the process
/// macOS holds responsible. Without it `CGEventPost` drops events silently,
/// so the script preflights first and fails with a fixed marker; the
/// permission flow of the app links to System Settings.
const MACRO_SCRIPT: &[&str] = &[
    "ObjC.import('AppKit');",
    "ObjC.import('CoreGraphics');",
    "ObjC.import('Foundation');",
    "ObjC.bindFunction('CGPreflightPostEventAccess', ['bool', []]);",
    "function run(argv) {",
    "if (argv.length !== 0) throw new Error('macro: unexpected argument');",
    "var data = $.NSFileHandle.fileHandleWithStandardInput.readDataToEndOfFile;",
    "var steps = JSON.parse($.NSString.alloc.initWithDataEncoding(data, $.NSUTF8StringEncoding).js);",
    "if (!Array.isArray(steps) || steps.length < 1 || steps.length > 64) throw new Error('macro: bad step count');",
    "var total = 0;",
    "steps.forEach(function (step) {",
    "if (!step || typeof step !== 'object' || Array.isArray(step)) throw new Error('macro: bad step');",
    "if (Object.keys(step).sort().join(',') !== 'code,delay,flags,hold') throw new Error('macro: bad step fields');",
    "if (typeof step.code !== 'number' || step.code < 0 || step.code > 127 || step.code % 1 !== 0) throw new Error('macro: bad key code');",
    "if (typeof step.flags !== 'number' || step.flags < 0 || step.flags > 1966080 || (step.flags & ~1966080) !== 0) throw new Error('macro: bad modifier flags');",
    "if (typeof step.delay !== 'number' || step.delay < 0 || step.delay > 10000 || step.delay % 1 !== 0) throw new Error('macro: bad wait');",
    "if (typeof step.hold !== 'number' || step.hold < 0 || step.hold > 5000 || step.hold % 1 !== 0) throw new Error('macro: bad hold');",
    "total += step.delay + step.hold;",
    "});",
    "if (total > 30000) throw new Error('macro: total duration exceeded');",
    "if (!$.CGPreflightPostEventAccess()) throw new Error('macro: not allowed to post events');",
    "function sleep(ms) { if (ms > 0) $.NSThread.sleepForTimeInterval(ms / 1000); }",
    "function post(down, code, flags) {",
    "var event = $.CGEventCreateKeyboardEvent($(), code, down);",
    "$.CGEventSetFlags(event, flags);",
    "var check = $.NSEvent.eventWithCGEvent(event);",
    "if (check.isNil() || Number(check.type) !== (down ? 10 : 11) || Number(check.keyCode) !== code) throw new Error('macro: event check failed');",
    "$.CGEventPost(0, event);",
    "}",
    "steps.forEach(function (step) {",
    "sleep(step.delay);",
    "post(true, step.code, step.flags);",
    "sleep(step.hold);",
    "post(false, step.code, step.flags);",
    "});",
    "return 'ok';",
    "}",
];
/// Marker the macro script throws when posting events is not allowed.
const MACRO_DENIED: &str = "macro: not allowed to post events";
/// Marker the macro script throws when a built event reads back wrong.
const MACRO_CHECK_FAILED: &str = "macro: event check failed";
/// The replay subprocess gets the macro's total duration plus this margin for
/// startup, reading stdin and validation. An in-flight script past that is
/// killed, so a stuck step cannot hold the action worker forever.
const MACRO_TIMEOUT_MARGIN: Duration = Duration::from_secs(5);
/// `kCGEventFlagMaskShift`, `kCGEventFlagMaskControl`,
/// `kCGEventFlagMaskAlternate` and `kCGEventFlagMaskCommand` from CoreGraphics.
const CG_FLAG_SHIFT: u32 = 1 << 17;
const CG_FLAG_CONTROL: u32 = 1 << 18;
const CG_FLAG_OPTION: u32 = 1 << 19;
const CG_FLAG_COMMAND: u32 = 1 << 20;

/// The modifier flag word of one macro step, as `CGEventSetFlags` reads it.
fn macro_flags(modifiers: &[Modifier]) -> u32 {
    modifiers.iter().fold(0, |flags, modifier| {
        flags
            | match modifier {
                Modifier::Command => CG_FLAG_COMMAND,
                Modifier::Control => CG_FLAG_CONTROL,
                Modifier::Option => CG_FLAG_OPTION,
                Modifier::Shift => CG_FLAG_SHIFT,
            }
    })
}

/// The fixed stdin payload of the macro replay: one JSON object per step, in
/// order. A validated macro always has a virtual key code per step; without
/// one the step encodes as `null` so the script rejects the whole list.
fn macro_payload(steps: &[MacroStep]) -> Result<Vec<u8>, String> {
    let items = steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let code = crate::profile::virtual_key_code(&step.key).ok_or_else(|| {
                format!("macro step {}: the key has no virtual key code", index + 1)
            })?;
            Ok(serde_json::json!({
                "code": code,
                "flags": macro_flags(&step.modifiers),
                "delay": step.delay_before_ms,
                "hold": step.hold_ms,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    serde_json::to_vec(&items).map_err(|error| format!("could not encode the macro: {error}"))
}

/// The total wall-clock time of a macro's waits and holds.
fn macro_duration(steps: &[MacroStep]) -> Duration {
    steps
        .iter()
        .map(|step| {
            Duration::from_millis(u64::from(step.delay_before_ms) + u64::from(step.hold_ms))
        })
        .sum()
}

/// Describes a failed macro replay by fixed markers in its stderr, which is
/// otherwise reported only by size. A partially played macro may already have
/// taken effect; the error never implies a retry.
fn macro_failure(output: &ProcessOutput) -> String {
    let has = |marker: &str| {
        output
            .stderr
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
    };
    let cause = if has(MACRO_DENIED) {
        "macOS did not allow posting keyboard events. Allow the app that runs the service in System Settings > Privacy & Security > Accessibility, then try again".to_string()
    } else if has(MACRO_CHECK_FAILED) {
        "this macOS did not accept the keyboard event layout, so no keys were sent".to_string()
    } else {
        format!("stderr: {} bytes, not shown", output.stderr.len())
    };
    format!("{OSASCRIPT}: {} ({cause})", output.status)
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

/// Executes validated actions on macOS. Shortcut and media-key execution may
/// require the user's Accessibility and Automation permissions for the
/// invoking application.
///
/// Failed or timed-out actions are returned to the caller and are never retried;
/// an action may already have taken effect before its process reports a failure.
pub struct MacOsDispatcher {
    runner: Box<dyn ProcessRunner>,
    sleeper: Box<dyn Sleeper>,
    /// The loopback OBS client for `obs_scene`, when this run has an endpoint.
    obs: Option<ObsClient>,
    /// Set once a media key has asked macOS for the Accessibility prompt;
    /// later media keys only check the permission (see [`MEDIA_KEY_SCRIPT`]).
    media_access_requested: bool,
}

impl MacOsDispatcher {
    /// Creates a dispatcher without launching subprocesses or requesting permissions.
    pub fn new() -> Self {
        Self {
            runner: Box::new(SystemProcessRunner { cancellation: None }),
            sleeper: Box::new(SystemSleeper),
            obs: None,
            media_access_requested: false,
        }
    }

    /// Creates a dispatcher that stops starting actions when the shared flag is set.
    ///
    /// Cancellation also stops monitoring an in-flight system-tool process and
    /// attempts bounded kill/reap cleanup. An action may already have taken effect.
    pub fn with_cancellation(cancellation: Arc<AtomicBool>) -> Self {
        Self {
            runner: Box::new(SystemProcessRunner {
                cancellation: Some(cancellation),
            }),
            sleeper: Box::new(SystemSleeper),
            obs: None,
            media_access_requested: false,
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

    /// Queries current output volume and mute state through bounded subprocesses.
    ///
    /// This performs no setter operations. A fixed-volume output can return an
    /// unsupported value; malformed or unavailable state is reported as an error.
    pub fn audio_snapshot(&mut self) -> Result<AudioSnapshot, String> {
        Ok(AudioSnapshot {
            output_volume: self.output_volume()?,
            output_muted: self.output_muted()?,
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
            .apple_script(format!(
                "set volume output volume {}",
                snapshot.output_volume
            ))
            .map_err(|error| format!("restoring output volume failed: {error}"));
        let muted = self
            .apple_script(format!("set volume output muted {}", snapshot.output_muted))
            .map_err(|error| format!("restoring output mute failed: {error}"));
        match (volume, muted) {
            (Ok(_), Ok(_)) => Ok(()),
            (Err(error), Ok(_)) | (Ok(_), Err(error)) => Err(error),
            (Err(volume), Err(muted)) => Err(format!("{volume}; {muted}")),
        }
    }

    fn run(&mut self, program: &'static str, args: Vec<String>) -> Result<Vec<u8>, String> {
        self.run_request(ProcessRequest {
            program,
            args,
            ..ProcessRequest::default()
        })
    }

    fn run_request(&mut self, request: ProcessRequest) -> Result<Vec<u8>, String> {
        self.run_checked(request).map(|output| output.stdout)
    }

    /// [`Self::run_checked`] whose failure message gives only the size of
    /// stderr, never its text, for tools that receive user text in argv.
    fn run_quiet(&mut self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        self.run_checked_with(request, false)
    }

    /// Runs one bounded request and fails on a nonzero exit; the successful
    /// output keeps stderr for callers that report diagnostics.
    fn run_checked(&mut self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        self.run_checked_with(request, true)
    }

    fn run_checked_with(
        &mut self,
        request: ProcessRequest,
        stderr_in_errors: bool,
    ) -> Result<ProcessOutput, String> {
        let program = request.program;
        let output = self.run_bounded(request)?;
        if !output.success {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let details = stderr.trim();
            return Err(if details.is_empty() {
                format!("{program}: {}", output.status)
            } else if !stderr_in_errors {
                format!(
                    "{program}: {} (stderr: {} bytes, not shown)",
                    output.status,
                    output.stderr.len()
                )
            } else {
                format!("{program}: {}: {details}", output.status)
            });
        }
        Ok(output)
    }

    /// Runs one request within the timeout and output limits; the output is
    /// returned whatever the exit status.
    fn run_bounded(&mut self, request: ProcessRequest) -> Result<ProcessOutput, String> {
        let program = request.program;
        check_cancellation(self.runner.cancellation(), program)?;
        if request
            .stdin
            .as_ref()
            .is_some_and(|input| input.len() > STDIN_LIMIT)
        {
            return Err(format!("{program}: stdin exceeds {STDIN_LIMIT} bytes"));
        }
        // Only the fixed timer constants of this module set their own bound.
        let timeout = if request.timeout.is_zero() {
            PROCESS_TIMEOUT
        } else {
            request.timeout
        };
        let output = self.runner.run(&ProcessRequest {
            timeout,
            output_limit: OUTPUT_LIMIT,
            ..request
        })?;
        if output.stdout.len() > OUTPUT_LIMIT || output.stderr.len() > OUTPUT_LIMIT {
            return Err(format!(
                "{program}: subprocess output exceeded {OUTPUT_LIMIT} bytes"
            ));
        }
        Ok(output)
    }

    fn apple_script(&mut self, script: String) -> Result<Vec<u8>, String> {
        self.run(OSASCRIPT, vec!["-e".into(), script])
    }

    fn pause(&mut self, duration: Duration) -> Result<(), String> {
        self.sleeper.sleep(duration, self.runner.cancellation())
    }

    fn set_clipboard(&mut self, text: Vec<u8>) -> Result<(), String> {
        self.run_request(ProcessRequest {
            program: PBCOPY,
            stdin: Some(text),
            env: UTF8_ENV,
            ..ProcessRequest::default()
        })
        .map(drop)
    }

    /// Returns the current clipboard text when it is small, plain text only.
    ///
    /// Any failure other than cancellation means "do not restore": the snippet
    /// is still pasted, and the clipboard keeps it afterwards.
    fn plain_text_clipboard(&mut self) -> Result<Option<Vec<u8>>, String> {
        let info = match self.run_request(ProcessRequest {
            program: OSASCRIPT,
            args: vec!["-e".into(), "clipboard info".into()],
            env: UTF8_ENV,
            ..ProcessRequest::default()
        }) {
            Ok(info) => info,
            Err(error) => {
                check_cancellation(self.runner.cancellation(), OSASCRIPT).map_err(|_| error)?;
                return Ok(None);
            }
        };
        if has_sensitive_clipboard_marker(&info) || !is_plain_text_clipboard(&info) {
            return Ok(None);
        }
        match self.run_request(ProcessRequest {
            program: PBPASTE,
            args: vec!["-Prefer".into(), "txt".into()],
            env: UTF8_ENV,
            ..ProcessRequest::default()
        }) {
            Ok(text)
                if !text.is_empty()
                    && text.len() <= STDIN_LIMIT
                    && std::str::from_utf8(&text).is_ok() =>
            {
                Ok(Some(text))
            }
            Ok(_) => Ok(None),
            Err(error) => {
                check_cancellation(self.runner.cancellation(), PBPASTE).map_err(|_| error)?;
                Ok(None)
            }
        }
    }

    /// Restores `saved` only while the clipboard still holds exactly `snippet`,
    /// so anything copied after the snippet was placed is never overwritten.
    ///
    /// A failed re-read (other than cancellation) skips the restore: the
    /// clipboard contents cannot be confirmed, so they are left alone.
    fn restore_clipboard_if_unchanged(
        &mut self,
        snippet: &[u8],
        saved: Vec<u8>,
    ) -> Result<(), String> {
        match self.run_request(ProcessRequest {
            program: PBPASTE,
            args: vec!["-Prefer".into(), "txt".into()],
            env: UTF8_ENV,
            ..ProcessRequest::default()
        }) {
            Ok(current) if current == snippet => self.set_clipboard(saved),
            Ok(_) => Ok(()),
            Err(error) => {
                check_cancellation(self.runner.cancellation(), PBPASTE).map_err(|_| error)
            }
        }
    }

    fn paste_text(&mut self, text: &str, press_enter: bool) -> Result<(), String> {
        let saved = self.plain_text_clipboard()?;
        self.set_clipboard(text.as_bytes().to_vec())?;
        // Fixed script lines only; the snippet travels through pbcopy's stdin.
        let mut lines = vec![
            "tell application \"System Events\"",
            "keystroke \"v\" using {command down}",
        ];
        if press_enter {
            lines.push("key code 36");
        }
        lines.push("end tell");
        let pasted = self.run(
            OSASCRIPT,
            lines
                .into_iter()
                .flat_map(|line| ["-e".to_string(), line.to_string()])
                .collect(),
        );
        let Some(saved) = saved else {
            return pasted.map(drop);
        };
        if let Err(error) = pasted {
            // Nothing pasted, so restore immediately; report both outcomes.
            return Err(
                match self.restore_clipboard_if_unchanged(text.as_bytes(), saved) {
                    Ok(()) => error,
                    Err(restore) => {
                        format!("{error}; restoring the previous clipboard text failed: {restore}")
                    }
                },
            );
        }
        self.pause(CLIPBOARD_RESTORE_DELAY)
            .and_then(|()| self.restore_clipboard_if_unchanged(text.as_bytes(), saved))
            .map_err(|error| {
                format!(
                    "text was pasted, but restoring the previous clipboard text failed: {error}"
                )
            })
    }

    fn execute(&mut self, action: &Action) -> Result<(), String> {
        match action {
            Action::Volume { delta } => {
                let current = self.output_volume()?;
                let target = (i32::from(current) + i32::from(*delta)).clamp(0, 100);
                self.apple_script(format!("set volume output volume {target}"))?;
            }
            Action::SetVolume { percent } => {
                self.apple_script(format!("set volume output volume {percent}"))?;
            }
            Action::SetMicInputVolume { percent } => {
                self.apple_script(format!("set volume input volume {percent}"))?;
            }
            Action::VolumeFromPosition
            | Action::MicInputVolumeFromPosition
            | Action::ObsInputVolumeFromPosition { .. } => {
                unreachable!("unresolved templates are rejected before host operations")
            }
            Action::ToggleMute => {
                let target = !self.output_muted()?;
                self.apple_script(format!("set volume output muted {target}"))?;
            }
            Action::Shortcut { key, modifiers } => {
                if let Some(command) = crate::profile::media_key_command(key) {
                    // The validator refuses modifiers on a media key, so the
                    // event flags the script builds are exactly its own.
                    debug_assert!(
                        modifiers.is_empty(),
                        "a media key cannot combine with modifiers"
                    );
                    self.media(command)?;
                } else {
                    self.apple_script(keystroke_script(key, modifiers))?;
                }
            }
            Action::LaunchApp { bundle_id } => {
                self.run(OPEN, vec!["-b".into(), bundle_id.clone()])?;
            }
            Action::OpenUrl { url } => {
                // The validated URL starts with http:// or https://, so it cannot be
                // read as an option; -u forces URL handling even if it matches a path.
                self.run(OPEN, vec!["-u".into(), url.clone()])?;
            }
            Action::Media { command } => self.media(*command)?,
            Action::System { command } => match command {
                SystemCommand::Undo => {
                    self.apple_script(keystroke_script("z", &[Modifier::Command]))?;
                }
                SystemCommand::Redo => {
                    self.apple_script(keystroke_script(
                        "z",
                        &[Modifier::Command, Modifier::Shift],
                    ))?;
                }
                SystemCommand::Screenshot => {
                    self.run(SCREENCAPTURE, vec!["-c".into()])?;
                }
            },
            Action::Text { text, press_enter } => self.paste_text(text, *press_enter)?,
            Action::Macro { steps } => self.macro_keys(steps)?,
            Action::Profile { .. } => {
                // The service never owns the profile list; `process_work`
                // reports the request to a session manager instead.
                return Err(
                    "profile switching is handled by the OpenStreamHub session manager, not the macOS backend; no action was taken"
                        .into(),
                );
            }
            Action::Timer { .. } => {
                // The countdown belongs to the service worker (`process_work`),
                // which calls `timer_alert` only when it ends.
                return Err(
                    "timer countdowns are run by the service worker, not the macOS backend; no action was taken"
                        .into(),
                );
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
            Action::MultiAction { actions } => {
                let deadline = self.sleeper.now() + MULTI_ACTION_DEADLINE;
                self.run_steps(actions, "multi_action", deadline)?;
            }
            Action::Conditional {
                when,
                then,
                otherwise,
            } => {
                let deadline = self.sleeper.now() + MULTI_ACTION_DEADLINE;
                let holds = if when.uses_obs() {
                    self.condition_holds(*when, deadline)
                } else {
                    self.system_condition_holds(*when)
                }
                .map_err(|error| {
                    format!(
                        "conditional could not read {}: {error}; neither branch ran",
                        when.name()
                    )
                })?;
                let (branch, label) = if holds {
                    (then, "conditional then")
                } else {
                    (otherwise, "conditional else")
                };
                self.run_steps(branch, label, deadline)?;
            }
        }
        Ok(())
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
                self.pause(STEP_SPACING).map_err(|error| {
                    format!(
                        "{label} stopped before step {} of {count}: {error}",
                        index + 1
                    )
                })?;
            }
            if self.sleeper.now() >= deadline {
                return Err(format!(
                    "{label} stopped before step {} of {count} ({}): the sequence exceeded its {} s overall deadline; earlier steps already took effect",
                    index + 1,
                    step.type_name(),
                    MULTI_ACTION_DEADLINE.as_secs()
                ));
            }
            // An OBS step gets only what is left of the sequence's deadline,
            // so a slow OBS cannot push the sequence past it.
            let result = if step.uses_obs() {
                self.obs_action(step, Some(deadline))
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

    /// Reads a condition about the default output from macOS.
    fn system_condition_holds(
        &mut self,
        condition: crate::profile::Condition,
    ) -> Result<bool, String> {
        use crate::profile::Condition;
        match condition {
            Condition::OutputMuted(expected) => Ok(self.output_muted()? == expected),
            Condition::OutputVolumeAtLeast(percent) => Ok(self.output_volume()? >= percent),
            _ => Err("this condition is read from OBS".into()),
        }
    }

    /// Reads a condition from OBS within what is left of `deadline`.
    fn condition_holds(
        &self,
        condition: crate::profile::Condition,
        deadline: std::time::Instant,
    ) -> Result<bool, String> {
        let obs = self.obs.as_ref().ok_or(OBS_UNAVAILABLE)?;
        let remaining = deadline.saturating_duration_since(self.sleeper.now());
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
    fn obs_action(
        &self,
        action: &Action,
        deadline: Option<std::time::Instant>,
    ) -> Result<(), String> {
        let obs = self.obs.as_ref().ok_or(OBS_UNAVAILABLE)?;
        let Some(deadline) = deadline else {
            return obs.run(action);
        };
        let remaining = deadline.saturating_duration_since(self.sleeper.now());
        if remaining.is_zero() {
            return Err(format!(
                "the multi_action's {} s overall deadline has passed; OBS was not contacted",
                MULTI_ACTION_DEADLINE.as_secs()
            ));
        }
        obs.clone().with_timeout(remaining).run(action)
    }

    /// Posts a media key, or stops Music and Spotify, through one fixed
    /// `osascript` JavaScript run (see [`MEDIA_KEY_SCRIPT`] and
    /// [`MEDIA_STOP_SCRIPT`]). No profile text reaches the process: the only
    /// argv item beyond the fixed lines is a key type from a closed set. A
    /// failure names a fixed cause and never contains stderr text.
    fn media(&mut self, command: MediaCommand) -> Result<(), String> {
        let (script, key_type) = match command {
            MediaCommand::PlayPause => (MEDIA_KEY_SCRIPT, Some(NX_KEYTYPE_PLAY)),
            MediaCommand::NextTrack => (MEDIA_KEY_SCRIPT, Some(NX_KEYTYPE_NEXT)),
            MediaCommand::PreviousTrack => (MEDIA_KEY_SCRIPT, Some(NX_KEYTYPE_PREVIOUS)),
            MediaCommand::Stop => (MEDIA_STOP_SCRIPT, None),
        };
        let mut args = vec!["-l".to_string(), "JavaScript".to_string()];
        for line in script {
            args.extend(["-e".to_string(), line.to_string()]);
        }
        if let Some(key_type) = key_type {
            // `--` ends osascript's options; the script checks both items again.
            let access = if std::mem::replace(&mut self.media_access_requested, true) {
                "check"
            } else {
                "request"
            };
            args.extend(["--".to_string(), key_type.to_string(), access.to_string()]);
        }
        let output = self.run_bounded(ProcessRequest {
            program: OSASCRIPT,
            args,
            ..ProcessRequest::default()
        })?;
        if output.success {
            Ok(())
        } else {
            Err(media_failure(&output))
        }
    }

    /// Replays a `macro` through one fixed `osascript` JavaScript run (see
    /// [`MACRO_SCRIPT`]). The steps travel on stdin as validated integers and
    /// the subprocess timeout is the macro's own duration plus a margin, so a
    /// long macro neither blocks forever nor rushes its pauses. A failure
    /// names a fixed cause and never contains stderr text; the keystrokes up
    /// to the failure may already have taken effect.
    fn macro_keys(&mut self, steps: &[MacroStep]) -> Result<(), String> {
        let mut args = vec!["-l".to_string(), "JavaScript".to_string()];
        for line in MACRO_SCRIPT {
            args.extend(["-e".to_string(), line.to_string()]);
        }
        let output = self.run_bounded(ProcessRequest {
            program: OSASCRIPT,
            args,
            stdin: Some(macro_payload(steps)?),
            timeout: macro_duration(steps) + MACRO_TIMEOUT_MARGIN,
            ..ProcessRequest::default()
        })?;
        if output.success {
            Ok(())
        } else {
            Err(macro_failure(&output))
        }
    }

    fn query(&mut self, property: &str) -> Result<String, String> {
        let output = self.apple_script(format!("{property} of (get volume settings)"))?;
        let text = String::from_utf8(output)
            .map_err(|_| format!("{OSASCRIPT}: invalid UTF-8 in {property} response"))?;
        Ok(text.trim().to_owned())
    }

    fn output_volume(&mut self) -> Result<u8, String> {
        let response = self.query("output volume")?;
        response
            .parse::<u8>()
            .ok()
            .filter(|value| *value <= 100)
            .ok_or_else(|| format!("{OSASCRIPT}: invalid output volume response {response:?}"))
    }

    fn output_muted(&mut self) -> Result<bool, String> {
        let response = self.query("output muted")?;
        match response.as_str() {
            "true" => Ok(true),
            "false" => Ok(false),
            _ => Err(format!(
                "{OSASCRIPT}: invalid output muted response {response:?}"
            )),
        }
    }
}

impl Default for MacOsDispatcher {
    fn default() -> Self {
        Self::new()
    }
}

impl ActionDispatcher for MacOsDispatcher {
    fn dispatch(&mut self, action: &Action) -> Result<(), String> {
        // Validate even when the caller did not use the profile loader. This
        // also validates every multi_action step before the first one runs.
        action.validate_executable()?;
        self.execute(action)
    }

    fn unavailable(&self, action: &Action) -> Option<&'static str> {
        (self.obs.is_none() && action.uses_obs()).then_some(OBS_UNAVAILABLE)
    }

    /// Posts a notification (3 s bound), then plays the sound (6 s bound), each
    /// in its own subprocess that is killed and reaped on timeout or
    /// cancellation. Both are attempted independently and their errors
    /// combined; neither is retried. `osascript` exits 0 even when macOS
    /// refuses the notification (for example "NSNotificationCenter connection
    /// invalid"), so a successful run that printed to stderr is returned as a
    /// fixed-category warning. Neither warnings nor errors ever contain stderr
    /// text, which could echo the label.
    fn timer_alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        alert.validate()?;
        let mut args = Vec::new();
        for line in TIMER_NOTIFICATION_SCRIPT {
            args.extend(["-e".to_string(), line.to_string()]);
        }
        // `--` ends osascript's options; the fixed title comes first, so the
        // message is never read as an option even if it starts with `-`.
        args.extend(["--".to_string(), TIMER_TITLE.to_string(), alert.message()]);
        let notification = self
            .run_quiet(ProcessRequest {
                program: OSASCRIPT,
                args,
                env: UTF8_ENV,
                timeout: TIMER_NOTIFICATION_TIMEOUT,
                ..ProcessRequest::default()
            })
            .map(|output| notification_warning(&output.stderr))
            .map_err(|error| format!("timer notification failed: {error}"));
        let sound = match alert.sound.path() {
            None => Ok(()),
            Some(path) => self
                .run_quiet(ProcessRequest {
                    program: AFPLAY,
                    args: vec![path.to_string()],
                    timeout: TIMER_SOUND_TIMEOUT,
                    ..ProcessRequest::default()
                })
                .map(drop)
                .map_err(|error| format!("timer sound failed: {error}")),
        };
        match (notification, sound) {
            (Ok(warning), Ok(())) => Ok(warning),
            (Err(error), Ok(())) | (Ok(_), Err(error)) => Err(error),
            (Err(notification), Err(sound)) => Err(format!("{notification}; {sound}")),
        }
    }

    /// A second system dispatcher cancelled by `cancel`, for an alert thread.
    /// It runs only `osascript` and `afplay`, so it gets no OBS client.
    fn timer_alerter(&mut self, cancel: Arc<AtomicBool>) -> Option<Box<dyn TimerAlerter>> {
        Some(Box::new(MacOsDispatcher {
            runner: self.runner.fork(cancel)?,
            sleeper: Box::new(SystemSleeper),
            obs: None,
            media_access_requested: false,
        }))
    }
}

impl TimerAlerter for MacOsDispatcher {
    fn alert(&mut self, alert: &TimerAlert) -> Result<Option<String>, String> {
        self.timer_alert(alert)
    }
}

/// Classifies the stderr of a notification that exited 0 into a fixed
/// warning. No stderr text is ever returned: a tool could echo the label in
/// some altered form (case, whitespace, normalization, invisible characters)
/// that no redaction reliably catches, so only a known marker is recognized
/// and anything else is reported by its byte count.
fn notification_warning(stderr: &[u8]) -> Option<String> {
    const UNAVAILABLE: &[u8] = b"NSNotificationCenter connection invalid";
    if stderr.iter().all(u8::is_ascii_whitespace) {
        return None;
    }
    let category = if stderr
        .windows(UNAVAILABLE.len())
        .any(|window| window == UNAVAILABLE)
    {
        "notification service unavailable (NSNotificationCenter connection invalid)".to_string()
    } else {
        format!(
            "the notification tool wrote to stderr ({} bytes)",
            stderr.len()
        )
    };
    Some(format!(
        "timer notification may not have been shown: {category}"
    ))
}

/// Describes a failed media script by fixed markers in its stderr, which is
/// otherwise reported only by size.
fn media_failure(output: &ProcessOutput) -> String {
    let has = |marker: &str| {
        output
            .stderr
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
    };
    // Each player's own result decides its hint, so an Automation denial of
    // one player is never attributed to the other.
    let failed_players: Vec<_> = MEDIA_STOP_PLAYERS
        .into_iter()
        .filter(|player| has(&format!("media-stop: {player} failed")))
        .map(|player| {
            let detail = if has(&format!("media-stop: {player} failed (-1743)")) {
                "macOS denied control of the player; allow it in System Settings > Privacy & Security > Automation"
            } else if has(&format!("media-stop: {player} failed (no reply)")) {
                "no answer: the player did not accept the command, or the Automation permission for it is missing"
            } else {
                "failed"
            };
            format!("{player} ({detail})")
        })
        .collect();
    let cause = if has(MEDIA_KEY_DENIED) {
        "macOS did not allow posting the media key. Allow the app that runs the service in System Settings > Privacy & Security > Accessibility, then try again".to_string()
    } else if has(MEDIA_KEY_CHECK_FAILED) {
        "this macOS did not accept the media-key event layout, so no media key was sent".to_string()
    } else if !failed_players.is_empty() {
        format!(
            "stop failed for {}; any other running player may already have stopped",
            failed_players.join(", ")
        )
    } else {
        format!("stderr: {} bytes, not shown", output.stderr.len())
    };
    format!("{OSASCRIPT}: {} ({cause})", output.status)
}

/// Builds a System Events keystroke from a validated key. A literal key is one
/// ASCII alphanumeric character or space and cannot introduce AppleScript
/// syntax; a named key is sent as an integer from the closed key-code table.
fn keystroke_script(key: &str, modifiers: &[Modifier]) -> String {
    let mut script = match crate::profile::named_key_code(key) {
        Some(code) => format!("tell application \"System Events\" to key code {code}"),
        None => {
            debug_assert!(
                key.len() == 1 && (key.as_bytes()[0].is_ascii_alphanumeric() || key == " ")
            );
            format!("tell application \"System Events\" to keystroke \"{key}\"")
        }
    };
    if !modifiers.is_empty() {
        let names = modifiers
            .iter()
            .map(|modifier| match modifier {
                Modifier::Command => "command down",
                Modifier::Control => "control down",
                Modifier::Option => "option down",
                Modifier::Shift => "shift down",
            })
            .collect::<Vec<_>>()
            .join(", ");
        script.push_str(&format!(" using {{{names}}}"));
    }
    script
}

/// Accepts `clipboard info` output listing only plain-text types, such as
/// `«class utf8», 5, «class ut16», 12, string, 5, Unicode text, 10`.
fn is_plain_text_clipboard(info: &[u8]) -> bool {
    let Ok(info) = std::str::from_utf8(info) else {
        return false;
    };
    let items: Vec<_> = info.trim().split(", ").collect();
    !info.trim().is_empty()
        && items.len() % 2 == 0
        && items.chunks(2).all(|pair| {
            PLAIN_TEXT_TYPES.contains(&pair[0])
                && pair[1]
                    .parse::<usize>()
                    .is_ok_and(|size| size <= 2 * STDIN_LIMIT)
        })
}

/// Rejects a clipboard whose `clipboard info` names any pasteboard-marker type,
/// such as `org.nspasteboard.ConcealedType` or `TransientType`, regardless of
/// whether every other listed type is plain text. Whether AppleScript lists
/// these markers at all is unverified, so this is a best-effort extra check.
fn has_sensitive_clipboard_marker(info: &[u8]) -> bool {
    let info = String::from_utf8_lossy(info).to_lowercase();
    ["nspasteboard", "concealed", "transient"]
        .iter()
        .any(|marker| info.contains(marker))
}

/// Waits between steps and reads the time; injected so tests never sleep.
trait Sleeper: Send {
    /// Monotonic time used for the `multi_action` deadline.
    fn now(&self) -> std::time::Instant {
        std::time::Instant::now()
    }

    fn sleep(
        &mut self,
        duration: Duration,
        cancellation: Option<&AtomicBool>,
    ) -> Result<(), String>;
}

struct SystemSleeper;

impl Sleeper for SystemSleeper {
    fn sleep(
        &mut self,
        duration: Duration,
        cancellation: Option<&AtomicBool>,
    ) -> Result<(), String> {
        let deadline = std::time::Instant::now() + duration;
        loop {
            check_cancellation(cancellation, "action pause")?;
            let now = std::time::Instant::now();
            if now >= deadline {
                return Ok(());
            }
            std::thread::sleep((deadline - now).min(Duration::from_millis(10)));
        }
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ProcessRequest {
    program: &'static str,
    args: Vec<String>,
    /// Bytes written to stdin before monitoring; `None` connects /dev/null.
    stdin: Option<Vec<u8>>,
    /// Fixed variables added to the inherited environment.
    env: &'static [(&'static str, &'static str)],
    timeout: Duration,
    output_limit: usize,
}

struct ProcessOutput {
    success: bool,
    status: String,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

trait ProcessRunner: Send {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String>;

    fn cancellation(&self) -> Option<&AtomicBool> {
        None
    }

    /// An independent system runner cancelled by `cancellation`, if supported.
    fn fork(&self, cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        let _ = cancellation;
        None
    }
}

struct SystemProcessRunner {
    cancellation: Option<Arc<AtomicBool>>,
}

impl SystemProcessRunner {
    fn fork_runner(cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        Some(Box::new(Self {
            cancellation: Some(cancellation),
        }))
    }
}

fn check_cancellation(cancellation: Option<&AtomicBool>, program: &str) -> Result<(), String> {
    if cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)) {
        Err(format!("{program}: action execution cancelled"))
    } else {
        Ok(())
    }
}

#[cfg(not(target_os = "macos"))]
impl ProcessRunner for SystemProcessRunner {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
        check_cancellation(self.cancellation(), request.program)?;
        Err("macOS action execution is unsupported on this platform".into())
    }

    fn cancellation(&self) -> Option<&AtomicBool> {
        self.cancellation.as_deref()
    }

    fn fork(&self, cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        Self::fork_runner(cancellation)
    }
}

#[cfg(target_os = "macos")]
impl ProcessRunner for SystemProcessRunner {
    fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
        use std::process::{Command, Stdio};
        check_cancellation(self.cancellation(), request.program)?;
        let mut child = Command::new(request.program)
            .args(&request.args)
            .envs(request.env.iter().copied())
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| format!("{}: could not start process: {error}", request.program))?;
        let result = write_child_stdin(&mut child, request)
            .and_then(|()| collect_child_output(&mut child, request, self.cancellation()));
        if let Err(error) = result {
            // Stop the direct system-tool process on all monitoring failures. No retry
            // is safe: a partially completed action could otherwise execute twice.
            return match stop_child(&mut child) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(format!(
                    "{error}; subprocess cleanup failed: {cleanup_error}"
                )),
            };
        }
        result
    }

    fn cancellation(&self) -> Option<&AtomicBool> {
        self.cancellation.as_deref()
    }

    fn fork(&self, cancellation: Arc<AtomicBool>) -> Option<Box<dyn ProcessRunner>> {
        Self::fork_runner(cancellation)
    }
}

#[cfg(target_os = "macos")]
fn write_child_stdin(
    child: &mut std::process::Child,
    request: &ProcessRequest,
) -> Result<(), String> {
    use std::io::Write;
    let Some(input) = &request.stdin else {
        return Ok(());
    };
    // Callers bound input below the pipe buffer, so this write cannot block.
    // Dropping the pipe afterwards signals end of input.
    let mut pipe = child
        .stdin
        .take()
        .ok_or_else(|| format!("{}: missing subprocess stdin pipe", request.program))?;
    pipe.write_all(input)
        .map_err(|error| format!("{}: could not write stdin: {error}", request.program))
}

#[cfg(target_os = "macos")]
fn stop_child(child: &mut std::process::Child) -> Result<(), String> {
    use std::time::Instant;
    if child
        .try_wait()
        .map_err(|error| error.to_string())?
        .is_some()
    {
        return Ok(());
    }
    if let Err(error) = child.kill() {
        // The process may have exited between the status check and kill.
        if child
            .try_wait()
            .map_err(|error| error.to_string())?
            .is_none()
        {
            return Err(format!("could not kill process: {error}"));
        }
        return Ok(());
    }
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return Ok(()),
            Err(error) => return Err(format!("could not reap killed process: {error}")),
            Ok(None) => {
                if started.elapsed() >= Duration::from_secs(1) {
                    return Err("killed process did not exit within 1000 ms".into());
                }
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[cfg(target_os = "macos")]
fn collect_child_output(
    child: &mut std::process::Child,
    request: &ProcessRequest,
    cancellation: Option<&AtomicBool>,
) -> Result<ProcessOutput, String> {
    use std::time::Instant;
    let mut stdout_pipe = child
        .stdout
        .take()
        .ok_or("missing subprocess stdout pipe")?;
    let mut stderr_pipe = child
        .stderr
        .take()
        .ok_or("missing subprocess stderr pipe")?;
    make_nonblocking(&stdout_pipe)?;
    make_nonblocking(&stderr_pipe)?;
    let start = Instant::now();
    collect_output(
        &mut stdout_pipe,
        &mut stderr_pipe,
        request,
        || {
            child
                .try_wait()
                .map(|status| status.map(|status| (status.success(), status.to_string())))
                .map_err(|error| error.to_string())
        },
        || start.elapsed(),
        || std::thread::sleep(Duration::from_millis(10)),
        || cancellation.is_some_and(|flag| flag.load(Ordering::Acquire)),
    )
}

#[cfg(any(target_os = "macos", test))]
fn collect_output(
    stdout_pipe: &mut impl std::io::Read,
    stderr_pipe: &mut impl std::io::Read,
    request: &ProcessRequest,
    mut poll_status: impl FnMut() -> Result<Option<(bool, String)>, String>,
    mut elapsed: impl FnMut() -> Duration,
    mut pause: impl FnMut(),
    mut cancelled: impl FnMut() -> bool,
) -> Result<ProcessOutput, String> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_closed = false;
    let mut stderr_closed = false;
    loop {
        if cancelled() {
            return Err(format!(
                "{}: action execution cancelled; action may already have taken effect",
                request.program
            ));
        }
        if !stdout_closed {
            stdout_closed = drain_pipe(stdout_pipe, &mut stdout, request.output_limit)
                .map_err(|error| format!("{}: stdout: {error}", request.program))?;
        }
        if !stderr_closed {
            stderr_closed = drain_pipe(stderr_pipe, &mut stderr, request.output_limit)
                .map_err(|error| format!("{}: stderr: {error}", request.program))?;
        }
        if cancelled() {
            return Err(format!(
                "{}: action execution cancelled; action may already have taken effect",
                request.program
            ));
        }
        if let Some((success, status)) = poll_status()
            .map_err(|error| format!("{}: could not inspect process: {error}", request.program))?
            && stdout_closed
            && stderr_closed
        {
            return Ok(ProcessOutput {
                success,
                status,
                stdout,
                stderr,
            });
        }
        if elapsed() >= request.timeout {
            return Err(format!(
                "{}: subprocess timed out after {} ms; action may already have taken effect",
                request.program,
                request.timeout.as_millis()
            ));
        }
        pause();
    }
}

#[cfg(target_os = "macos")]
fn make_nonblocking(pipe: &impl std::os::fd::AsRawFd) -> Result<(), String> {
    let fd = pipe.as_raw_fd();
    // SAFETY: fd belongs to a live child pipe. These fcntl operations read and
    // modify only its status flags and do not take ownership of the descriptor.
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(format!(
            "could not make subprocess pipe nonblocking: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "macos", test))]
fn drain_pipe(
    pipe: &mut impl std::io::Read,
    collected: &mut Vec<u8>,
    limit: usize,
) -> Result<bool, String> {
    use std::io::ErrorKind;
    let mut buffer = [0; 1024];
    // A read budget also bounds repeated Interrupted responses so the caller
    // regains control to check its deadline even under continuous signals.
    for _ in 0..32 {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(count) => {
                if count > limit.saturating_sub(collected.len()) {
                    return Err(format!("subprocess output exceeded {limit} bytes"));
                }
                collected.extend_from_slice(&buffer[..count]);
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == ErrorKind::Interrupted => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ProfileDirection;
    use std::{collections::VecDeque, sync::Mutex};

    #[derive(Default)]
    struct FakeState {
        requests: Vec<ProcessRequest>,
        results: VecDeque<Result<ProcessOutput, String>>,
        /// Pauses, recorded after the number of requests already started.
        pauses: Vec<(usize, Duration)>,
        /// Virtual time since the dispatcher was built; pauses advance it.
        elapsed: Duration,
        /// Virtual time each subprocess takes.
        request_cost: Duration,
    }
    struct FakeSleeper(Arc<Mutex<FakeState>>, std::time::Instant);
    impl Sleeper for FakeSleeper {
        fn sleep(&mut self, duration: Duration, _: Option<&AtomicBool>) -> Result<(), String> {
            let mut state = self.0.lock().unwrap();
            let started = state.requests.len();
            state.pauses.push((started, duration));
            state.elapsed += duration;
            Ok(())
        }

        fn now(&self) -> std::time::Instant {
            self.1 + self.0.lock().unwrap().elapsed
        }
    }
    struct FakeRunner(Arc<Mutex<FakeState>>);
    impl ProcessRunner for FakeRunner {
        fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
            let mut state = self.0.lock().unwrap();
            state.requests.push(ProcessRequest {
                program: request.program,
                args: request.args.clone(),
                stdin: request.stdin.clone(),
                env: request.env,
                timeout: request.timeout,
                output_limit: request.output_limit,
            });
            let cost = state.request_cost;
            state.elapsed += cost;
            state.results.pop_front().expect("unexpected subprocess")
        }
    }

    fn success(stdout: &str) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            success: true,
            status: "exit status: 0".into(),
            stdout: stdout.as_bytes().to_vec(),
            stderr: Vec::new(),
        })
    }

    fn fake(
        results: Vec<Result<ProcessOutput, String>>,
    ) -> (MacOsDispatcher, Arc<Mutex<FakeState>>) {
        let state = Arc::new(Mutex::new(FakeState {
            results: results.into(),
            ..FakeState::default()
        }));
        let dispatcher = MacOsDispatcher {
            runner: Box::new(FakeRunner(state.clone())),
            sleeper: Box::new(FakeSleeper(state.clone(), std::time::Instant::now())),
            obs: None,
            media_access_requested: false,
        };
        (dispatcher, state)
    }

    #[test]
    fn volume_queries_before_setting_and_clamps_both_edges() {
        for (initial, delta, target) in [("98\n", 5, 100), ("2\n", -5, 0), ("40\n", 4, 44)] {
            let (mut dispatcher, state) = fake(vec![success(initial), success("")]);
            dispatcher.dispatch(&Action::Volume { delta }).unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 2);
            assert_eq!(state.requests[0].program, OSASCRIPT);
            assert_eq!(
                state.requests[0].args,
                ["-e", "output volume of (get volume settings)"]
            );
            assert_eq!(
                state.requests[1].args,
                [
                    "-e".to_string(),
                    format!("set volume output volume {target}")
                ]
            );
            assert!(
                state
                    .requests
                    .iter()
                    .all(|request| request.timeout == PROCESS_TIMEOUT
                        && request.output_limit == OUTPUT_LIMIT)
            );
        }
    }

    #[test]
    fn absolute_volume_sets_the_resolved_value_without_query_or_delta() {
        for percent in [0, 1, 50, 100] {
            let (mut dispatcher, state) = fake(vec![success("")]);
            dispatcher.dispatch(&Action::SetVolume { percent }).unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 1);
            assert_eq!(state.requests[0].program, OSASCRIPT);
            assert_eq!(
                state.requests[0].args,
                [
                    "-e".to_string(),
                    format!("set volume output volume {percent}")
                ]
            );
        }
    }

    #[test]
    fn microphone_volume_sets_the_input_volume_only() {
        for percent in [0, 1, 50, 100] {
            let (mut dispatcher, state) = fake(vec![success("")]);
            dispatcher
                .dispatch(&Action::SetMicInputVolume { percent })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 1);
            assert_eq!(state.requests[0].program, OSASCRIPT);
            assert_eq!(
                state.requests[0].args,
                [
                    "-e".to_string(),
                    format!("set volume input volume {percent}")
                ]
            );
        }
    }

    #[test]
    fn unresolved_fader_template_or_invalid_absolute_volume_never_starts_host_tools() {
        let (mut dispatcher, state) = fake(vec![]);
        for action in [
            Action::VolumeFromPosition,
            Action::MicInputVolumeFromPosition,
            Action::SetMicInputVolume { percent: 101 },
            Action::SetVolume { percent: 101 },
        ] {
            assert!(dispatcher.dispatch(&action).is_err());
        }
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn invalid_volume_response_stops_before_setting() {
        for response in ["missing value", "101", "-1", "50.1", "0\n100"] {
            let (mut dispatcher, state) = fake(vec![success(response)]);
            assert!(
                dispatcher
                    .dispatch(&Action::Volume { delta: 1 })
                    .unwrap_err()
                    .contains("invalid output volume")
            );
            assert_eq!(state.lock().unwrap().requests.len(), 1);
        }
    }

    #[test]
    fn toggle_mute_reads_and_inverts_only_boolean_responses() {
        for (current, target) in [("true\n", "false"), ("false\n", "true")] {
            let (mut dispatcher, state) = fake(vec![success(current), success("")]);
            dispatcher.dispatch(&Action::ToggleMute).unwrap();
            let state = state.lock().unwrap();
            assert_eq!(
                state.requests[0].args[1],
                "output muted of (get volume settings)"
            );
            assert_eq!(
                state.requests[1].args[1],
                format!("set volume output muted {target}")
            );
        }
        let (mut dispatcher, state) = fake(vec![success("missing value")]);
        assert!(dispatcher.dispatch(&Action::ToggleMute).is_err());
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn shortcuts_use_literal_validated_keys_and_modifier_names() {
        let (mut dispatcher, state) = fake(vec![success(""), success("")]);
        dispatcher
            .dispatch(&Action::Shortcut {
                key: "s".into(),
                modifiers: vec![
                    Modifier::Command,
                    Modifier::Control,
                    Modifier::Option,
                    Modifier::Shift,
                ],
            })
            .unwrap();
        dispatcher
            .dispatch(&Action::Shortcut {
                key: " ".into(),
                modifiers: vec![],
            })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(
            state.requests[0].args[1],
            "tell application \"System Events\" to keystroke \"s\" using {command down, control down, option down, shift down}"
        );
        assert_eq!(
            state.requests[1].args[1],
            "tell application \"System Events\" to keystroke \" \""
        );
    }

    #[test]
    fn launch_app_uses_bundle_id_argument_without_shell() {
        let (mut dispatcher, state) = fake(vec![success("")]);
        dispatcher
            .dispatch(&Action::LaunchApp {
                bundle_id: "com.apple.TextEdit".into(),
            })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests[0].program, OPEN);
        assert_eq!(state.requests[0].args, ["-b", "com.apple.TextEdit"]);
    }

    #[test]
    fn media_key_shortcuts_post_the_hardware_media_key_script() {
        for (key, key_type) in [
            ("play_pause", "16"),
            ("next_track", "17"),
            ("previous_track", "18"),
        ] {
            let (mut dispatcher, state) = fake(vec![success("")]);
            dispatcher
                .dispatch(&Action::Shortcut {
                    key: key.into(),
                    modifiers: vec![],
                })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 1);
            assert_eq!(state.requests[0].program, OSASCRIPT);
            assert_eq!(state.requests[0].args[0], "-l");
            assert_eq!(*state.requests[0].args.last().unwrap(), "request");
            // The first media key of a dispatcher asks macOS for its prompt.
            assert!(state.requests[0].args.contains(&key_type.to_string()));
        }
        // With modifiers the action is refused before any subprocess starts.
        let (mut dispatcher, state) = fake(vec![]);
        assert!(
            dispatcher
                .dispatch(&Action::Shortcut {
                    key: "play_pause".into(),
                    modifiers: vec![Modifier::Command],
                })
                .is_err()
        );
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn macro_replays_one_fixed_script_with_the_steps_on_stdin() {
        let (mut dispatcher, state) = fake(vec![success("")]);
        dispatcher
            .dispatch(&Action::Macro {
                steps: vec![
                    MacroStep {
                        key: "c".into(),
                        modifiers: vec![Modifier::Command],
                        delay_before_ms: 0,
                        hold_ms: 80,
                    },
                    MacroStep {
                        key: "arrow_left".into(),
                        modifiers: vec![Modifier::Shift, Modifier::Option],
                        delay_before_ms: 120,
                        hold_ms: 0,
                    },
                ],
            })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 1);
        let request = &state.requests[0];
        assert_eq!(request.program, OSASCRIPT);
        assert_eq!(request.args[0], "-l");
        assert_eq!(request.args[1], "JavaScript");
        assert_eq!(request.args[2], "-e");
        // One `-e` item per script line, then nothing else: the steps are on stdin.
        assert_eq!(request.args.len(), 2 + 2 * MACRO_SCRIPT.len());
        let payload: serde_json::Value =
            serde_json::from_slice(request.stdin.as_deref().unwrap()).unwrap();
        assert_eq!(
            payload,
            serde_json::json!([
                {"code": 0x08, "flags": 0x100000, "delay": 0, "hold": 80},
                {"code": 123, "flags": 0x0A0000, "delay": 120, "hold": 0},
            ])
        );
        // The timeout is the macro's own duration (80 + 120 ms) plus the margin.
        assert_eq!(
            request.timeout,
            Duration::from_millis(200) + MACRO_TIMEOUT_MARGIN
        );
    }

    #[test]
    fn macro_failures_name_the_fixed_permission_and_layout_causes() {
        let denied = Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: b"script error: Error: macro: not allowed to post events.\n".to_vec(),
        });
        let (mut dispatcher, _state) = fake(vec![denied]);
        let error = dispatcher
            .dispatch(&Action::Macro {
                steps: vec![MacroStep {
                    key: "a".into(),
                    modifiers: vec![],
                    delay_before_ms: 0,
                    hold_ms: 0,
                }],
            })
            .unwrap_err();
        assert!(
            error.contains("System Settings > Privacy & Security > Accessibility"),
            "{error}"
        );
        assert!(!error.contains("script error"), "{error}");
        let checked = Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: b"script error: Error: macro: event check failed.\n".to_vec(),
        });
        let (mut dispatcher, _state) = fake(vec![checked]);
        let error = dispatcher
            .dispatch(&Action::Macro {
                steps: vec![MacroStep {
                    key: "a".into(),
                    modifiers: vec![],
                    delay_before_ms: 0,
                    hold_ms: 0,
                }],
            })
            .unwrap_err();
        assert!(error.contains("keyboard event layout"), "{error}");
        // Any other stderr is reported by size only.
        let other = Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: b"script error: something quoted \"a b\"\n".to_vec(),
        });
        let (mut dispatcher, _state) = fake(vec![other]);
        let error = dispatcher
            .dispatch(&Action::Macro {
                steps: vec![MacroStep {
                    key: "a".into(),
                    modifiers: vec![],
                    delay_before_ms: 0,
                    hold_ms: 0,
                }],
            })
            .unwrap_err();
        assert!(error.contains("stderr: "), "{error}");
        assert!(!error.contains("quoted"), "{error}");
    }

    /// Runs the fixed script against stdin that the script must refuse before
    /// it touches any permission, so the JSON plumbing is exercised on this
    /// macOS without posting anything. Shape errors carry a `macro:` marker;
    /// unparsable input fails in JSON.parse itself.
    #[test]
    fn macro_script_rejects_bad_stdin_before_posting_anything() {
        for input in [
            "[]",
            "[{\"code\":8,\"flags\":0,\"delay\":0,\"hold\":80,\"x\":1}]",
            "[{\"code\":8,\"flags\":1048577,\"delay\":0,\"hold\":80}]",
            "[{\"code\":128,\"flags\":0,\"delay\":0,\"hold\":80}]",
        ] {
            let output = std::process::Command::new(OSASCRIPT)
                .arg("-l")
                .arg("JavaScript")
                .args(MACRO_SCRIPT.iter().flat_map(|line| ["-e", line]))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .and_then(|mut child| {
                    use std::io::Write;
                    child.stdin.take().unwrap().write_all(input.as_bytes())?;
                    child.wait_with_output()
                })
                .unwrap();
            assert!(!output.status.success(), "{input}");
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains("macro:"), "{input}: {stderr}");
            assert!(!stderr.contains(MACRO_DENIED), "{input}: {stderr}");
        }
        let output = std::process::Command::new(OSASCRIPT)
            .arg("-l")
            .arg("JavaScript")
            .args(MACRO_SCRIPT.iter().flat_map(|line| ["-e", line]))
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                child.stdin.take().unwrap().write_all(b"not json")?;
                child.wait_with_output()
            })
            .unwrap();
        assert!(!output.status.success());
    }

    #[test]
    fn invalid_actions_never_start_subprocesses() {
        let actions = [
            Action::Volume { delta: 0 },
            Action::Volume { delta: 101 },
            Action::Shortcut {
                key: "\" & do shell script \"bad".into(),
                modifiers: vec![],
            },
            Action::Shortcut {
                key: "s".into(),
                modifiers: vec![Modifier::Command, Modifier::Command],
            },
            Action::LaunchApp {
                bundle_id: "-a TextEdit".into(),
            },
        ];
        let (mut dispatcher, state) = fake(vec![]);
        for action in actions {
            assert!(dispatcher.dispatch(&action).is_err());
        }
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn exit_stderr_and_timeout_are_propagated_without_retry() {
        let (mut dispatcher, state) = fake(vec![Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: b"System Events is not allowed assistive access (-1719)\n".to_vec(),
        })]);
        let action = Action::Shortcut {
            key: "s".into(),
            modifiers: vec![Modifier::Command],
        };
        let error = dispatcher.dispatch(&action).unwrap_err();
        assert!(error.contains("exit status: 1"));
        assert!(error.contains("not allowed assistive access (-1719)"));
        assert_eq!(state.lock().unwrap().requests.len(), 1);
        let (mut dispatcher, state) = fake(vec![Err("subprocess timed out after 5000 ms".into())]);
        assert_eq!(
            dispatcher.dispatch(&action).unwrap_err(),
            "subprocess timed out after 5000 ms"
        );
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn excessive_output_fails_before_any_followup_command() {
        let (mut dispatcher, state) = fake(vec![success(&"9".repeat(OUTPUT_LIMIT + 1))]);
        assert!(
            dispatcher
                .dispatch(&Action::Volume { delta: 1 })
                .unwrap_err()
                .contains("output exceeded")
        );
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn drain_pipe_is_bounded_and_preserves_exact_bytes() {
        let mut data = std::io::Cursor::new(b"hello".to_vec());
        let mut collected = Vec::new();
        assert!(drain_pipe(&mut data, &mut collected, 5).unwrap());
        assert_eq!(collected, b"hello");
        let mut data = std::io::Cursor::new(vec![0; 4096]);
        let mut collected = Vec::new();
        assert!(
            drain_pipe(&mut data, &mut collected, 1100)
                .unwrap_err()
                .contains("output exceeded")
        );
        assert!(collected.len() <= 1100);
    }

    #[test]
    fn drain_pipe_handles_would_block_and_read_failure() {
        struct Reader(std::io::ErrorKind);
        impl std::io::Read for Reader {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(self.0, "fake read result"))
            }
        }
        assert!(
            !drain_pipe(
                &mut Reader(std::io::ErrorKind::WouldBlock),
                &mut Vec::new(),
                8
            )
            .unwrap()
        );
        assert!(
            drain_pipe(
                &mut Reader(std::io::ErrorKind::BrokenPipe),
                &mut Vec::new(),
                8
            )
            .unwrap_err()
            .contains("fake read result")
        );
    }

    #[test]
    fn monitoring_times_out_for_a_running_process_or_unclosed_pipe() {
        use std::cell::Cell;
        struct OpenPipe;
        impl std::io::Read for OpenPipe {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::WouldBlock.into())
            }
        }
        for exited in [false, true] {
            let clock = Cell::new(Duration::ZERO);
            let request = ProcessRequest {
                program: OSASCRIPT,
                args: vec![],
                timeout: PROCESS_TIMEOUT,
                output_limit: OUTPUT_LIMIT,
                ..ProcessRequest::default()
            };
            let error = collect_output(
                &mut OpenPipe,
                &mut std::io::empty(),
                &request,
                || Ok(exited.then(|| (true, "exit status: 0".into()))),
                || clock.get(),
                || clock.set(clock.get() + Duration::from_secs(1)),
                || false,
            )
            .err()
            .unwrap();
            assert!(error.contains("timed out after 5000 ms"));
            assert_eq!(clock.get(), PROCESS_TIMEOUT);
        }
    }

    #[test]
    fn monitoring_collects_both_pipes_and_returns_exit_status() {
        let request = ProcessRequest {
            program: OSASCRIPT,
            args: vec![],
            timeout: PROCESS_TIMEOUT,
            output_limit: OUTPUT_LIMIT,
            ..ProcessRequest::default()
        };
        let result = collect_output(
            &mut std::io::Cursor::new(b"result"),
            &mut std::io::Cursor::new(b"permission denied"),
            &request,
            || Ok(Some((false, "exit status: 1".into()))),
            || Duration::ZERO,
            || panic!("completed process should not pause"),
            || false,
        )
        .unwrap();
        assert!(!result.success);
        assert_eq!(result.status, "exit status: 1");
        assert_eq!(result.stdout, b"result");
        assert_eq!(result.stderr, b"permission denied");
    }

    #[test]
    fn repeated_interrupted_reads_return_control_to_deadline_checker() {
        struct Interrupted;
        impl std::io::Read for Interrupted {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::Interrupted.into())
            }
        }
        assert!(!drain_pipe(&mut Interrupted, &mut Vec::new(), OUTPUT_LIMIT).unwrap());
    }

    #[test]
    fn dispatcher_can_move_to_an_action_worker() {
        fn assert_send<T: Send>() {}
        assert_send::<MacOsDispatcher>();
    }

    #[test]
    fn cancellation_prevents_process_start_without_real_actions() {
        let cancellation = Arc::new(AtomicBool::new(true));
        let mut dispatcher = MacOsDispatcher::with_cancellation(cancellation.clone());
        let error = dispatcher.run("/not/an/executable", vec![]).unwrap_err();
        assert!(error.contains("action execution cancelled"));
        let mut runner = SystemProcessRunner {
            cancellation: Some(cancellation),
        };
        let error = runner
            .run(&ProcessRequest {
                program: "/not/an/executable",
                args: vec![],
                timeout: PROCESS_TIMEOUT,
                output_limit: OUTPUT_LIMIT,
                ..ProcessRequest::default()
            })
            .err()
            .unwrap();
        assert!(error.contains("action execution cancelled"));
    }

    #[test]
    fn cancellation_between_query_and_setter_prevents_the_setter() {
        struct CancellingRunner {
            cancellation: Arc<AtomicBool>,
            requests: Arc<Mutex<Vec<ProcessRequest>>>,
        }
        impl ProcessRunner for CancellingRunner {
            fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
                self.requests.lock().unwrap().push(ProcessRequest {
                    program: request.program,
                    args: request.args.clone(),
                    stdin: request.stdin.clone(),
                    env: request.env,
                    timeout: request.timeout,
                    output_limit: request.output_limit,
                });
                self.cancellation.store(true, Ordering::Release);
                success("40")
            }

            fn cancellation(&self) -> Option<&AtomicBool> {
                Some(&self.cancellation)
            }
        }
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = MacOsDispatcher {
            runner: Box::new(CancellingRunner {
                cancellation: Arc::new(AtomicBool::new(false)),
                requests: requests.clone(),
            }),
            sleeper: Box::new(SystemSleeper),
            obs: None,
            media_access_requested: false,
        };
        assert!(
            dispatcher
                .dispatch(&Action::Volume { delta: 5 })
                .unwrap_err()
                .contains("action execution cancelled")
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].args[1],
            "output volume of (get volume settings)"
        );
    }

    #[test]
    fn audio_snapshot_queries_both_fields_without_setters() {
        let (mut dispatcher, requests) = fake(vec![success("40\n"), success("true\n")]);
        assert_eq!(
            dispatcher.audio_snapshot().unwrap(),
            AudioSnapshot {
                output_volume: 40,
                output_muted: true,
            }
        );
        let requests = requests.lock().unwrap();
        assert_eq!(requests.requests.len(), 2);
        assert_eq!(
            requests.requests[0].args[1],
            "output volume of (get volume settings)"
        );
        assert_eq!(
            requests.requests[1].args[1],
            "output muted of (get volume settings)"
        );
    }

    #[test]
    fn restore_snapshot_validates_before_any_process_request() {
        let (mut dispatcher, requests) = fake(vec![]);
        for volume in [101, 255] {
            assert!(
                dispatcher
                    .restore_audio_snapshot(&AudioSnapshot {
                        output_volume: volume,
                        output_muted: false,
                    })
                    .is_err()
            );
        }
        assert!(requests.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn restore_snapshot_sets_exact_volume_and_mute_independently() {
        let (mut dispatcher, requests) = fake(vec![
            Err("synthetic volume failure".into()),
            Err("synthetic mute failure".into()),
        ]);
        let error = dispatcher
            .restore_audio_snapshot(&AudioSnapshot {
                output_volume: 0,
                output_muted: true,
            })
            .unwrap_err();
        assert!(error.contains("restoring output volume failed: synthetic volume failure"));
        assert!(error.contains("restoring output mute failed: synthetic mute failure"));
        let requests = requests.lock().unwrap();
        assert_eq!(requests.requests.len(), 2);
        assert_eq!(requests.requests[0].args[1], "set volume output volume 0");
        assert_eq!(requests.requests[1].args[1], "set volume output muted true");
    }

    #[test]
    fn restore_snapshot_does_not_bypass_cancellation() {
        struct CancelledRunner(AtomicBool);
        impl ProcessRunner for CancelledRunner {
            fn run(&mut self, _: &ProcessRequest) -> Result<ProcessOutput, String> {
                panic!("cancelled restoration must not reach subprocess runner");
            }

            fn cancellation(&self) -> Option<&AtomicBool> {
                Some(&self.0)
            }
        }
        let mut dispatcher = MacOsDispatcher {
            runner: Box::new(CancelledRunner(AtomicBool::new(true))),
            sleeper: Box::new(SystemSleeper),
            obs: None,
            media_access_requested: false,
        };
        let error = dispatcher
            .restore_audio_snapshot(&AudioSnapshot {
                output_volume: 50,
                output_muted: false,
            })
            .unwrap_err();
        assert!(error.contains("restoring output volume failed"));
        assert!(error.contains("restoring output mute failed"));
        assert!(error.contains("action execution cancelled"));
    }

    #[test]
    fn monitoring_cancels_a_running_process_before_timeout() {
        use std::cell::Cell;
        struct OpenPipe;
        impl std::io::Read for OpenPipe {
            fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::ErrorKind::WouldBlock.into())
            }
        }
        let cancellation = AtomicBool::new(false);
        let polls = Cell::new(0);
        let elapsed = Cell::new(Duration::ZERO);
        let error = collect_output(
            &mut OpenPipe,
            &mut std::io::empty(),
            &ProcessRequest {
                program: OSASCRIPT,
                args: vec![],
                timeout: PROCESS_TIMEOUT,
                output_limit: OUTPUT_LIMIT,
                ..ProcessRequest::default()
            },
            || {
                polls.set(polls.get() + 1);
                Ok(None)
            },
            || elapsed.get(),
            || {
                elapsed.set(Duration::from_millis(10));
                cancellation.store(true, Ordering::Release);
            },
            || cancellation.load(Ordering::Acquire),
        )
        .err()
        .unwrap();
        assert!(error.contains("action execution cancelled"));
        assert!(error.contains("action may already have taken effect"));
        assert_eq!(polls.get(), 1);
        assert_eq!(elapsed.get(), Duration::from_millis(10));
    }

    fn failure(stderr: &str) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    fn args(request: &ProcessRequest) -> Vec<&str> {
        request.args.iter().map(String::as_str).collect()
    }

    const PLAIN_INFO: &str = "«class utf8», 3, «class ut16», 8, string, 3, Unicode text, 6\n";
    const PASTE_ARGS: [&str; 6] = [
        "-e",
        "tell application \"System Events\"",
        "-e",
        "keystroke \"v\" using {command down}",
        "-e",
        "end tell",
    ];

    #[test]
    fn open_url_passes_the_url_as_one_argument_to_open() {
        let url = "https://example.com/a?b=1&c=it's#frag";
        let (mut dispatcher, state) = fake(vec![success("")]);
        dispatcher
            .dispatch(&Action::OpenUrl { url: url.into() })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 1);
        assert_eq!(state.requests[0].program, OPEN);
        assert_eq!(args(&state.requests[0]), ["-u", url]);
        assert_eq!(state.requests[0].stdin, None);
        assert_eq!(state.requests[0].timeout, PROCESS_TIMEOUT);
    }

    #[test]
    fn named_keys_use_closed_integer_key_codes() {
        let cases = [
            (
                "arrow_left",
                vec![Modifier::Command, Modifier::Shift],
                "tell application \"System Events\" to key code 123 using {command down, shift down}",
            ),
            (
                "f20",
                vec![],
                "tell application \"System Events\" to key code 90",
            ),
            (
                "return",
                vec![Modifier::Option],
                "tell application \"System Events\" to key code 36 using {option down}",
            ),
            (
                "space",
                vec![Modifier::Control],
                "tell application \"System Events\" to key code 49 using {control down}",
            ),
            (
                "page_down",
                vec![],
                "tell application \"System Events\" to key code 121",
            ),
        ];
        let (mut dispatcher, state) = fake(cases.iter().map(|_| success("")).collect());
        for (key, modifiers, _) in &cases {
            dispatcher
                .dispatch(&Action::Shortcut {
                    key: (*key).into(),
                    modifiers: modifiers.clone(),
                })
                .unwrap();
        }
        let state = state.lock().unwrap();
        for (request, (_, _, script)) in state.requests.iter().zip(&cases) {
            assert_eq!(request.program, OSASCRIPT);
            assert_eq!(args(request), ["-e", script]);
        }
    }

    #[test]
    fn profile_actions_never_reach_the_host() {
        let (mut dispatcher, state) = fake(vec![]);
        for direction in [ProfileDirection::Next, ProfileDirection::Previous] {
            let error = dispatcher
                .dispatch(&Action::Profile { direction })
                .unwrap_err();
            assert!(error.contains("session manager"), "{error}");
            assert!(error.contains("no action was taken"));
        }
        assert!(state.lock().unwrap().requests.is_empty());
    }

    fn alert(label: Option<&str>, sound: crate::TimerSound) -> TimerAlert {
        TimerAlert {
            seconds: 300,
            label: label.map(str::to_owned),
            sound,
        }
    }

    #[test]
    fn timer_presses_never_reach_the_host() {
        let (mut dispatcher, state) = fake(vec![]);
        let error = dispatcher
            .dispatch(&Action::Timer {
                seconds: 5,
                label: None,
                sound: None,
            })
            .unwrap_err();
        assert!(error.contains("service worker"), "{error}");
        assert!(error.contains("no action was taken"));
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn timer_alert_passes_hostile_labels_only_as_argv_after_a_fixed_script() {
        for label in [
            "\" & do shell script \"id\" & \"",
            "`id` $(id) ${HOME}",
            "'; quit; '",
            "-e display dialog \"x\"",
            "--",
            "end run",
            "zażółć 🎉 \\\"",
        ] {
            let (mut dispatcher, state) = fake(vec![success(""), success("")]);
            dispatcher
                .timer_alert(&alert(Some(label), crate::TimerSound::Ping))
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 2);
            let notification = &state.requests[0];
            assert_eq!(notification.program, OSASCRIPT);
            assert_eq!(
                args(notification),
                [
                    "-e",
                    "on run argv",
                    "-e",
                    "display notification (item 2 of argv) with title (item 1 of argv)",
                    "-e",
                    "end run",
                    "--",
                    "Timer finished",
                    label,
                ],
                "{label:?}"
            );
            // The script lines are the fixed constants; the label is only the last item.
            assert_eq!(
                notification.args[..6]
                    .iter()
                    .filter(|arg| *arg != "-e")
                    .collect::<Vec<_>>(),
                TIMER_NOTIFICATION_SCRIPT
            );
            assert_eq!(notification.args.last().unwrap(), label);
            assert_eq!(notification.stdin, None);
            assert_eq!(notification.timeout, TIMER_NOTIFICATION_TIMEOUT);
            let sound = &state.requests[1];
            assert_eq!(sound.program, AFPLAY);
            assert_eq!(args(sound), ["/System/Library/Sounds/Ping.aiff"]);
            assert_eq!(sound.timeout, TIMER_SOUND_TIMEOUT);
        }
    }

    #[test]
    fn timer_alert_labels_with_line_breaks_or_invisible_characters_never_start_processes() {
        let (mut dispatcher, state) = fake(vec![]);
        for label in [
            "a\nb",
            "a\rb",
            "\u{202E}gnp.exe",
            "tab\there",
            " padded",
            "",
        ] {
            assert!(
                dispatcher
                    .timer_alert(&alert(Some(label), crate::TimerSound::Glass))
                    .is_err(),
                "{label:?}"
            );
        }
        let mut too_long = alert(None, crate::TimerSound::Glass);
        too_long.seconds = 0;
        assert!(dispatcher.timer_alert(&too_long).is_err());
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn timer_alert_without_label_or_sound_uses_fixed_text_and_no_player() {
        let (mut dispatcher, state) = fake(vec![success("")]);
        dispatcher
            .timer_alert(&alert(None, crate::TimerSound::None))
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 1);
        assert_eq!(
            args(&state.requests[0])[6..],
            ["--", "Timer finished", "5:00 countdown ended"]
        );
    }

    #[test]
    fn every_timer_sound_maps_to_a_fixed_system_file() {
        for sound in crate::TimerSound::ALL {
            let (mut dispatcher, state) = fake(vec![success(""), success("")]);
            dispatcher.timer_alert(&alert(None, sound)).unwrap();
            let state = state.lock().unwrap();
            match sound.path() {
                None => assert_eq!(state.requests.len(), 1),
                Some(path) => {
                    assert!(path.starts_with("/System/Library/Sounds/") && path.ends_with(".aiff"));
                    assert_eq!(state.requests[1].program, AFPLAY);
                    assert_eq!(args(&state.requests[1]), [path]);
                }
            }
        }
    }

    #[test]
    fn timer_alert_timeouts_are_reported_and_the_sound_is_still_attempted() {
        let (mut dispatcher, state) = fake(vec![
            Err("/usr/bin/osascript: subprocess timed out after 3000 ms; action may already have taken effect".into()),
            failure("afplay: cannot open file"),
        ]);
        let error = dispatcher
            .timer_alert(&alert(Some("Tea"), crate::TimerSound::Hero))
            .unwrap_err();
        assert!(error.starts_with("timer notification failed: "), "{error}");
        assert!(error.contains("timed out after 3000 ms"));
        assert!(error.contains("timer sound failed: "));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 2, "no retry");
        assert_eq!(state.requests[0].timeout, TIMER_NOTIFICATION_TIMEOUT);
        assert_eq!(state.requests[1].timeout, TIMER_SOUND_TIMEOUT);
    }

    #[test]
    fn the_sound_bound_leaves_room_for_the_longest_system_sound() {
        // Funk.aiff, the longest closed-set file, takes about 2.9 s of afplay
        // wall time; the bound must not cut it short.
        assert!(TIMER_SOUND_TIMEOUT >= Duration::from_secs(6));
        assert!(TIMER_NOTIFICATION_TIMEOUT <= PROCESS_TIMEOUT);
    }

    fn warning_for(label: &str, stderr: Vec<u8>) -> Option<String> {
        let (mut dispatcher, _) = fake(vec![
            Ok(ProcessOutput {
                success: true,
                status: "exit status: 0".into(),
                stdout: Vec::new(),
                stderr,
            }),
            success(""),
        ]);
        dispatcher
            .timer_alert(&alert(Some(label), crate::TimerSound::Glass))
            .unwrap()
    }

    #[test]
    fn a_notification_warning_is_a_fixed_category_that_never_contains_stderr_text() {
        let label = "secret café";
        let unavailable = "timer notification may not have been shown: notification service unavailable (NSNotificationCenter connection invalid)";
        // Every way a tool could echo the label back must be reduced to a category.
        let echoes = [
            "secret café".to_string(),
            "secret\tcafé".into(),
            "secret\ncafé".into(),
            "secret  café".into(),
            "secret\u{200B}café".into(),
            "SECRET CAFÉ".into(),
            "secret cafe\u{301}".into(),
            "s\u{202E}éfac terces".into(),
            "\u{1b}[31msecret café\u{1b}[0m".into(),
        ];
        for echo in &echoes {
            for (stderr, expected) in [
                (
                    format!("osascript[1] NSNotificationCenter connection invalid for {echo}\n"),
                    unavailable.to_string(),
                ),
                (
                    format!("error: {echo}\n"),
                    format!(
                        "timer notification may not have been shown: the notification tool wrote to stderr ({} bytes)",
                        format!("error: {echo}\n").len()
                    ),
                ),
            ] {
                let warning = warning_for(label, stderr.into_bytes()).unwrap();
                assert_eq!(warning, expected, "{echo:?}");
                for piece in ["secret", "SECRET", "caf", "CAF", "terces"] {
                    assert!(!warning.contains(piece), "{echo:?} leaked {piece}");
                }
            }
        }
        // Hostile stderr: bidi controls, non-UTF-8 bytes, near the output limit.
        let mut hostile = b"\xff\xfe\x80 \xe2\x80\xae".to_vec();
        hostile.extend(std::iter::repeat_n(b'x', OUTPUT_LIMIT - 16));
        let warning = warning_for(label, hostile.clone()).unwrap();
        assert_eq!(
            warning,
            format!(
                "timer notification may not have been shown: the notification tool wrote to stderr ({} bytes)",
                hostile.len()
            )
        );
        assert!(warning.is_ascii());
        // The category is found even in non-UTF-8 output.
        let mut mixed = b"\xff".to_vec();
        mixed.extend(b"NSNotificationCenter connection invalid\x80");
        assert_eq!(warning_for(label, mixed).unwrap(), unavailable);
        // A failing tool's stderr never reaches the error either.
        let (mut dispatcher, _) = fake(vec![
            failure("execution error: Can’t get \"SECRET CAFÉ\". (-1728)"),
            failure("afplay: secret\tcafé"),
        ]);
        let error = dispatcher
            .timer_alert(&alert(Some(label), crate::TimerSound::Glass))
            .unwrap_err();
        assert!(error.contains("stderr: "), "{error}");
        for piece in ["secret", "SECRET", "caf", "CAF"] {
            assert!(!error.contains(piece), "{error}");
        }
        // Whitespace-only stderr is no warning.
        let (mut dispatcher, _) = fake(vec![
            Ok(ProcessOutput {
                success: true,
                status: "exit status: 0".into(),
                stdout: Vec::new(),
                stderr: b" \n\t".to_vec(),
            }),
            success(""),
        ]);
        assert_eq!(
            dispatcher
                .timer_alert(&alert(None, crate::TimerSound::Glass))
                .unwrap(),
            None
        );
    }

    #[test]
    fn the_alerter_is_cancelled_by_its_own_flag() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut dispatcher = MacOsDispatcher::with_cancellation(Arc::new(AtomicBool::new(false)));
        let mut alerter = dispatcher
            .timer_alerter(cancel.clone())
            .expect("system runner forks");
        cancel.store(true, Ordering::Release);
        let error = alerter
            .alert(&alert(Some("Tea"), crate::TimerSound::Glass))
            .unwrap_err();
        assert!(error.contains("action execution cancelled"), "{error}");
        // Test runners cannot fork, so their alerts stay inline.
        let (mut fake_dispatcher, _) = fake(vec![]);
        assert!(fake_dispatcher.timer_alerter(cancel).is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn cancellation_kills_and_reaps_a_running_child_within_the_budget() {
        let cancellation = Arc::new(AtomicBool::new(false));
        let mut runner = SystemProcessRunner {
            cancellation: Some(cancellation.clone()),
        };
        let flag = cancellation.clone();
        let stopper = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Release);
        });
        let started = std::time::Instant::now();
        // Stands in for a long afplay: only the cancellation can end it early.
        let error = runner
            .run(&ProcessRequest {
                program: "/bin/sleep",
                args: vec!["30".into()],
                timeout: TIMER_SOUND_TIMEOUT,
                output_limit: OUTPUT_LIMIT,
                ..ProcessRequest::default()
            })
            .err()
            .unwrap();
        stopper.join().unwrap();
        assert!(error.contains("action execution cancelled"), "{error}");
        assert!(!error.contains("cleanup failed"), "{error}");
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn cancelled_timer_alerts_start_no_process() {
        let mut dispatcher = MacOsDispatcher::with_cancellation(Arc::new(AtomicBool::new(true)));
        let error = dispatcher
            .timer_alert(&alert(Some("Tea"), crate::TimerSound::Glass))
            .unwrap_err();
        assert!(error.contains("timer notification failed"));
        assert!(error.contains("timer sound failed"));
        assert!(error.contains("action execution cancelled"));
    }

    #[test]
    fn obs_scene_without_an_endpoint_is_unavailable_and_starts_nothing() {
        let (mut dispatcher, state) = fake(vec![]);
        let scene = Action::ObsScene {
            scene: "Live".into(),
        };
        let multi = Action::MultiAction {
            actions: vec![Action::ToggleMute, scene.clone()],
        };
        assert_eq!(dispatcher.unavailable(&scene), Some(OBS_UNAVAILABLE));
        assert_eq!(dispatcher.unavailable(&multi), Some(OBS_UNAVAILABLE));
        assert_eq!(dispatcher.unavailable(&Action::ToggleMute), None);
        assert_eq!(dispatcher.dispatch(&scene).unwrap_err(), OBS_UNAVAILABLE);
        assert!(state.lock().unwrap().requests.is_empty());
    }

    /// A fake OBS on a loopback port that accepts the upgrade and answers one
    /// `SetCurrentProgramScene` for `scene`, calling `on_request` when the
    /// request arrives.
    fn fake_obs(
        scene: &'static str,
        on_request: impl FnOnce() + Send + 'static,
    ) -> (u16, std::thread::JoinHandle<()>) {
        fake_obs_checked(
            "SetCurrentProgramScene",
            move |request| assert_eq!(request["d"]["requestData"]["sceneName"], scene),
            on_request,
        )
    }

    /// Like [`fake_obs`] for any request type: `check` sees the request.
    fn fake_obs_checked(
        request_type: &'static str,
        check: impl FnOnce(&serde_json::Value) + Send + 'static,
        on_request: impl FnOnce() + Send + 'static,
    ) -> (u16, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut seen = Vec::new();
            let mut chunk = [0; 4096];
            while !seen.windows(4).any(|window| window == b"\r\n\r\n") {
                let read = stream.read(&mut chunk).unwrap();
                seen.extend_from_slice(&chunk[..read]);
            }
            let request = String::from_utf8(seen).unwrap();
            let key = request
                .lines()
                .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                .unwrap()
                .to_owned();
            stream
                .write_all(
                    format!(
                        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                        crate::obs::tests_support::accept(&key)
                    )
                    .as_bytes(),
                )
                .unwrap();
            let send = |stream: &mut std::net::TcpStream, text: String| {
                let mut frame = vec![0x81];
                if text.len() < 126 {
                    frame.push(text.len() as u8);
                } else {
                    frame.push(126);
                    frame.extend_from_slice(&(text.len() as u16).to_be_bytes());
                }
                frame.extend_from_slice(text.as_bytes());
                stream.write_all(&frame).unwrap();
            };
            send(&mut stream, r#"{"op":0,"d":{"rpcVersion":1}}"#.into());
            let identify = crate::obs::tests_support::read_text(&mut stream);
            assert!(identify.contains("\"eventSubscriptions\":0"));
            send(
                &mut stream,
                r#"{"op":2,"d":{"negotiatedRpcVersion":1}}"#.into(),
            );
            let request = crate::obs::tests_support::read_text(&mut stream);
            let request: serde_json::Value = serde_json::from_str(&request).unwrap();
            assert_eq!(request["d"]["requestType"], request_type);
            check(&request);
            on_request();
            send(
                &mut stream,
                serde_json::json!({"op":7,"d":{"requestType":request_type,"requestId":request["d"]["requestId"],"requestStatus":{"result":true,"code":100}}})
                    .to_string(),
            );
        });
        (port, server)
    }

    /// A fake OBS that serves one request per connection, `replies.len()`
    /// connections in a row. Each reply is a request type and the
    /// `responseData` to answer it with; the thread returns the request types
    /// it saw, in order.
    fn fake_obs_script(
        replies: Vec<(&'static str, serde_json::Value)>,
    ) -> (u16, std::thread::JoinHandle<Vec<String>>) {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let mut seen_types = Vec::new();
            for (request_type, data) in replies {
                let (mut stream, _) = listener.accept().unwrap();
                let mut seen = Vec::new();
                let mut chunk = [0; 4096];
                while !seen.windows(4).any(|window| window == b"\r\n\r\n") {
                    let read = stream.read(&mut chunk).unwrap();
                    seen.extend_from_slice(&chunk[..read]);
                }
                let request = String::from_utf8(seen).unwrap();
                let key = request
                    .lines()
                    .find_map(|line| line.strip_prefix("Sec-WebSocket-Key: "))
                    .unwrap()
                    .to_owned();
                stream
                    .write_all(
                        format!(
                            "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n",
                            crate::obs::tests_support::accept(&key)
                        )
                        .as_bytes(),
                    )
                    .unwrap();
                let send = |stream: &mut std::net::TcpStream, text: String| {
                    let mut frame = vec![0x81];
                    if text.len() < 126 {
                        frame.push(text.len() as u8);
                    } else {
                        frame.push(126);
                        frame.extend_from_slice(&(text.len() as u16).to_be_bytes());
                    }
                    frame.extend_from_slice(text.as_bytes());
                    stream.write_all(&frame).unwrap();
                };
                send(&mut stream, r#"{"op":0,"d":{"rpcVersion":1}}"#.into());
                let _identify = crate::obs::tests_support::read_text(&mut stream);
                send(
                    &mut stream,
                    r#"{"op":2,"d":{"negotiatedRpcVersion":1}}"#.into(),
                );
                let request = crate::obs::tests_support::read_text(&mut stream);
                let request: serde_json::Value = serde_json::from_str(&request).unwrap();
                assert_eq!(request["d"]["requestType"], request_type);
                seen_types.push(request_type.to_string());
                send(
                    &mut stream,
                    serde_json::json!({"op":7,"d":{"requestType":request_type,"requestId":request["d"]["requestId"],"requestStatus":{"result":true,"code":100},"responseData":data}})
                        .to_string(),
                );
            }
            seen_types
        });
        (port, server)
    }

    fn conditional(when: crate::profile::Condition) -> Action {
        let key = |key: &str| Action::Shortcut {
            key: key.into(),
            modifiers: vec![Modifier::Command],
        };
        Action::Conditional {
            when,
            then: vec![key("a")],
            otherwise: vec![key("b")],
        }
    }

    #[test]
    fn a_conditional_runs_the_branch_the_obs_state_picks() {
        use crate::profile::Condition;
        for (active, expected_key) in [(true, "a"), (false, "b")] {
            let (dispatcher, state) = fake(vec![success("")]);
            let (port, server) = fake_obs_script(vec![(
                "GetRecordStatus",
                serde_json::json!({"outputActive":active}),
            )]);
            let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
                crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
                None,
            )));
            let action = conditional(Condition::Recording(true));
            assert_eq!(dispatcher.unavailable(&action), None);
            dispatcher.dispatch(&action).unwrap();
            assert_eq!(server.join().unwrap(), ["GetRecordStatus"]);
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 1, "exactly one branch step ran");
            assert_eq!(
                args(&state.requests[0]),
                [
                    "-e",
                    format!(
                        "tell application \"System Events\" to keystroke \"{expected_key}\" using {{command down}}"
                    )
                    .as_str()
                ]
            );
        }
    }

    #[test]
    fn a_conditional_compares_with_the_value_it_was_written_with() {
        use crate::profile::Condition;
        // `false` holds while OBS is not streaming, so `then` runs.
        let (dispatcher, state) = fake(vec![success("")]);
        let (port, server) = fake_obs_script(vec![(
            "GetStreamStatus",
            serde_json::json!({"outputActive":false}),
        )]);
        let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        )));
        dispatcher
            .dispatch(&conditional(Condition::Streaming(false)))
            .unwrap();
        server.join().unwrap();
        let state = state.lock().unwrap();
        assert!(args(&state.requests[0])[1].contains("keystroke \"a\""));
    }

    #[test]
    fn a_conditional_without_an_else_does_nothing_when_the_condition_fails() {
        use crate::profile::Condition;
        let (dispatcher, state) = fake(vec![]);
        let (port, server) = fake_obs_script(vec![(
            "GetVirtualCamStatus",
            serde_json::json!({"outputActive":false}),
        )]);
        let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        )));
        let action = Action::Conditional {
            when: Condition::VirtualCamera(true),
            then: vec![Action::ToggleMute],
            otherwise: vec![],
        };
        dispatcher.dispatch(&action).unwrap();
        assert_eq!(server.join().unwrap(), ["GetVirtualCamStatus"]);
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn a_system_condition_needs_no_obs_and_reads_the_output() {
        use crate::profile::Condition;
        let key = |key: &str| Action::Shortcut {
            key: key.into(),
            modifiers: vec![],
        };
        // Muted: the output is read once, then the `then` branch runs.
        let (mut dispatcher, state) = fake(vec![success("true"), success("")]);
        let action = Action::Conditional {
            when: Condition::OutputMuted(true),
            then: vec![key("a")],
            otherwise: vec![key("b")],
        };
        assert_eq!(dispatcher.unavailable(&action), None);
        dispatcher.dispatch(&action).unwrap();
        {
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 2);
            assert_eq!(
                state.requests[0].args[1],
                "output muted of (get volume settings)"
            );
            assert!(state.requests[1].args[1].contains("keystroke \"a\""));
        }

        // Volume 40 is at least 40 but not at least 41.
        for (threshold, branch) in [(40, "a"), (41, "b")] {
            let (mut dispatcher, state) = fake(vec![success("40"), success("")]);
            dispatcher
                .dispatch(&Action::Conditional {
                    when: Condition::OutputVolumeAtLeast(threshold),
                    then: vec![key("a")],
                    otherwise: vec![key("b")],
                })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(
                state.requests[0].args[1],
                "output volume of (get volume settings)"
            );
            assert!(
                state.requests[1].args[1].contains(&format!("keystroke \"{branch}\"")),
                "threshold {threshold}"
            );
        }

        // An unreadable answer runs neither branch.
        let (mut dispatcher, state) = fake(vec![success("missing value")]);
        let error = dispatcher.dispatch(&action).unwrap_err();
        assert!(error.contains("could not read output_muted"), "{error}");
        assert!(error.contains("neither branch ran"), "{error}");
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn a_conditional_that_cannot_read_obs_runs_neither_branch() {
        use crate::profile::Condition;
        // No endpoint at all.
        let (mut dispatcher, state) = fake(vec![]);
        let action = conditional(Condition::Recording(true));
        assert_eq!(dispatcher.unavailable(&action), Some(OBS_UNAVAILABLE));
        let error = dispatcher.dispatch(&action).unwrap_err();
        assert!(error.contains("neither branch ran"), "{error}");
        assert!(state.lock().unwrap().requests.is_empty());

        // An endpoint nothing listens on.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        let (dispatcher, state) = fake(vec![]);
        let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        )));
        let error = dispatcher.dispatch(&action).unwrap_err();
        assert!(
            error.contains("could not read obs_recording") && error.contains("neither branch ran"),
            "{error}"
        );
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn a_conditional_branch_can_run_an_obs_step_after_the_read() {
        use crate::profile::{Condition, ObsRecordOp};
        let (mut dispatcher, _state) = fake(vec![]);
        let (port, server) = fake_obs_script(vec![
            ("GetRecordStatus", serde_json::json!({"outputActive":true})),
            ("StopRecord", serde_json::Value::Null),
        ]);
        dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        )));
        let action = Action::Conditional {
            when: Condition::Recording(true),
            then: vec![Action::ObsRecord {
                op: ObsRecordOp::Stop,
            }],
            otherwise: vec![Action::ObsRecord {
                op: ObsRecordOp::Start,
            }],
        };
        dispatcher.dispatch(&action).unwrap();
        assert_eq!(server.join().unwrap(), ["GetRecordStatus", "StopRecord"]);
    }

    #[test]
    fn obs_scene_with_an_endpoint_switches_through_the_client() {
        let (port, server) = fake_obs("Live", || {});
        let (dispatcher, state) = fake(vec![]);
        let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        )));
        let scene = Action::ObsScene {
            scene: "Live".into(),
        };
        assert_eq!(dispatcher.unavailable(&scene), None);
        dispatcher.dispatch(&scene).unwrap();
        server.join().unwrap();
        // No subprocess: OBS is reached over the loopback socket only.
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn obs_output_actions_send_their_request_without_request_data() {
        use crate::profile::{ObsOutputOp, ObsReplayBufferOp};
        let cases = [
            (
                Action::ObsVirtualCamera {
                    op: ObsOutputOp::Toggle,
                },
                "ToggleVirtualCam",
            ),
            (
                Action::ObsVirtualCamera {
                    op: ObsOutputOp::Start,
                },
                "StartVirtualCam",
            ),
            (
                Action::ObsVirtualCamera {
                    op: ObsOutputOp::Stop,
                },
                "StopVirtualCam",
            ),
            (
                Action::ObsReplayBuffer {
                    op: ObsReplayBufferOp::Toggle,
                },
                "ToggleReplayBuffer",
            ),
            (
                Action::ObsReplayBuffer {
                    op: ObsReplayBufferOp::Start,
                },
                "StartReplayBuffer",
            ),
            (
                Action::ObsReplayBuffer {
                    op: ObsReplayBufferOp::Stop,
                },
                "StopReplayBuffer",
            ),
            (
                Action::ObsReplayBuffer {
                    op: ObsReplayBufferOp::Save,
                },
                "SaveReplayBuffer",
            ),
        ];
        for (action, request_type) in cases {
            let (port, server) = fake_obs_checked(
                request_type,
                |request| assert!(request["d"].get("requestData").is_none(), "{request}"),
                || {},
            );
            let (dispatcher, state) = fake(vec![]);
            let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
                crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
                None,
            )));
            assert_eq!(dispatcher.unavailable(&action), None);
            dispatcher.dispatch(&action).unwrap();
            server.join().unwrap();
            assert!(state.lock().unwrap().requests.is_empty(), "{request_type}");
        }
    }

    #[test]
    fn obs_output_actions_without_an_endpoint_are_unavailable_and_start_nothing() {
        use crate::profile::{ObsOutputOp, ObsReplayBufferOp};
        let (dispatcher, state) = fake(vec![]);
        let mut dispatcher = dispatcher;
        for action in [
            Action::ObsVirtualCamera {
                op: ObsOutputOp::Toggle,
            },
            Action::ObsReplayBuffer {
                op: ObsReplayBufferOp::Save,
            },
            Action::MultiAction {
                actions: vec![
                    Action::ToggleMute,
                    Action::ObsReplayBuffer {
                        op: ObsReplayBufferOp::Save,
                    },
                ],
            },
        ] {
            assert_eq!(dispatcher.unavailable(&action), Some(OBS_UNAVAILABLE));
            // The caller (the service worker) refuses an unavailable
            // multi_action before any step runs; only single actions are
            // dispatched here.
            if !matches!(action, Action::MultiAction { .. }) {
                assert_eq!(dispatcher.dispatch(&action).unwrap_err(), OBS_UNAVAILABLE);
            }
        }
        assert!(state.lock().unwrap().requests.is_empty());
    }

    #[test]
    fn an_obs_output_step_runs_inside_a_multi_action_in_order() {
        use crate::profile::ObsReplayBufferOp;
        let (dispatcher, state) = fake(vec![success("")]);
        let seen = Arc::new(Mutex::new(None));
        let (shared, record) = (Arc::clone(&state), Arc::clone(&seen));
        let (port, server) = fake_obs_checked(
            "SaveReplayBuffer",
            |_| {},
            move || {
                *record.lock().unwrap() = Some(shared.lock().unwrap().requests.len());
            },
        );
        let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        )));
        dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::Shortcut {
                        key: "a".into(),
                        modifiers: vec![Modifier::Command],
                    },
                    Action::ObsReplayBuffer {
                        op: ObsReplayBufferOp::Save,
                    },
                ],
            })
            .unwrap();
        server.join().unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(1), "the shortcut ran first");
    }

    #[test]
    fn a_multi_action_runs_shortcut_obs_scene_shortcut_in_order() {
        let (dispatcher, state) = fake(vec![success(""), success("")]);
        // When OBS gets the request, only the first shortcut has run.
        let seen = Arc::new(Mutex::new(None));
        let (shared, record) = (Arc::clone(&state), Arc::clone(&seen));
        let (port, server) = fake_obs("Be right back", move || {
            *record.lock().unwrap() = Some(shared.lock().unwrap().requests.len());
        });
        let mut dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        )));
        let shortcut = |key: &str| Action::Shortcut {
            key: key.into(),
            modifiers: vec![Modifier::Command],
        };
        let sequence = Action::MultiAction {
            actions: vec![
                shortcut("a"),
                Action::ObsScene {
                    scene: "Be right back".into(),
                },
                shortcut("b"),
            ],
        };
        assert_eq!(dispatcher.unavailable(&sequence), None);
        dispatcher.dispatch(&sequence).unwrap();
        server.join().unwrap();
        assert_eq!(*seen.lock().unwrap(), Some(1));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 2, "the OBS step starts no subprocess");
        assert_eq!(
            args(&state.requests[0]),
            [
                "-e",
                "tell application \"System Events\" to keystroke \"a\" using {command down}"
            ]
        );
        assert_eq!(
            args(&state.requests[1]),
            [
                "-e",
                "tell application \"System Events\" to keystroke \"b\" using {command down}"
            ]
        );
        // Spacing runs between all three steps, the OBS step included.
        assert_eq!(state.pauses, [(1, STEP_SPACING), (1, STEP_SPACING)]);
    }

    #[test]
    fn an_obs_step_never_runs_past_the_multi_action_deadline() {
        use std::time::Instant;
        // A fake OBS that accepts and then never answers.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (release, wait) = std::sync::mpsc::channel::<()>();
        let server = std::thread::spawn(move || {
            let (_stream, _) = listener.accept().unwrap();
            let _ = wait.recv_timeout(Duration::from_secs(10));
        });
        let client = ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", port).unwrap(),
            None,
        );
        let (dispatcher, state) = fake(vec![success("")]);
        let mut dispatcher = dispatcher.with_obs(Some(client.clone()));
        // The first step uses up all but 200 ms of the 15 s budget (virtual
        // time: its cost plus the 280 ms spacing).
        state.lock().unwrap().request_cost =
            MULTI_ACTION_DEADLINE - STEP_SPACING - Duration::from_millis(200);
        let started = Instant::now();
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 50 },
                    Action::ObsScene {
                        scene: "Live".into(),
                    },
                ],
            })
            .unwrap_err();
        let elapsed = started.elapsed();
        release.send(()).unwrap();
        server.join().unwrap();
        assert!(error.contains("step 2 of 2 (obs_scene) failed"), "{error}");
        assert!(error.contains("timed out"), "{error}");
        assert!(
            elapsed >= Duration::from_millis(200) && elapsed < Duration::from_millis(1500),
            "the OBS step used {elapsed:?} instead of the 200 ms left"
        );

        // A deadline that has passed fails without any connection attempt.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        closed.set_nonblocking(true).unwrap();
        let (dispatcher, _) = fake(vec![]);
        let dispatcher = dispatcher.with_obs(Some(ObsClient::new(
            crate::obs::ObsEndpoint::new("127.0.0.1", closed.local_addr().unwrap().port()).unwrap(),
            None,
        )));
        let now = dispatcher.sleeper.now();
        let error = dispatcher
            .obs_action(
                &Action::ObsScene {
                    scene: "Live".into(),
                },
                Some(now),
            )
            .unwrap_err();
        assert!(error.contains("deadline has passed"), "{error}");
        assert!(closed.accept().is_err(), "OBS was contacted");
    }

    /// `-l JavaScript`, then each fixed script line as its own `-e` item.
    fn jxa(lines: &[&str]) -> Vec<String> {
        let mut args = vec!["-l".to_string(), "JavaScript".to_string()];
        for line in lines {
            args.extend(["-e".to_string(), line.to_string()]);
        }
        args
    }

    fn failed_with(stderr: &str) -> Result<ProcessOutput, String> {
        Ok(ProcessOutput {
            success: false,
            status: "exit status: 1".into(),
            stdout: Vec::new(),
            stderr: stderr.as_bytes().to_vec(),
        })
    }

    #[test]
    fn media_keys_run_one_fixed_script_with_a_closed_key_type() {
        for (command, key_type) in [
            (MediaCommand::PlayPause, "16"),
            (MediaCommand::NextTrack, "17"),
            (MediaCommand::PreviousTrack, "18"),
        ] {
            let (mut dispatcher, state) = fake(vec![success("")]);
            dispatcher.dispatch(&Action::Media { command }).unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 1, "{command:?}");
            let request = &state.requests[0];
            assert_eq!(request.program, OSASCRIPT);
            let mut expected = jxa(MEDIA_KEY_SCRIPT);
            // `--` ends osascript's options; a fresh dispatcher may ask
            // macOS to show its Accessibility prompt.
            expected.extend(["--".into(), key_type.into(), "request".into()]);
            assert_eq!(request.args, expected, "{command:?}");
            // Nothing but the fixed lines, `--`, one closed-set number and
            // one closed-set mode.
            assert_eq!(&request.args[..2], ["-l", "JavaScript"]);
            assert_eq!(request.stdin, None);
            assert!(request.env.is_empty());
            assert_eq!(request.timeout, PROCESS_TIMEOUT);
            assert_eq!(request.output_limit, OUTPUT_LIMIT);
            assert!(state.pauses.is_empty());
        }
        // The script posts a key-down and a key-up system-defined event
        // (type 14, subtype 8) and checks the permission first.
        let script = MEDIA_KEY_SCRIPT.join("\n");
        for needle in [
            "['16', '17', '18'].indexOf(argv[0])",
            "['request', 'check'].indexOf(argv[1])",
            "CGPreflightPostEventAccess",
            "if (argv[1] === 'request') $.CGRequestPostEventAccess();",
            "[0xA, 0xB].map",
            "CGEventSetType(event, 14)",
            "CGEventSetIntegerValueField(event, 83, 8)",
            "CGEventSetIntegerValueField(event, 149, data1)",
            "NSEvent.eventWithCGEvent(event)",
            "CGEventPost(0, event)",
            MEDIA_KEY_DENIED,
            MEDIA_KEY_CHECK_FAILED,
        ] {
            assert!(script.contains(needle), "missing {needle}");
        }
        assert_eq!(
            MEDIA_KEY_SCRIPT
                .iter()
                .filter(|line| line.contains("CGEventPost"))
                .count(),
            1
        );
    }

    #[test]
    fn media_keys_ask_for_the_accessibility_prompt_at_most_once_per_dispatcher() {
        let denied = "execution error: Error: Error: media-key: not allowed to post events (-2700)";
        let (mut dispatcher, state) = fake(vec![
            failed_with(denied),
            success("nothing to stop\n"),
            failed_with(denied),
            success(""),
        ]);
        for command in [
            MediaCommand::PlayPause,
            MediaCommand::Stop,
            MediaCommand::NextTrack,
            MediaCommand::PlayPause,
        ] {
            let _ = dispatcher.dispatch(&Action::Media { command });
        }
        let state = state.lock().unwrap();
        let modes: Vec<_> = state
            .requests
            .iter()
            .map(|request| request.args.last().unwrap().as_str())
            .collect();
        // Only the first key press may prompt, even though it was denied;
        // later presses only check and fail with the same message. A stop
        // never takes a mode.
        assert_eq!(
            modes,
            [
                "request",
                MEDIA_STOP_SCRIPT.last().unwrap(),
                "check",
                "check"
            ]
        );
    }

    #[test]
    fn media_stop_pauses_only_running_music_or_spotify_by_process_id() {
        for stdout in ["nothing to stop\n", "stopped Music, Spotify\n"] {
            let (mut dispatcher, state) = fake(vec![success(stdout)]);
            dispatcher
                .dispatch(&Action::Media {
                    command: MediaCommand::Stop,
                })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 1);
            assert_eq!(state.requests[0].program, OSASCRIPT);
            // No argv after the script: the player set is fixed.
            assert_eq!(state.requests[0].args, jxa(MEDIA_STOP_SCRIPT));
            assert_eq!(state.requests[0].timeout, PROCESS_TIMEOUT);
        }
        let script = MEDIA_STOP_SCRIPT.join("\n");
        for needle in [
            "['com.apple.Music', 'Music', stopMusic]",
            "['com.spotify.client', 'Spotify', pauseSpotify]",
            "runningApplicationsWithBundleIdentifier",
            // Music's `stop` (hook/Stop) as an Apple event addressed to the
            // process id, which cannot start a process that has quit.
            "descriptorWithProcessIdentifier(pid)",
            "0x686F6F6B, 0x53746F70",
            // Spotify's codes are unverified here, so it keeps JXA, which can
            // resolve the app by name and relaunch one that just quit.
            "Application(pid).pause();",
            "'nothing to stop'",
        ] {
            assert!(script.contains(needle), "missing {needle}");
        }
        // The liveness re-check runs immediately before every send, and a
        // player that quit after a failed send is skipped, not reported.
        let recheck = script
            .find("if (!running(pid, player[0])) continue;")
            .expect("re-check before the send");
        let send = script.find("player[2](pid);").unwrap();
        assert!(recheck < send);
        assert!(script.contains("catch (error) { if (running(pid, player[0]))"));
        assert!(script.contains("runningApplicationWithProcessIdentifier(pid)"));
        // No player is ever addressed by name or bundle id.
        for forbidden in [
            "Application('",
            "Application(player",
            "Application(pid).stop",
            "tell application",
            "launch",
            "activate",
            "CGEventPost",
        ] {
            assert!(!script.contains(forbidden), "found {forbidden}");
        }
    }

    #[test]
    fn media_failures_name_a_fixed_cause_and_never_echo_stderr() {
        let cases = [
            (
                MediaCommand::PlayPause,
                "execution error: Error: Error: media-key: not allowed to post events (-2700)",
                "Accessibility",
            ),
            (
                MediaCommand::NextTrack,
                "execution error: Error: Error: media-key: event check failed (-2700)",
                "no media key was sent",
            ),
            (
                MediaCommand::Stop,
                "execution error: Error: Error: media-stop: Spotify failed (-1743) (-2700)",
                "Automation",
            ),
            (
                MediaCommand::PreviousTrack,
                "something unexpected /Users/someone/secret",
                "stderr: 42 bytes, not shown",
            ),
        ];
        for (command, stderr, expected) in cases {
            let (mut dispatcher, state) = fake(vec![failed_with(stderr)]);
            let error = dispatcher.dispatch(&Action::Media { command }).unwrap_err();
            assert!(error.contains(expected), "{error}");
            assert!(error.contains("exit status: 1"), "{error}");
            assert!(!error.contains("execution error"), "{error}");
            assert!(!error.contains("secret"), "{error}");
            assert_eq!(state.lock().unwrap().requests.len(), 1);
        }
        // The Automation hint belongs only to the player that reported -1743.
        for (stderr, music, spotify) in [
            (
                "media-stop: Music failed (no reply); media-stop: Spotify failed (-1743)",
                "Music (no answer",
                "Spotify (macOS denied control",
            ),
            (
                "media-stop: Music failed (-1743); media-stop: Spotify failed (-600)",
                "Music (macOS denied control",
                "Spotify (failed)",
            ),
        ] {
            let (mut dispatcher, _) = fake(vec![failed_with(stderr)]);
            let error = dispatcher
                .dispatch(&Action::Media {
                    command: MediaCommand::Stop,
                })
                .unwrap_err();
            assert!(error.contains(music), "{error}");
            assert!(error.contains(spotify), "{error}");
            assert_eq!(error.matches("denied control").count(), 1, "{error}");
        }

        // Timeouts propagate once, without a retry.
        let (mut dispatcher, state) =
            fake(vec![Err("/usr/bin/osascript: subprocess timed out after 5000 ms; action may already have taken effect".into())]);
        let error = dispatcher
            .dispatch(&Action::Media {
                command: MediaCommand::PlayPause,
            })
            .unwrap_err();
        assert!(error.contains("timed out"), "{error}");
        assert_eq!(state.lock().unwrap().requests.len(), 1);
    }

    #[test]
    fn cancelled_media_actions_never_start_a_process() {
        struct CancelledRunner(AtomicBool, usize);
        impl ProcessRunner for CancelledRunner {
            fn run(&mut self, _: &ProcessRequest) -> Result<ProcessOutput, String> {
                self.1 += 1;
                success("")
            }

            fn cancellation(&self) -> Option<&AtomicBool> {
                Some(&self.0)
            }
        }
        for command in [
            MediaCommand::PlayPause,
            MediaCommand::NextTrack,
            MediaCommand::PreviousTrack,
            MediaCommand::Stop,
        ] {
            let mut dispatcher = MacOsDispatcher {
                runner: Box::new(CancelledRunner(AtomicBool::new(true), 0)),
                sleeper: Box::new(SystemSleeper),
                obs: None,
                media_access_requested: false,
            };
            let error = dispatcher.dispatch(&Action::Media { command }).unwrap_err();
            assert!(error.contains("action execution cancelled"), "{error}");
            assert!(dispatcher.unavailable(&Action::Media { command }).is_none());
        }
    }

    /// Proves every fixed media script compiles as JavaScript for Automation.
    /// `osacompile` only parses: no script runs, no event is posted and no
    /// player is touched.
    #[cfg(target_os = "macos")]
    #[test]
    fn media_scripts_compile_with_osacompile() {
        let directory =
            std::env::temp_dir().join(format!("cooltek-media-scripts-{}", std::process::id()));
        std::fs::create_dir_all(&directory).unwrap();
        for (name, lines) in [
            ("key", MEDIA_KEY_SCRIPT),
            ("stop", MEDIA_STOP_SCRIPT),
            ("macro", MACRO_SCRIPT),
        ] {
            let source = directory.join(format!("{name}.js"));
            // osascript joins `-e` items with newlines, so this is the same text.
            std::fs::write(&source, lines.join("\n")).unwrap();
            let output = std::process::Command::new("/usr/bin/osacompile")
                .args(["-l", "JavaScript", "-o"])
                .arg(directory.join(format!("{name}.scpt")))
                .arg(&source)
                .output()
                .unwrap();
            assert!(
                output.status.success(),
                "{name}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        std::fs::remove_dir_all(&directory).unwrap();
    }

    #[test]
    fn system_commands_use_fixed_keystrokes_and_screencapture() {
        let (mut dispatcher, state) = fake(vec![success(""), success(""), success("")]);
        for command in [
            SystemCommand::Undo,
            SystemCommand::Redo,
            SystemCommand::Screenshot,
        ] {
            dispatcher.dispatch(&Action::System { command }).unwrap();
        }
        let state = state.lock().unwrap();
        assert_eq!(
            args(&state.requests[0]),
            [
                "-e",
                "tell application \"System Events\" to keystroke \"z\" using {command down}"
            ]
        );
        assert_eq!(
            args(&state.requests[1]),
            [
                "-e",
                "tell application \"System Events\" to keystroke \"z\" using {command down, shift down}"
            ]
        );
        assert_eq!(state.requests[2].program, SCREENCAPTURE);
        assert_eq!(args(&state.requests[2]), ["-c"]);
    }

    #[test]
    fn text_pastes_through_stdin_and_restores_plain_clipboard_text() {
        for press_enter in [false, true] {
            let (mut dispatcher, state) = fake(vec![
                success(PLAIN_INFO),
                success("old"),
                success(""),
                success(""),
                success("zażółć\nnext"),
                success(""),
            ]);
            dispatcher
                .dispatch(&Action::Text {
                    text: "zażółć\nnext".into(),
                    press_enter,
                })
                .unwrap();
            let state = state.lock().unwrap();
            let requests = &state.requests;
            assert_eq!(requests.len(), 6);
            assert_eq!(requests[0].program, OSASCRIPT);
            assert_eq!(args(&requests[0]), ["-e", "clipboard info"]);
            assert_eq!(requests[1].program, PBPASTE);
            assert_eq!(args(&requests[1]), ["-Prefer", "txt"]);
            assert_eq!(requests[2].program, PBCOPY);
            assert!(requests[2].args.is_empty());
            assert_eq!(
                requests[2].stdin.as_deref(),
                Some("zażółć\nnext".as_bytes())
            );
            assert_eq!(requests[3].program, OSASCRIPT);
            let mut expected = PASTE_ARGS.to_vec();
            if press_enter {
                expected.splice(4..4, ["-e", "key code 36"]);
            }
            assert_eq!(args(&requests[3]), expected);
            assert_eq!(requests[3].stdin, None);
            // The clipboard is re-read after the wait and restored only while
            // it still holds the snippet.
            assert_eq!(requests[4].program, PBPASTE);
            assert_eq!(args(&requests[4]), ["-Prefer", "txt"]);
            assert_eq!(requests[5].program, PBCOPY);
            assert_eq!(requests[5].stdin.as_deref(), Some(b"old".as_slice()));
            for index in [0, 1, 2, 4, 5] {
                assert_eq!(requests[index].env, UTF8_ENV);
            }
            assert_eq!(state.pauses, [(4, CLIPBOARD_RESTORE_DELAY)]);
        }
    }

    #[test]
    fn text_with_script_syntax_never_reaches_osascript_arguments() {
        let hostile = "\" & (do shell script \"touch /tmp/pwned\") & \"\\\nend tell";
        let (mut dispatcher, state) = fake(vec![
            success("«class PNGf», 2048, «class utf8», 3\n"),
            success(""),
            success(""),
        ]);
        dispatcher
            .dispatch(&Action::Text {
                text: hostile.into(),
                press_enter: false,
            })
            .unwrap();
        let state = state.lock().unwrap();
        // Rich or non-text clipboard contents are neither read nor restored.
        assert_eq!(state.requests.len(), 3);
        assert_eq!(state.requests[1].program, PBCOPY);
        assert_eq!(state.requests[1].stdin.as_deref(), Some(hostile.as_bytes()));
        assert_eq!(args(&state.requests[2]), PASTE_ARGS);
        for request in &state.requests {
            assert!(request.args.iter().all(|arg| !arg.contains("pwned")));
        }
        assert!(state.pauses.is_empty());
    }

    #[test]
    fn text_skips_restore_for_unreadable_empty_or_large_clipboards() {
        let large = "x".repeat(STDIN_LIMIT + 1);
        for snapshot in [
            vec![Err("synthetic clipboard info failure".to_string())],
            vec![success("")],
            vec![success("«class utf8», 3, «class RTF », 90")],
            vec![success(PLAIN_INFO), success("")],
            vec![success(PLAIN_INFO), success(&large)],
            vec![success(PLAIN_INFO), failure("pbpaste failed")],
        ] {
            let snapshot_requests = snapshot.len();
            let mut results = snapshot;
            results.extend([success(""), success("")]);
            let (mut dispatcher, state) = fake(results);
            dispatcher
                .dispatch(&Action::Text {
                    text: "x".into(),
                    press_enter: false,
                })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), snapshot_requests + 2);
            assert_eq!(state.requests.last().unwrap().program, OSASCRIPT);
            assert!(state.pauses.is_empty());
        }
    }

    #[test]
    fn failed_paste_restores_immediately_and_reports_the_failure() {
        let (mut dispatcher, state) = fake(vec![
            success(PLAIN_INFO),
            success("old"),
            success(""),
            failure("not allowed assistive access (-1719)"),
            success("x"),
            success(""),
        ]);
        let error = dispatcher
            .dispatch(&Action::Text {
                text: "x".into(),
                press_enter: true,
            })
            .unwrap_err();
        assert!(error.contains("-1719"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 6);
        assert_eq!(state.requests[4].program, PBPASTE);
        assert_eq!(state.requests[5].stdin.as_deref(), Some(b"old".as_slice()));
        assert!(state.pauses.is_empty());

        let (mut dispatcher, _) = fake(vec![
            success(PLAIN_INFO),
            success("old"),
            success(""),
            success(""),
            success("x"),
            failure("pbcopy failed"),
        ]);
        let error = dispatcher
            .dispatch(&Action::Text {
                text: "x".into(),
                press_enter: false,
            })
            .unwrap_err();
        assert!(error.starts_with("text was pasted, but restoring"));
    }

    #[test]
    fn plain_text_clipboard_detection_is_strict() {
        assert!(is_plain_text_clipboard(PLAIN_INFO.as_bytes()));
        assert!(is_plain_text_clipboard(b"string, 4"));
        for info in [
            "".as_bytes(),
            b"\n",
            b"\xff\xfe",
            "«class utf8», 3, «class RTF », 80".as_bytes(),
            "«class utf8»".as_bytes(),
            "«class utf8», x".as_bytes(),
            "«class utf8», 99999999".as_bytes(),
            b"org.nspasteboard.ConcealedType, 0, string, 8",
            b"{string, 4}",
        ] {
            assert!(!is_plain_text_clipboard(info), "{info:?}");
        }
    }

    #[test]
    fn text_never_overwrites_a_clipboard_changed_during_the_wait() {
        for current in [
            success("copied meanwhile"),
            success(""),
            // A prefix or a trailing newline is not byte-for-byte equal.
            success("snippe"),
            success("snippet\n"),
            failure("pbpaste failed"),
        ] {
            let (mut dispatcher, state) = fake(vec![
                success(PLAIN_INFO),
                success("old"),
                success(""),
                success(""),
                current,
            ]);
            dispatcher
                .dispatch(&Action::Text {
                    text: "snippet".into(),
                    press_enter: false,
                })
                .unwrap();
            let state = state.lock().unwrap();
            assert_eq!(state.requests.len(), 5);
            assert_eq!(state.requests[4].program, PBPASTE);
            assert!(
                state
                    .requests
                    .iter()
                    .all(|request| request.stdin.as_deref() != Some(b"old".as_slice()))
            );
            assert_eq!(state.pauses, [(4, CLIPBOARD_RESTORE_DELAY)]);
        }

        // Same rule for the immediate restore after a failed paste.
        let (mut dispatcher, state) = fake(vec![
            success(PLAIN_INFO),
            success("old"),
            success(""),
            failure("paste failed"),
            success("copied meanwhile"),
        ]);
        let error = dispatcher
            .dispatch(&Action::Text {
                text: "snippet".into(),
                press_enter: false,
            })
            .unwrap_err();
        assert!(error.contains("paste failed"));
        assert_eq!(state.lock().unwrap().requests.len(), 5);
    }

    #[test]
    fn text_skips_saving_concealed_or_transient_clipboards() {
        for info in [
            "org.nspasteboard.ConcealedType, 0, «class utf8», 3",
            "«class utf8», 3, org.nspasteboard.TransientType, 0",
            "«class utf8», 3, ORG.NSPASTEBOARD.AutoGeneratedType, 0",
            "«class utf8», 3, «class Conc», 0, concealed, 0",
            "«class utf8», 3, Transient, 0",
        ] {
            assert!(has_sensitive_clipboard_marker(info.as_bytes()), "{info}");
            let (mut dispatcher, state) = fake(vec![success(info), success(""), success("")]);
            dispatcher
                .dispatch(&Action::Text {
                    text: "x".into(),
                    press_enter: false,
                })
                .unwrap();
            let state = state.lock().unwrap();
            // Never read with pbpaste, so nothing is restored and there is no wait.
            assert_eq!(state.requests.len(), 3);
            assert!(state.requests.iter().all(|r| r.program != PBPASTE));
            assert_eq!(state.requests[1].program, PBCOPY);
            assert!(state.pauses.is_empty());
        }
        assert!(!has_sensitive_clipboard_marker(PLAIN_INFO.as_bytes()));
    }

    #[test]
    fn multi_action_stops_when_the_overall_deadline_passes() {
        let (mut dispatcher, state) = fake(vec![success(""), success(""), success("")]);
        // Each subprocess "takes" 7 s of virtual time; no real time passes.
        state.lock().unwrap().request_cost = Duration::from_secs(7);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 10 },
                    Action::SetVolume { percent: 20 },
                    Action::OpenUrl {
                        url: "https://example.com".into(),
                    },
                    Action::SetVolume { percent: 40 },
                ],
            })
            .unwrap_err();
        // 7 s + 0.28 s + 7 s + 0.28 s = 14.56 s, so step 3 starts; at 21.84 s
        // step 4 is refused.
        assert!(
            error.starts_with("multi_action stopped before step 4 of 4 (set_volume)"),
            "{error}"
        );
        assert!(error.contains("15 s overall deadline"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 3);
        assert_eq!(state.requests[2].program, OPEN);
        assert_eq!(MULTI_ACTION_DEADLINE, Duration::from_secs(15));
    }

    #[test]
    fn multi_action_deadline_is_checked_after_the_last_pause_before_a_step() {
        let (mut dispatcher, state) = fake(vec![success("")]);
        // The first step ends just under the deadline; the pause crosses it.
        state.lock().unwrap().request_cost = MULTI_ACTION_DEADLINE - Duration::from_millis(1);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 10 },
                    Action::SetVolume { percent: 20 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action stopped before step 2 of 2"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 1);
        assert_eq!(state.pauses, [(1, STEP_SPACING)]);
    }

    #[test]
    fn oversize_stdin_is_rejected_before_any_process() {
        let (mut dispatcher, state) = fake(vec![]);
        let error = dispatcher
            .set_clipboard(vec![b'x'; STDIN_LIMIT + 1])
            .unwrap_err();
        assert!(error.contains("stdin exceeds"));
        assert!(state.lock().unwrap().requests.is_empty());
        const { assert!(Action::MAX_TEXT_BYTES <= STDIN_LIMIT) };
    }

    #[test]
    fn invalid_new_actions_never_start_subprocesses() {
        let (mut dispatcher, state) = fake(vec![]);
        for action in [
            Action::OpenUrl {
                url: "file:///etc/passwd".into(),
            },
            Action::OpenUrl {
                url: "https://example.com/\"; do shell script \"x\"".into(),
            },
            Action::Shortcut {
                key: "key code 36".into(),
                modifiers: vec![],
            },
            Action::Text {
                text: "a\u{1b}b".into(),
                press_enter: false,
            },
            Action::Text {
                text: "x".repeat(Action::MAX_TEXT_BYTES + 1),
                press_enter: false,
            },
            Action::MultiAction {
                actions: vec![Action::ToggleMute],
            },
            Action::MultiAction {
                actions: vec![
                    Action::ToggleMute,
                    Action::MultiAction {
                        actions: vec![Action::ToggleMute, Action::ToggleMute],
                    },
                ],
            },
            // An invalid later step blocks the whole sequence before step 1 runs.
            Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 10 },
                    Action::OpenUrl {
                        url: "javascript:alert(1)".into(),
                    },
                ],
            },
        ] {
            assert!(dispatcher.dispatch(&action).is_err(), "{action:?}");
        }
        assert!(state.lock().unwrap().requests.is_empty());
        assert!(state.lock().unwrap().pauses.is_empty());
    }

    #[test]
    fn multi_action_runs_steps_in_order_with_fixed_spacing() {
        let (mut dispatcher, state) = fake(vec![
            success(""),
            success(""),
            success("true\n"),
            success(""),
        ]);
        dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::OpenUrl {
                        url: "https://example.com".into(),
                    },
                    Action::Shortcut {
                        key: "tab".into(),
                        modifiers: vec![Modifier::Command],
                    },
                    Action::ToggleMute,
                ],
            })
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 4);
        assert_eq!(state.requests[0].program, OPEN);
        assert_eq!(
            args(&state.requests[1]),
            [
                "-e",
                "tell application \"System Events\" to key code 48 using {command down}"
            ]
        );
        assert_eq!(
            args(&state.requests[3]),
            ["-e", "set volume output muted false"]
        );
        // Spacing runs between steps only, never before the first or after the last.
        assert_eq!(state.pauses, [(1, STEP_SPACING), (2, STEP_SPACING)]);
        assert_eq!(STEP_SPACING, Duration::from_millis(280));
    }

    #[test]
    fn multi_action_stops_at_the_first_failed_step() {
        let (mut dispatcher, state) = fake(vec![success(""), failure("denied")]);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 20 },
                    Action::System {
                        command: SystemCommand::Undo,
                    },
                    Action::SetVolume { percent: 30 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action step 2 of 3 (system) failed:"));
        assert!(error.contains("denied"));
        let state = state.lock().unwrap();
        assert_eq!(state.requests.len(), 2);
        assert_eq!(state.pauses.len(), 1);

        let (mut dispatcher, state) = fake(vec![
            success(""),
            failed_with(
                "execution error: Error: Error: media-key: not allowed to post events (-2700)",
            ),
        ]);
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 20 },
                    Action::Media {
                        command: MediaCommand::PlayPause,
                    },
                    Action::SetVolume { percent: 30 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action step 2 of 3 (media) failed:"));
        assert!(error.contains("Accessibility"), "{error}");
        assert_eq!(state.lock().unwrap().requests.len(), 2);
    }

    #[test]
    fn media_steps_run_inside_a_multi_action() {
        let (mut dispatcher, state) =
            fake(vec![success(""), success(""), success("nothing to stop\n")]);
        dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::Media {
                        command: MediaCommand::NextTrack,
                    },
                    Action::SetVolume { percent: 30 },
                    Action::Media {
                        command: MediaCommand::Stop,
                    },
                ],
            })
            .unwrap();
        let state = state.lock().unwrap();
        let mut next = jxa(MEDIA_KEY_SCRIPT);
        next.extend(["--".into(), "17".into(), "request".into()]);
        assert_eq!(state.requests[0].args, next);
        assert_eq!(state.requests[2].args, jxa(MEDIA_STOP_SCRIPT));
        assert_eq!(state.pauses, [(1, STEP_SPACING), (2, STEP_SPACING)]);
    }

    #[test]
    fn multi_action_cancellation_during_spacing_stops_remaining_steps() {
        struct FlagRunner {
            cancellation: Arc<AtomicBool>,
            requests: Arc<Mutex<Vec<ProcessRequest>>>,
        }
        impl ProcessRunner for FlagRunner {
            fn run(&mut self, request: &ProcessRequest) -> Result<ProcessOutput, String> {
                self.requests.lock().unwrap().push(ProcessRequest {
                    program: request.program,
                    args: request.args.clone(),
                    ..ProcessRequest::default()
                });
                success("")
            }

            fn cancellation(&self) -> Option<&AtomicBool> {
                Some(&self.cancellation)
            }
        }
        struct CancellingSleeper(Arc<AtomicBool>);
        impl Sleeper for CancellingSleeper {
            fn sleep(
                &mut self,
                _: Duration,
                cancellation: Option<&AtomicBool>,
            ) -> Result<(), String> {
                self.0.store(true, Ordering::Release);
                check_cancellation(cancellation, "action pause")
            }
        }
        let cancellation = Arc::new(AtomicBool::new(false));
        let requests = Arc::new(Mutex::new(Vec::new()));
        let mut dispatcher = MacOsDispatcher {
            runner: Box::new(FlagRunner {
                cancellation: cancellation.clone(),
                requests: requests.clone(),
            }),
            sleeper: Box::new(CancellingSleeper(cancellation)),
            obs: None,
            media_access_requested: false,
        };
        let error = dispatcher
            .dispatch(&Action::MultiAction {
                actions: vec![
                    Action::SetVolume { percent: 20 },
                    Action::SetVolume { percent: 30 },
                ],
            })
            .unwrap_err();
        assert!(error.starts_with("multi_action stopped before step 2 of 2"));
        assert!(error.contains("action execution cancelled"));
        assert_eq!(requests.lock().unwrap().len(), 1);
    }

    #[test]
    fn system_sleeper_observes_cancellation_without_waiting() {
        let mut sleeper = SystemSleeper;
        assert!(sleeper.sleep(Duration::ZERO, None).is_ok());
        let cancelled = AtomicBool::new(true);
        assert!(
            sleeper
                .sleep(Duration::from_secs(60), Some(&cancelled))
                .unwrap_err()
                .contains("action execution cancelled")
        );
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn native_backend_reports_unsupported_platform() {
        let error = MacOsDispatcher::new()
            .dispatch(&Action::ToggleMute)
            .unwrap_err();
        assert!(error.contains("unsupported on this platform"));
    }
}
