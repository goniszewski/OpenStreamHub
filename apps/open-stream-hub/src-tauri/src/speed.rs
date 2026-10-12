//! The display speed test (a demo): how fast the 15-key module takes key images through a run's control channel.
//!
//! Two passes. The grid pass redraws all 15 keys over and over (a frame is one image per key); the single pass changes
//! one key at a time, round the grid. Each write is timed twice: as the app sees it (request to answer) and, when the
//! service reports it, as the device write alone. The loop is generic over a sink and a clock, so the numbers are
//! checked against a fake with known delays.

use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Serialize;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

/// The solid colours the test cycles through; the page draws the same ones in its preview (`demos/speed-test.ts`).
pub const PALETTE: [[u8; 3]; 8] = [
    [255, 64, 64],
    [255, 160, 32],
    [240, 224, 48],
    [64, 220, 96],
    [48, 210, 220],
    [64, 128, 255],
    [160, 96, 255],
    [255, 96, 200],
];
pub const KEYS: usize = 15;
/// Consecutive failed writes after which a pass gives up: the module is gone or stuck.
const GIVE_UP_AFTER: u32 = 10;

/// One 72x72 baseline JPEG per palette colour, base64, ready to send.
pub fn frames() -> Result<Vec<String>, String> {
    PALETTE
        .iter()
        .map(|colour| {
            let mut jpeg = Vec::new();
            let pixels: Vec<u8> = (0..72 * 72).flat_map(|_| *colour).collect();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 85)
                .encode(&pixels, 72, 72, image::ExtendedColorType::Rgb8)
                .map_err(|error| error.to_string())?;
            Ok(STANDARD.encode(jpeg))
        })
        .collect()
}

/// Where the test writes: one image to one key. Returns the device write time when the service reports it.
pub trait KeySink {
    fn draw(&self, key: u8, image: usize) -> Result<Option<Duration>, String>;
}

