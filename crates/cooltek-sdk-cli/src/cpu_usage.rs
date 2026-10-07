//! Whole-machine CPU usage for the dial's `cpu` face, with no extra dependency.
//!
//! [`CpuUsage`] turns successive per-core tick counters into a usage
//! percentage: the busy ticks (user, system, nice) gained since the previous
//! sample divided by all ticks gained, summed over every core. The first sample
//! only records a baseline, so it reports nothing. [`MacCpuStat`] reads the
//! counters on macOS through the stable Mach call
//! `host_processor_info(PROCESSOR_CPU_LOAD_INFO)`, declared below in a small
//! `extern "C"` block; on every other platform it reports nothing.
//!
//! Only CPU usage is provided. GPU usage and temperatures need private IOKit or
//! SMC interfaces and are out of scope.

use crate::dial_artwork::{StatKind, StatReading, StatSource};

/// One core's cumulative tick counters, as the kernel reports them. Each
/// counter is a `u32` that wraps around.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CoreTicks {
    pub user: u32,
    pub system: u32,
    pub idle: u32,
    pub nice: u32,
}

/// CPU usage from a tick reader `R`, which returns every core's counters or
/// `None` when they cannot be read.
pub struct CpuUsage<R> {
    read: R,
    previous: Option<Vec<CoreTicks>>,
    /// The last percentage reported since the baseline was (re)started.
    last: Option<u8>,
}

impl<R: FnMut() -> Option<Vec<CoreTicks>>> CpuUsage<R> {
    /// No baseline yet: the first sample reports nothing.
    pub fn new(read: R) -> Self {
        Self {
            read,
            previous: None,
            last: None,
        }
    }
}

impl<R: FnMut() -> Option<Vec<CoreTicks>>> StatSource for CpuUsage<R> {
    /// Usage since the previous sample, rounded and clamped to 0–100.
    ///
    /// `None` for the first sample, after a failed read (which also drops the
    /// baseline), or when the core count changed (the new counters become the
    /// baseline). When the read worked but no tick elapsed, the older baseline
    /// is kept, so the next sample covers the longer span, and the last
    /// percentage is repeated (`None` only if there is none yet): an idle
    /// interval is not an unavailable sensor.
    fn sample(&mut self) -> Option<StatReading> {
        let Some(current) = (self.read)() else {
            self.previous = None;
            self.last = None;
            return None;
        };
        match &self.previous {
            Some(previous) if previous.len() == current.len() => {
                if let Some(percent) = usage(previous, &current) {
                    self.previous = Some(current);
                    self.last = Some(percent);
                }
            }
            _ => {
                self.previous = Some(current);
                self.last = None;
            }
        }
        self.last.map(|percent| StatReading {
            kind: StatKind::Cpu,
            percent,
        })
    }
}

/// Usage in percent between two samples of the same cores, or `None` when no
/// tick elapsed. Each counter's delta is taken with wrapping subtraction, so a
/// counter that wrapped once since the last sample still counts correctly.
fn usage(previous: &[CoreTicks], current: &[CoreTicks]) -> Option<u8> {
    let (mut busy, mut total) = (0u64, 0u64);
    for (before, after) in previous.iter().zip(current) {
        let delta = |from: u32, to: u32| u64::from(to.wrapping_sub(from));
        let core_busy = delta(before.user, after.user)
            + delta(before.system, after.system)
            + delta(before.nice, after.nice);
        busy += core_busy;
        total += core_busy + delta(before.idle, after.idle);
    }
    // At most 4 × u32::MAX ticks per core: no realistic core count overflows.
    (total > 0).then(|| ((busy * 100 + total / 2) / total).min(100) as u8)
}

/// The machine's CPU usage: Mach counters on macOS, nothing elsewhere.
pub struct MacCpuStat(CpuUsage<fn() -> Option<Vec<CoreTicks>>>);

impl MacCpuStat {
    /// Reads nothing until the first [`StatSource::sample`].
    pub fn new() -> Self {
        Self(CpuUsage::new(read_ticks))
    }
}

impl Default for MacCpuStat {
    fn default() -> Self {
        Self::new()
    }
}

impl StatSource for MacCpuStat {
    fn sample(&mut self) -> Option<StatReading> {
        self.0.sample()
    }
}

#[cfg(not(target_os = "macos"))]
fn read_ticks() -> Option<Vec<CoreTicks>> {
    None
}

#[cfg(target_os = "macos")]
use mach::read_ticks;

/// The Mach declarations, from `<mach/host_info.h>`, `<mach/processor_info.h>`,
/// `<mach/machine.h>` and `<mach/vm_map.h>`. All are stable public API.
#[cfg(target_os = "macos")]
mod mach {
    use std::ffi::{c_int, c_uint};

    use super::CoreTicks;

