//! The control channel of a running `masterhub-service`.
//!
//! A supervising process that holds the service's stdin open (`--exit-on-stdin-eof`) may also send it JSON lines
//! there, as `--control` allows. Each line is one request with an `id`; the service answers with one
//! `control_result` record on its stdout, in the same JSONL stream as its other records. The service owns the
//! module's HID interface for the whole run, so this is how anything else gets to the module while a run holds it.
//!
//! Requests:
//!
//! - `{"id":3,"type":"mode","execute":true}` switches the run between executing its actions (Live) and only
//!   logging them (Test), from the next action on. Nothing restarts: a countdown already running keeps running, and
//!   its alert follows the mode it expires in. Refused where actions cannot execute (not macOS).
//!
//! - `{"id":4,"type":"profile","profile":{...}}` replaces the run's bindings with a new profile, from the next input
//!   report on, with the same checks as a start. Refused when the run draws on the module (display sync), which takes a
//!   new profile by restarting; the dial's current function and a keypad's folder start again at the first.
//!
//! - `{"id":5,"type":"key_image","key":4,"jpeg":"<base64>"}` draws one 72×72 baseline JPEG on one key of an M4315
//!   keypad, through the handle the run holds. Accepted only by a run that has the module in software mode and draws
//!   nothing itself (no display sync), such as the one a demo starts; any other run refuses it, so a profile's own
//!   key artwork is never overwritten. The image is checked as the SDK checks every key image before anything is written.
//!
//! Lighting, only for modules with per-LED lighting (the knobs and the faders):
//!
//! - `{"id":1,"type":"led_look","led":null,"colour":[255,0,0],"mode":1,"brightness":100}` writes one look to one
//!   LED (`led` a number) or to every LED (`null`), as the official app does.
//! - `{"id":2,"type":"led_state"}` reads what the module reports about its LEDs.
//!
//! A line that is not a valid request is answered with `ok: false` and no `id`. End of stdin still ends the run.

use cooltek_sdk_masterhub::{
    ReportTransport,
    lighting::{LedLook, LedModule, read_led_state, write_led_lighting},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::{
    io::{self, Write},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, Ordering},
    },
};

/// Replaces the run's profile: checks it as a start does, then hands it to the input loop.
pub type ProfileSwap = Box<dyn Fn(&Value) -> Result<(), String> + Send>;

/// What the supervisor may change about a run besides lighting.
#[derive(Default)]
pub struct Hooks {
    /// The mode switch, when the run has one.
    pub mode: Option<RunMode>,
    /// The profile swap, when the run takes one.
    pub profile: Option<ProfileSwap>,
    /// Whether the run accepts `key_image`: an M4315 in software mode that draws nothing of its own.
    pub key_images: bool,
    /// The key report length, read from the module's descriptor with the first key image only.
    #[doc(hidden)]
    pub key_report_length: OnceLock<usize>,
}

/// The run's execute flag, which the dispatcher reads for every action, and whether this platform can execute.
#[derive(Clone)]
pub struct RunMode {
    /// True while the run executes its actions.
    pub execute: Arc<AtomicBool>,
    /// False where actions cannot execute at all; a switch to Live is then refused.
    pub can_execute: bool,
}

#[derive(Deserialize)]
struct Envelope {
    id: u64,
    #[serde(flatten)]
    request: Request,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Request {
    LedLook {
        led: Option<u8>,
        colour: [u8; 3],
        mode: u8,
        brightness: u8,
    },
    LedState,
    Mode {
        execute: bool,
    },
    Profile {
        profile: Value,
    },
    KeyImage {
        key: u8,
        jpeg: String,
    },
}

/// The largest key image accepted, well under the 64 KiB a control line may carry once encoded.
const MAX_KEY_IMAGE: usize = 40 * 1024;

fn draw_key<T: ReportTransport + ?Sized>(
    transport: &T,
    length: &OnceLock<usize>,
    key: u8,
    jpeg: &str,
) -> Result<u128, String> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let jpeg = STANDARD
        .decode(jpeg)
        .map_err(|_| "the key image is not valid base64".to_string())?;
    if jpeg.len() > MAX_KEY_IMAGE {
        return Err("the key image is too large".into());
    }
    let length = match length.get() {
        Some(length) => *length,
        None => {
            let read = cooltek_sdk_masterhub::output_report_length(transport, 0x02)
                .map_err(|error| error.to_string())?;
            *length.get_or_init(|| read)
        }
    };
    // Only the device write is timed: what the module needs for one image, without the request or the decode.
    let started = std::time::Instant::now();
    cooltek_sdk_masterhub::keypad::display_jpeg(transport, key, &jpeg, length)
        .map_err(|error| error.to_string())?;
    Ok(started.elapsed().as_micros())
}