/// Monotonic time, so the loop can be driven by a fake in tests.
pub trait Clock {
    fn now(&self) -> Duration;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Pass {
    Grid,
    Single,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Progress {
    pub writes: u64,
    pub frames: u64,
    pub elapsed_ms: u64,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Quantiles {
    pub count: u64,
    pub mean_ms: f64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PassResult {
    pub pass: Pass,
    pub seconds: f64,
    pub writes: u64,
    pub failed: u64,
    /// Complete grids drawn (grid pass only).
    pub frames: u64,
    pub writes_per_second: f64,
    pub frames_per_second: f64,
    /// Request to answer, as the app sees it.
    pub round_trip: Quantiles,
    /// The device write alone, as the service timed it.
    pub device: Option<Quantiles>,
    pub cancelled: bool,
    /// Why the pass stopped early, when it did.
    pub stopped: Option<String>,
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

/// Mean, median, 95th percentile (nearest rank) and maximum of `samples`.
pub fn quantiles(samples: &[Duration]) -> Quantiles {
    if samples.is_empty() {
        return Quantiles::default();
    }
    let mut sorted = samples.to_vec();
    sorted.sort();
    let rank = |share: f64| {
        sorted[((share * sorted.len() as f64).ceil() as usize).clamp(1, sorted.len()) - 1]
    };
    let total: Duration = sorted.iter().sum();
    Quantiles {
        count: sorted.len() as u64,
        mean_ms: ms(total) / sorted.len() as f64,
        p50_ms: ms(rank(0.5)),
        p95_ms: ms(rank(0.95)),
        max_ms: ms(*sorted.last().unwrap()),
    }
}

/// Runs one pass for `duration` (or until cancelled), reporting progress after every write.
pub fn run_pass(
    sink: &dyn KeySink,
    clock: &dyn Clock,
    pass: Pass,
    duration: Duration,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(Progress),
) -> PassResult {
    let started = clock.now();
    let mut round_trip = Vec::new();
    let mut device = Vec::new();
    let (mut writes, mut failed, mut streak) = (0u64, 0u64, 0u32);
    let mut stopped = None;
    let mut cancelled = false;
    loop {
        let elapsed = clock.now() - started;
        if cancel.load(Ordering::SeqCst) {
            cancelled = true;
            break;
        }
        if elapsed >= duration {
            break;
        }
        // The grid pass writes key after key and changes every colour each frame; the single pass moves one key on
        // at a time.
        let (key, image) = match pass {
            Pass::Grid => (
                (writes % KEYS as u64) as u8,
                ((writes / KEYS as u64) + writes % KEYS as u64) as usize % PALETTE.len(),
            ),
            Pass::Single => (
                (writes % KEYS as u64) as u8,
                writes as usize % PALETTE.len(),
            ),
        };
        let before = clock.now();
        match sink.draw(key, image) {
            Ok(write) => {
                round_trip.push(clock.now() - before);
                device.extend(write);
                streak = 0;
            }
            Err(_)
                if {
                    streak += 1;
                    streak >= GIVE_UP_AFTER
                } =>
            {
                failed += 1;
                writes += 1;
                stopped = Some(format!("{GIVE_UP_AFTER} writes in a row failed"));
                break;
            }
            Err(_) => failed += 1,
        }
        writes += 1;
        progress(Progress {
            writes,
            frames: writes / KEYS as u64,
            elapsed_ms: (clock.now() - started).as_millis() as u64,
        });
    }
    let seconds = (clock.now() - started).as_secs_f64().max(f64::EPSILON);
    let frames = writes / KEYS as u64;
    PassResult {
        pass,
        seconds,
        writes,
        failed,
        frames,
        writes_per_second: writes as f64 / seconds,
        frames_per_second: if pass == Pass::Grid {
            frames as f64 / seconds
        } else {
            0.0
        },
        round_trip: quantiles(&round_trip),
        device: (!device.is_empty()).then(|| quantiles(&device)),
        cancelled,
        stopped,
    }
}

/// The sink of a real run: one `key_image` request per write over the run's control channel.
pub struct ControlSink {
    pub pipe: std::sync::Arc<crate::service::ControlPipe>,
    pub frames: Vec<String>,
}

impl KeySink for ControlSink {
    fn draw(&self, key: u8, image: usize) -> Result<Option<Duration>, String> {
        let answer = self.pipe.request(
            serde_json::json!({"type":"key_image","key":key,"jpeg":self.frames[image % self.frames.len()]}),
            crate::service::CONTROL_TIMEOUT,
        )?;
        Ok(answer
            .get("write_micros")
            .and_then(serde_json::Value::as_u64)
            .map(Duration::from_micros))
    }
}

/// The real clock.
pub struct SystemClock(pub std::time::Instant);

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}

/// Set by `demo_speed_cancel`; read by the pass that is running.
pub static CANCEL: AtomicBool = AtomicBool::new(false);

/// The longest a pass may run, whatever the page asks for.
pub const MAX_SECONDS: u64 = 120;

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    struct Fake {
        now: Cell<Duration>,
        delays: RefCell<Vec<Result<Duration, ()>>>,
        calls: RefCell<Vec<(u8, usize)>>,
        device: bool,
    }

    impl Clock for Fake {
        fn now(&self) -> Duration {
            self.now.get()
        }
    }

    impl KeySink for Fake {
        fn draw(&self, key: u8, image: usize) -> Result<Option<Duration>, String> {
            self.calls.borrow_mut().push((key, image));
            let next = self
                .delays
                .borrow()
                .get(self.calls.borrow().len() - 1)
                .copied()
                .unwrap_or(Ok(Duration::from_millis(10)));
            match next {
                Ok(delay) => {
                    self.now.set(self.now.get() + delay);
                    Ok(self.device.then(|| delay / 2))
                }
                Err(()) => {
                    self.now.set(self.now.get() + Duration::from_millis(3000));
                    Err("timed out".into())
                }
            }
        }
    }

    fn fake(delays: Vec<Result<Duration, ()>>, device: bool) -> Fake {
        Fake {
            now: Cell::new(Duration::ZERO),
            delays: RefCell::new(delays),
            calls: RefCell::new(Vec::new()),
            device,
        }
    }

    fn run(sink: &Fake, pass: Pass, seconds: u64) -> (PassResult, Vec<Progress>) {
        let mut seen = Vec::new();
        let result = run_pass(
            sink,
            sink,
            pass,
            Duration::from_secs(seconds),
            &AtomicBool::new(false),
            &mut |progress| seen.push(progress),
        );
        (result, seen)
    }

    #[test]
    fn quantiles_use_the_nearest_rank() {
        let samples: Vec<Duration> = (1..=20).map(Duration::from_millis).collect();
        let q = quantiles(&samples);
        assert_eq!(
            (q.count, q.p50_ms, q.p95_ms, q.max_ms),
            (20, 10.0, 19.0, 20.0)
        );
        assert!((q.mean_ms - 10.5).abs() < 1e-9);
        assert_eq!(quantiles(&[]), Quantiles::default());
        assert_eq!(quantiles(&[Duration::from_millis(7)]).p95_ms, 7.0);
    }

    #[test]
    fn a_grid_pass_counts_frames_and_times_every_write() {
        // 10 ms per image: 100 writes a second, 6.67 frames a second, for 3 s.
        let sink = fake(vec![], true);
        let (result, seen) = run(&sink, Pass::Grid, 3);
        assert_eq!((result.writes, result.frames, result.failed), (300, 20, 0));
        assert!((result.writes_per_second - 100.0).abs() < 0.01);
        assert!((result.frames_per_second - 20.0 / 3.0).abs() < 0.01);
        assert_eq!(
            (result.round_trip.p50_ms, result.round_trip.p95_ms),
            (10.0, 10.0)
        );
        assert_eq!(result.device.unwrap().p95_ms, 5.0);
        // Keys go round in order and every frame changes every key's colour.
        let calls = sink.calls.borrow();
        assert_eq!(
            calls[..3].iter().map(|call| call.0).collect::<Vec<_>>(),
            [0, 1, 2]
        );
        assert_ne!(calls[0].1, calls[KEYS].1);
        assert_eq!(seen.last().unwrap().writes, 300);
    }

    #[test]
    fn a_single_pass_changes_one_key_at_a_time_and_reports_the_slow_tail() {
        let mut delays = vec![Ok(Duration::from_millis(5)); 100];
        delays.extend([Ok(Duration::from_millis(90)); 5]);
        let sink = fake(delays, false);
        let (result, _) = run(&sink, Pass::Single, 1);
        assert_eq!(result.frames_per_second, 0.0);
        assert!(result.device.is_none());
        assert_eq!(result.round_trip.max_ms, 90.0);
        assert!(result.round_trip.p95_ms >= 5.0);
        assert_eq!(
            sink.calls
                .borrow()
                .iter()
                .take(16)
                .map(|call| call.0)
                .collect::<Vec<_>>(),
            (0..15).chain([0]).collect::<Vec<u8>>()
        );
    }

    #[test]
    fn failed_writes_are_counted_and_ten_in_a_row_end_the_pass() {
        let sink = fake(
            vec![
                Ok(Duration::from_millis(10)),
                Err(()),
                Ok(Duration::from_millis(10)),
            ],
            false,
        );
        let (result, _) = run(&sink, Pass::Grid, 1);
        assert_eq!(result.failed, 1);
        assert!(result.stopped.is_none());
        let dead = fake(vec![Err(()); 50], false);
        let (result, _) = run(&dead, Pass::Grid, 600);
        assert_eq!((result.failed, result.writes), (10, 10));
        assert!(result.stopped.unwrap().contains("in a row"));
    }

    #[test]
    fn a_cancelled_pass_stops_at_once_and_says_so() {
        let sink = fake(vec![], false);
        let cancel = AtomicBool::new(true);
        let result = run_pass(
            &sink,
            &sink,
            Pass::Grid,
            Duration::from_secs(30),
            &cancel,
            &mut |_| {},
        );
        assert!(result.cancelled);
        assert_eq!(result.writes, 0);
    }

    #[test]
    fn every_palette_colour_is_a_key_sized_jpeg() {
        let frames = frames().unwrap();
        assert_eq!(frames.len(), PALETTE.len());
        for frame in frames {
            cooltek_sdk_masterhub::keypad::validate_jpeg(&STANDARD.decode(frame).unwrap()).unwrap();
        }
    }
}