    type KernReturn = c_int;
    type Natural = c_uint;
    type MachPort = c_uint;
    type Integer = c_int;

    const KERN_SUCCESS: KernReturn = 0;
    const PROCESSOR_CPU_LOAD_INFO: c_int = 2;
    /// Counters per core, in the order `CPU_STATE_USER` (0), `CPU_STATE_SYSTEM`
    /// (1), `CPU_STATE_IDLE` (2), `CPU_STATE_NICE` (3).
    const CPU_STATE_MAX: usize = 4;

    unsafe extern "C" {
        /// The task's own port; what the `mach_task_self()` macro reads.
        #[allow(non_upper_case_globals)]
        static mach_task_self_: MachPort;
        fn mach_host_self() -> MachPort;
        fn mach_port_deallocate(task: MachPort, name: MachPort) -> KernReturn;
        fn host_processor_info(
            host: MachPort,
            flavor: c_int,
            out_processor_count: *mut Natural,
            out_processor_info: *mut *mut Integer,
            out_processor_info_count: *mut Natural,
        ) -> KernReturn;
        fn vm_deallocate(task: MachPort, address: usize, size: usize) -> KernReturn;
    }

    /// Every core's tick counters, or `None` when the call fails or returns an
    /// array of an unexpected shape.
    pub(super) fn read_ticks() -> Option<Vec<CoreTicks>> {
        let mut cpu_count: Natural = 0;
        let mut info: *mut Integer = std::ptr::null_mut();
        let mut info_count: Natural = 0;
        // SAFETY:
        // - `mach_task_self_` is set up by libSystem before `main` and never
        //   changes afterwards; reading it is a plain load of a `u32`.
        // - `mach_host_self` returns a send right to the host port that this
        //   function owns; it is released with `mach_port_deallocate` right after
        //   the one call that uses it, whatever that call returned.
        // - The three out-pointers refer to live, aligned locals for the whole
        //   call. On success the kernel stores the core count, a pointer to a
        //   freshly mapped array of `info_count` `integer_t`s, and `info_count`.
        // - That array is owned by this function from then on. The only early
        //   `return` is for a failed call or a null array, where nothing was
        //   mapped. Between a successful call and the single `vm_deallocate`
        //   below there is no `return`, `?` or panic, so the array is unmapped
        //   on every path, with exactly the size the kernel reported. Nothing
        //   reads it afterwards: the counters are copied into an owned `Vec`.
        // - The slice covers exactly `info_count` elements, as reported, and is
        //   only built for a non-null pointer. Copying uses `as_chunks` into
        //   fixed-size arrays, which cannot panic; an allocation failure aborts
        //   the process rather than unwinding.
        // - `u32` tick deltas are computed later with wrapping arithmetic, so
        //   the reinterpreted `integer_t` values never overflow anything here.
        unsafe {
            let task = mach_task_self_;
            let host = mach_host_self();
            let result = host_processor_info(
                host,
                PROCESSOR_CPU_LOAD_INFO,
                &mut cpu_count,
                &mut info,
                &mut info_count,
            );
            mach_port_deallocate(task, host);
            if result != KERN_SUCCESS || info.is_null() {
                return None;
            }
            let length = info_count as usize;
            let values = std::slice::from_raw_parts(info, length);
            let ticks = if cpu_count > 0
                && (cpu_count as usize).checked_mul(CPU_STATE_MAX) == Some(length)
            {
                // `natural_t` counters stored as `integer_t`: reinterpret the bits.
                let cores = values.as_chunks::<CPU_STATE_MAX>().0.iter();
                Some(
                    cores
                        .map(|&[user, system, idle, nice]| CoreTicks {
                            user: user as u32,
                            system: system as u32,
                            idle: idle as u32,
                            nice: nice as u32,
                        })
                        .collect(),
                )
            } else {
                None
            };
            vm_deallocate(
                task,
                info as usize,
                length.saturating_mul(std::mem::size_of::<Integer>()),
            );
            ticks
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use super::*;

    fn core(busy: u32, idle: u32) -> CoreTicks {
        CoreTicks {
            user: busy,
            system: 0,
            idle,
            nice: 0,
        }
    }

    /// A tick reader replaying `samples`, then failing.
    fn replay(
        samples: Vec<Option<Vec<CoreTicks>>>,
    ) -> CpuUsage<impl FnMut() -> Option<Vec<CoreTicks>>> {
        let mut samples = VecDeque::from(samples);
        CpuUsage::new(move || samples.pop_front().flatten())
    }

    fn percents(usage: &mut impl StatSource, count: usize) -> Vec<Option<u8>> {
        (0..count)
            .map(|_| {
                usage.sample().map(|reading| {
                    assert_eq!(reading.kind, StatKind::Cpu);
                    reading.percent
                })
            })
            .collect()
    }

    #[test]
    fn the_first_sample_is_only_a_baseline() {
        let mut usage = replay(vec![Some(vec![core(10, 10)]), Some(vec![core(40, 40)])]);
        assert_eq!(percents(&mut usage, 2), [None, Some(50)]);
    }

    #[test]
    fn usage_sums_busy_states_over_every_core() {
        let before = [
            CoreTicks {
                user: 0,
                system: 0,
                idle: 0,
                nice: 0,
            },
            CoreTicks {
                user: 0,
                system: 0,
                idle: 0,
                nice: 0,
            },
        ];
        let after = [
            CoreTicks {
                user: 10,
                system: 5,
                idle: 85,
                nice: 0,
            },
            CoreTicks {
                user: 30,
                system: 20,
                idle: 40,
                nice: 10,
            },
        ];
        // Busy 75 of 200 ticks: 37.5 rounds to 38.
        assert_eq!(usage(&before, &after), Some(38));
        assert_eq!(usage(&before, &before), None);
        // All idle and all busy.
        assert_eq!(usage(&[core(5, 5)], &[core(5, 105)]), Some(0));
        assert_eq!(usage(&[core(5, 5)], &[core(105, 5)]), Some(100));
    }

    #[test]
    fn wrapped_counters_still_count_the_ticks_since_the_last_sample() {
        let before = core(u32::MAX - 9, u32::MAX - 29);
        // 20 busy and 60 idle ticks, both counters wrapped past zero.
        let after = core(10, 30);
        assert_eq!(usage(&[before], &[after]), Some(25));
        let mut stat = replay(vec![Some(vec![before]), Some(vec![after])]);
        assert_eq!(percents(&mut stat, 2), [None, Some(25)]);
    }

    #[test]
    fn no_elapsed_ticks_report_nothing_and_keep_the_older_baseline() {
        let mut stat = replay(vec![
            Some(vec![core(0, 0)]),
            Some(vec![core(0, 0)]),
            // Measured from the first baseline: 30 of 40 ticks busy.
            Some(vec![core(30, 10)]),
        ]);
        assert_eq!(percents(&mut stat, 3), [None, None, Some(75)]);
    }

    #[test]
    fn no_elapsed_ticks_after_a_reading_repeat_that_reading() {
        let mut stat = replay(vec![
            Some(vec![core(0, 0)]),
            Some(vec![core(10, 10)]),
            // Nothing elapsed: the last percentage stands, not "unavailable".
            Some(vec![core(10, 10)]),
            Some(vec![core(10, 10)]),
            Some(vec![core(40, 20)]),
            // A failed read forgets it.
            None,
            Some(vec![core(40, 20)]),
            Some(vec![core(40, 20)]),
        ]);
        assert_eq!(
            percents(&mut stat, 8),
            [
                None,
                Some(50),
                Some(50),
                Some(50),
                Some(75),
                None,
                None,
                None
            ]
        );
    }

    #[test]
    fn a_failed_read_or_a_new_core_count_restarts_the_baseline() {
        let mut stat = replay(vec![
            Some(vec![core(0, 0)]),
            None,
            Some(vec![core(10, 10)]),
            Some(vec![core(20, 20)]),
            Some(vec![core(20, 20), core(0, 0)]),
            Some(vec![core(30, 30), core(10, 0)]),
        ]);
        assert_eq!(
            percents(&mut stat, 6),
            [None, None, None, Some(50), None, Some(67)]
        );
    }

    #[test]
    fn usage_is_clamped_and_never_overflows() {
        // The largest possible delta on many cores stays in range.
        let before = vec![core(0, 0); 512];
        let after = vec![
            CoreTicks {
                user: u32::MAX,
                system: u32::MAX,
                idle: 0,
                nice: u32::MAX,
            };
            512
        ];
        assert_eq!(usage(&before, &after), Some(100));
        assert_eq!(usage(&[], &[]), None);
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn other_platforms_report_no_cpu_usage() {
        let mut stat = MacCpuStat::new();
        assert_eq!(stat.sample(), None);
        assert_eq!(stat.sample(), None);
    }

    /// Calls the real Mach API twice.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_real_sampler_reports_a_percentage_on_macos() {
        let mut stat = MacCpuStat::new();
        let ticks = read_ticks().expect("host_processor_info failed");
        assert!(!ticks.is_empty());
        assert_eq!(stat.sample(), None, "the first sample is a baseline");
        // Spin so ticks certainly elapse between the two samples.
        let start = std::time::Instant::now();
        let mut spin = 0u64;
        while start.elapsed() < std::time::Duration::from_millis(250) {
            spin = std::hint::black_box(spin.wrapping_add(1));
        }
        let reading = stat.sample().expect("no second sample");
        assert_eq!(reading.kind, StatKind::Cpu);
        assert!(reading.percent <= 100, "{reading:?}");
    }
}