/// Answers one control line: the `control_result` record to write.
pub fn answer<T: ReportTransport + ?Sized>(
    transport: &T,
    module: Option<LedModule>,
    hooks: &Hooks,
    line: &str,
) -> Value {
    let Ok(envelope) = serde_json::from_str::<Envelope>(line) else {
        return json!({"type":"control_result","id":null,"ok":false,"error":"the control message was not understood"});
    };
    let id = envelope.id;
    let outcome = match (envelope.request, module) {
        (Request::Profile { profile }, _) => match &hooks.profile {
            Some(swap) => swap(&profile).map(|()| json!({})),
            None => Err("this run cannot change its profile".to_string()),
        },
        (Request::Mode { execute }, _) => match hooks.mode.as_ref() {
            None => Err("this run cannot change its mode".to_string()),
            Some(_) if !execute => Ok(()),
            Some(mode) if mode.can_execute => Ok(()),
            Some(_) => Err("executing actions is supported only on macOS".to_string()),
        }
        .map(|()| {
            if let Some(mode) = hooks.mode.as_ref() {
                mode.execute.store(execute, Ordering::SeqCst);
            }
            json!({"execute": execute})
        }),
        (Request::KeyImage { key, jpeg }, _) => {
            if hooks.key_images {
                draw_key(transport, &hooks.key_report_length, key, &jpeg)
                    .map(|micros| json!({"write_micros": micros}))
            } else {
                Err("this run does not take key images".to_string())
            }
        }
        (_, None) => Err("this module has no per-LED lighting".to_string()),
        (
            Request::LedLook {
                led,
                colour,
                mode,
                brightness,
            },
            Some(module),
        ) => write_led_lighting(
            transport,
            module,
            led,
            LedLook {
                colour,
                mode,
                brightness,
            },
        )
        .map(|()| json!({})),
        (Request::LedState, Some(module)) => read_led_state(transport, module)
            .map(|state| json!({"brightness":state.brightness,"modes":state.modes})),
    };
    match outcome {
        Ok(Value::Object(mut result)) => {
            result.insert("type".into(), json!("control_result"));
            result.insert("id".into(), json!(id));
            result.insert("ok".into(), json!(true));
            Value::Object(result)
        }
        Ok(_) => unreachable!("results are objects"),
        Err(error) => json!({"type":"control_result","id":id,"ok":false,"error":error}),
    }
}

/// A writer several threads can hold clones of: each clone buffers its own bytes and passes only complete lines on,
/// one at a time, so records from different threads never interleave inside a line.
pub struct SharedRecords<W> {
    inner: Arc<Mutex<W>>,
    pending: Vec<u8>,
}

impl<W: Write> SharedRecords<W> {
    /// Wraps `writer`; clone the result for each thread that writes.
    pub fn new(writer: W) -> Self {
        Self {
            inner: Arc::new(Mutex::new(writer)),
            pending: Vec::new(),
        }
    }
}

impl<W> Clone for SharedRecords<W> {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            pending: Vec::new(),
        }
    }
}

impl<W: Write> Write for SharedRecords<W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        self.pending.extend_from_slice(buffer);
        while let Some(end) = self.pending.iter().position(|&byte| byte == b'\n') {
            let line: Vec<u8> = self.pending.drain(..=end).collect();
            let mut inner = self
                .inner
                .lock()
                .map_err(|_| io::Error::other("the record writer is unavailable"))?;
            inner.write_all(&line)?;
            inner.flush()?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// The handler for the stdin watchdog: answers each control line on `out`.
pub fn serve<T, W>(
    transport: T,
    module: Option<LedModule>,
    hooks: Hooks,
    mut out: SharedRecords<W>,
) -> impl FnMut(&str) + Send + 'static
where
    T: ReportTransport + Send + 'static,
    W: Write + Send + 'static,
{
    move |line| {
        let record = answer(&transport, module, &hooks, line);
        // A failed write only loses the answer; the run goes on.
        let _ = writeln!(out, "{record}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cooltek_sdk_masterhub::hidapi::HidResult;
    use std::cell::RefCell;

    struct Fake {
        writes: RefCell<Vec<Vec<u8>>>,
    }

    impl ReportTransport for Fake {
        fn read_timeout(&self, _: &mut [u8], _: i32) -> HidResult<usize> {
            Ok(0)
        }
        fn get_feature_report(&self, buffer: &mut [u8]) -> HidResult<usize> {
            let id = buffer[0];
            buffer[1] = 3;
            for byte in &mut buffer[2..5] {
                *byte = if id == 0x32 { 100 } else { 9 };
            }
            Ok(32)
        }
        fn send_feature_report(&self, _: &[u8]) -> HidResult<()> {
            Ok(())
        }
        fn write_output(&self, data: &[u8]) -> HidResult<usize> {
            self.writes.borrow_mut().push(data.to_vec());
            Ok(data.len())
        }
        fn get_report_descriptor(&self, buffer: &mut [u8]) -> HidResult<usize> {
            // Output report 2 with 63 payload bytes, as the knob and fader modules declare.
            const DESCRIPTOR: [u8; 17] = [
                0x05, 0x0c, 0x09, 0x01, 0xa1, 0x01, 0x75, 0x08, 0x96, 0x3f, 0x00, 0x85, 0x02, 0x91,
                0x02, 0xc0, 0x00,
            ];
            buffer[..DESCRIPTOR.len()].copy_from_slice(&DESCRIPTOR);
            Ok(DESCRIPTOR.len())
        }
    }

    fn fake() -> Fake {
        Fake {
            writes: RefCell::new(Vec::new()),
        }
    }

    #[test]
    fn a_look_for_one_led_is_written_as_colour_effect_brightness() {
        let device = fake();
        let line =
            r#"{"id":7,"type":"led_look","led":1,"colour":[255,0,0],"mode":9,"brightness":80}"#;
        let result = answer(&device, Some(LedModule::Knob), &Hooks::default(), line);
        assert_eq!(result["type"], "control_result");
        assert_eq!(result["id"], 7);
        assert_eq!(result["ok"], true);
        assert_eq!(device.writes.borrow().len(), 3);
    }

    #[test]
    fn every_led_is_written_when_none_is_named() {
        let device = fake();
        let line =
            r#"{"id":1,"type":"led_look","led":null,"colour":[0,255,0],"mode":1,"brightness":100}"#;
        assert_eq!(
            answer(&device, Some(LedModule::Knob), &Hooks::default(), line)["ok"],
            true
        );
        assert_eq!(device.writes.borrow().len(), 9);
    }

    #[test]
    fn the_modules_own_report_is_read_back() {
        let result = answer(
            &fake(),
            Some(LedModule::Knob),
            &Hooks::default(),
            r#"{"id":3,"type":"led_state"}"#,
        );
        assert_eq!(result["ok"], true);
        assert_eq!(result["brightness"], json!([100, 100, 100]));
        assert_eq!(result["modes"], json!([9, 9, 9]));
    }

    #[test]
    fn bad_values_and_unlit_modules_are_refused_without_writing() {
        let device = fake();
        let too_bright =
            r#"{"id":4,"type":"led_look","led":0,"colour":[1,2,3],"mode":1,"brightness":101}"#;
        let refused = answer(
            &device,
            Some(LedModule::Knob),
            &Hooks::default(),
            too_bright,
        );
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["id"], 4);
        let no_leds = r#"{"id":5,"type":"led_state"}"#;
        assert_eq!(
            answer(&device, None, &Hooks::default(), no_leds)["ok"],
            false
        );
        let missing =
            r#"{"id":6,"type":"led_look","led":9,"colour":[1,2,3],"mode":1,"brightness":5}"#;
        assert_eq!(
            answer(&device, Some(LedModule::Knob), &Hooks::default(), missing)["ok"],
            false
        );
        assert!(device.writes.borrow().is_empty());
    }

    fn with_mode(execute: &Arc<AtomicBool>, can_execute: bool) -> Hooks {
        Hooks {
            mode: Some(RunMode {
                execute: Arc::clone(execute),
                can_execute,
            }),
            profile: None,
            ..Hooks::default()
        }
    }

    #[test]
    fn the_mode_switches_in_place_and_live_is_refused_where_actions_cannot_run() {
        let execute = Arc::new(AtomicBool::new(false));
        let hooks = with_mode(&execute, true);
        let on = answer(
            &fake(),
            None,
            &hooks,
            r#"{"id":1,"type":"mode","execute":true}"#,
        );
        assert_eq!(
            (on["ok"].as_bool(), on["execute"].as_bool()),
            (Some(true), Some(true))
        );
        assert!(execute.load(Ordering::SeqCst));
        let off = answer(
            &fake(),
            None,
            &hooks,
            r#"{"id":2,"type":"mode","execute":false}"#,
        );
        assert_eq!(off["ok"], true);
        assert!(!execute.load(Ordering::SeqCst));

        let elsewhere = with_mode(&execute, false);
        let refused = answer(
            &fake(),
            None,
            &elsewhere,
            r#"{"id":3,"type":"mode","execute":true}"#,
        );
        assert_eq!(refused["ok"], false);
        assert!(!execute.load(Ordering::SeqCst));
        // Test is always allowed, and a run without a switchable mode says so.
        let test = answer(
            &fake(),
            None,
            &elsewhere,
            r#"{"id":4,"type":"mode","execute":false}"#,
        );
        assert_eq!(test["ok"], true);
        let none = answer(
            &fake(),
            None,
            &Hooks::default(),
            r#"{"id":5,"type":"mode","execute":false}"#,
        );
        assert_eq!(none["ok"], false);
    }

    #[test]
    fn a_profile_is_handed_to_the_swap_and_its_refusal_is_the_answer() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let recorder = Arc::clone(&seen);
        let hooks = Hooks {
            mode: None,
            key_images: false,
            profile: Some(Box::new(move |profile: &Value| {
                recorder.lock().unwrap().push(profile.to_string());
                if profile["name"] == "bad" {
                    Err("the profile is not valid".to_string())
                } else {
                    Ok(())
                }
            })),
            ..Hooks::default()
        };
        let good = answer(
            &fake(),
            None,
            &hooks,
            r#"{"id":1,"type":"profile","profile":{"name":"ok"}}"#,
        );
        assert_eq!(good["ok"], true);
        let bad = answer(
            &fake(),
            None,
            &hooks,
            r#"{"id":2,"type":"profile","profile":{"name":"bad"}}"#,
        );
        assert_eq!(
            (bad["ok"].as_bool(), bad["error"].as_str()),
            (Some(false), Some("the profile is not valid"))
        );
        assert_eq!(seen.lock().unwrap().len(), 2);
        let without = answer(
            &fake(),
            None,
            &Hooks::default(),
            r#"{"id":3,"type":"profile","profile":{}}"#,
        );
        assert_eq!(without["ok"], false);
    }

    #[test]
    fn an_unreadable_line_is_answered_without_an_id() {
        for line in [
            "",
            "not json",
            r#"{"type":"led_state"}"#,
            r#"{"id":1,"type":"nope"}"#,
        ] {
            let result = answer(&fake(), Some(LedModule::Knob), &Hooks::default(), line);
            assert_eq!(result["ok"], false, "{line}");
            assert_eq!(result["id"], Value::Null, "{line}");
        }
    }

    #[test]
    fn records_from_two_writers_never_interleave_inside_a_line() {
        let sink = Arc::new(Mutex::new(Vec::<u8>::new()));
        struct Out(Arc<Mutex<Vec<u8>>>);
        impl Write for Out {
            fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(buffer);
                Ok(buffer.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut first = SharedRecords::new(Out(Arc::clone(&sink)));
        let mut second = first.clone();
        // The first writer's record arrives in two pieces with the second's whole record in between.
        first.write_all(b"{\"a\":").unwrap();
        second.write_all(b"{\"b\":2}\n").unwrap();
        first.write_all(b"1}\n").unwrap();
        let text = String::from_utf8(sink.lock().unwrap().clone()).unwrap();
        assert_eq!(text, "{\"b\":2}\n{\"a\":1}\n");
    }

    fn key_jpeg(side: u32) -> String {
        use base64::{Engine, engine::general_purpose::STANDARD};
        let mut bytes = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, 80)
            .encode(
                &vec![200; (side * side * 3) as usize],
                side,
                side,
                image::ExtendedColorType::Rgb8,
            )
            .unwrap();
        STANDARD.encode(bytes)
    }

    #[test]
    fn a_key_image_is_drawn_only_by_a_run_that_takes_them() {
        let device = fake();
        let line = format!(
            r#"{{"id":8,"type":"key_image","key":4,"jpeg":"{}"}}"#,
            key_jpeg(72)
        );
        let refused = answer(&device, None, &Hooks::default(), &line);
        assert_eq!(refused["ok"], false);
        assert_eq!(refused["id"], 8);
        assert!(device.writes.borrow().is_empty());
        let hooks = Hooks {
            key_images: true,
            ..Hooks::default()
        };
        let drawn = answer(&device, None, &hooks, &line);
        assert_eq!(drawn["ok"], true, "{drawn}");
        assert!(drawn["write_micros"].is_u64());
        assert!(!device.writes.borrow().is_empty());
        // The key index sits in the packet header after the report id and command.
        assert!(
            device
                .writes
                .borrow()
                .iter()
                .all(|report| report[0] == 0x02 && report[6] == 4)
        );
    }

    #[test]
    fn a_bad_key_image_is_refused_before_anything_is_written() {
        let device = fake();
        let hooks = Hooks {
            key_images: true,
            ..Hooks::default()
        };
        for line in [
            r#"{"id":1,"type":"key_image","key":0,"jpeg":"not base64!"}"#.to_string(),
            r#"{"id":2,"type":"key_image","key":0,"jpeg":"AAAA"}"#.to_string(),
            format!(
                r#"{{"id":3,"type":"key_image","key":0,"jpeg":"{}"}}"#,
                key_jpeg(64)
            ),
            format!(
                r#"{{"id":4,"type":"key_image","key":15,"jpeg":"{}"}}"#,
                key_jpeg(72)
            ),
        ] {
            assert_eq!(answer(&device, None, &hooks, &line)["ok"], false, "{line}");
        }
        assert!(device.writes.borrow().is_empty());
    }
}
