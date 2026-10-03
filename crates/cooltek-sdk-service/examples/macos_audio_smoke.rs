//! Controlled audio check without HID. Read-only unless `--execute` is provided.

use cooltek_sdk_service::{
    Action, ActionDispatcher,
    macos::{AudioSnapshot, MacOsDispatcher},
};
use std::process::ExitCode;

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("macos_audio_smoke: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let execute = match arguments(std::env::args().skip(1))? {
        Arguments::Help => {
            println!(
                "Usage: macos_audio_smoke [--execute]\nWithout --execute, only current audio state is queried. Execution changes volume by five percentage points, toggles mute, and restores the original state. Volume verification permits one percentage point of target deviation, requires movement in the requested direction, and preserves mute."
            );
            return Ok(());
        }
        Arguments::Preview => false,
        Arguments::Execute => true,
    };
    let mut host = MacOsDispatcher::new();
    let original = host.snapshot().map_err(|error| {
        format!("audio state unavailable; output may not support system volume control; no changes attempted: {error}")
    })?;
    println!("Original: {original:?}");
    if !execute {
        println!(
            "Read-only preview. --execute would change volume by {} percentage points, toggle mute, then restore this exact snapshot.",
            delta(original)
        );
        return Ok(());
    }
    let observations = controlled_smoke(&mut host, original)?;
    println!("Volume verified: {:?}", observations.after_volume);
    println!("Mute verified: {:?}", observations.after_mute);
    println!("Original state restored and verified: {original:?}");
    Ok(())
}

#[derive(Debug, Eq, PartialEq)]
enum Arguments {
    Preview,
    Execute,
    Help,
}

fn arguments(args: impl Iterator<Item = String>) -> Result<Arguments, String> {
    let mut result = Arguments::Preview;
    for argument in args {
        if result != Arguments::Preview {
            return Err("provide only one of --execute or --help".into());
        }
        result = match argument.as_str() {
            "--execute" => Arguments::Execute,
            "--help" | "-h" => Arguments::Help,
            _ => {
                return Err(format!(
                    "unknown argument {argument:?}; expected --execute or --help"
                ));
            }
        };
    }
    Ok(result)
}

trait AudioHost {
    fn snapshot(&mut self) -> Result<AudioSnapshot, String>;
    fn restore(&mut self, snapshot: &AudioSnapshot) -> Result<(), String>;
    fn dispatch(&mut self, action: &Action) -> Result<(), String>;
}

impl AudioHost for MacOsDispatcher {
    fn snapshot(&mut self) -> Result<AudioSnapshot, String> {
        self.audio_snapshot()
    }

    fn restore(&mut self, snapshot: &AudioSnapshot) -> Result<(), String> {
        self.restore_audio_snapshot(snapshot)
    }

    fn dispatch(&mut self, action: &Action) -> Result<(), String> {
        ActionDispatcher::dispatch(self, action)
    }
}

struct Observations {
    after_volume: AudioSnapshot,
    after_mute: AudioSnapshot,
}

fn delta(original: AudioSnapshot) -> i16 {
    if original.output_volume > 95 { -5 } else { 5 }
}

fn verify_actions(
    host: &mut impl AudioHost,
    original: AudioSnapshot,
) -> Result<Observations, String> {
    let requested_delta = delta(original);
    host.dispatch(&Action::Volume {
        delta: requested_delta,
    })?;
    let expected_volume = (i16::from(original.output_volume) + requested_delta) as u8;
    let after_volume = host.snapshot()?;
    // Integer readback can quantize a requested percentage. Tolerance never
    // permits an unchanged output or movement opposite to the requested delta.
    let moved_in_requested_direction = if requested_delta > 0 {
        after_volume.output_volume > original.output_volume
    } else {
        after_volume.output_volume < original.output_volume
    };
    if after_volume.output_volume.abs_diff(expected_volume) > 1
        || !moved_in_requested_direction
        || after_volume.output_muted != original.output_muted
    {
        return Err(format!(
            "volume change was not verified: expected volume {expected_volume} within one percentage point, movement in the requested direction, and mute {}, got {after_volume:?}; output may be fixed-volume or state may have changed concurrently",
            original.output_muted
        ));
    }
    host.dispatch(&Action::ToggleMute)?;
    let after_mute = host.snapshot()?;
    let expected_volume = after_volume.output_volume;
    if after_mute.output_muted == original.output_muted
        || after_mute.output_volume != expected_volume
    {
        return Err(format!(
            "mute toggle was not verified: expected volume {expected_volume} and mute {}, got {after_mute:?}; output may be unsupported or state may have changed concurrently",
            !original.output_muted
        ));
    }
    Ok(Observations {
        after_volume,
        after_mute,
    })
}

fn controlled_smoke(
    host: &mut impl AudioHost,
    original: AudioSnapshot,
) -> Result<Observations, String> {
    original.validate()?;
    // No printing or early return between the first mutation and restoration:
    // all ordinary action/query errors still reach exact best-effort cleanup.
    let checks = verify_actions(host, original);
    let mut cleanup_errors = Vec::new();
    if let Err(error) = host.restore(&original) {
        cleanup_errors.push(error);
    }
    match host.snapshot() {
        Ok(restored) if restored == original => {}
        Ok(restored) => cleanup_errors.push(format!(
            "restoration verification mismatch: expected {original:?}, got {restored:?}"
        )),
        Err(error) => cleanup_errors.push(format!("restoration verification failed: {error}")),
    }
    if !cleanup_errors.is_empty() {
        let cleanup = format!(
            "original audio restoration failed: {}",
            cleanup_errors.join("; ")
        );
        return Err(match checks {
            Ok(_) => cleanup,
            Err(error) => format!("{error}; {cleanup}"),
        });
    }
    checks.map_err(|error| format!("{error}; original audio state restored and verified"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    struct FakeHost {
        snapshots: VecDeque<Result<AudioSnapshot, String>>,
        actions: Vec<Action>,
        action_error: Option<String>,
        restores: Vec<AudioSnapshot>,
        restore_error: Option<String>,
    }
    impl AudioHost for FakeHost {
        fn snapshot(&mut self) -> Result<AudioSnapshot, String> {
            self.snapshots
                .pop_front()
                .expect("unexpected snapshot query")
        }
        fn restore(&mut self, snapshot: &AudioSnapshot) -> Result<(), String> {
            self.restores.push(*snapshot);
            self.restore_error.take().map_or(Ok(()), Err)
        }
        fn dispatch(&mut self, action: &Action) -> Result<(), String> {
            self.actions.push(action.clone());
            self.action_error.take().map_or(Ok(()), Err)
        }
    }
    fn state(volume: u8, muted: bool) -> AudioSnapshot {
        AudioSnapshot {
            output_volume: volume,
            output_muted: muted,
        }
    }
    fn fake(snapshots: Vec<Result<AudioSnapshot, String>>) -> FakeHost {
        FakeHost {
            snapshots: snapshots.into(),
            actions: vec![],
            action_error: None,
            restores: vec![],
            restore_error: None,
        }
    }

    #[test]
    fn explicit_execution_preflight_rejects_unknown_or_combined_flags() {
        assert_eq!(arguments([].into_iter()).unwrap(), Arguments::Preview);
        assert_eq!(
            arguments(["--execute".into()].into_iter()).unwrap(),
            Arguments::Execute
        );
        assert!(arguments(["--execut".into()].into_iter()).is_err());
        assert!(arguments(["--execute".into(), "--help".into()].into_iter()).is_err());
        assert!(arguments(["--execute".into(), "--execute".into()].into_iter()).is_err());
    }

    #[test]
    fn successful_check_restores_exact_snapshot_at_volume_edges_and_step_boundary() {
        for (original, expected_delta, target) in [
            (state(0, false), 5, 5),
            (state(95, true), 5, 100),
            (state(96, false), -5, 91),
            (state(100, true), -5, 95),
        ] {
            let mut host = fake(vec![
                Ok(state(target, original.output_muted)),
                Ok(state(target, !original.output_muted)),
                Ok(original),
            ]);
            assert!(controlled_smoke(&mut host, original).is_ok());
            assert_eq!(host.restores, [original]);
            assert_eq!(
                host.actions,
                [
                    Action::Volume {
                        delta: expected_delta
                    },
                    Action::ToggleMute
                ]
            );
        }
    }

    #[test]
    fn action_and_query_failures_always_restore_original_state() {
        let original = state(40, false);
        let mut action_failure = fake(vec![Ok(original)]);
        action_failure.action_error = Some("synthetic action failure".into());
        assert!(
            controlled_smoke(&mut action_failure, original)
                .err()
                .unwrap()
                .contains("original audio state restored and verified")
        );
        assert_eq!(action_failure.restores, [original]);
        let mut query_failure = fake(vec![Err("synthetic query failure".into()), Ok(original)]);
        assert!(controlled_smoke(&mut query_failure, original).is_err());
        assert_eq!(query_failure.restores, [original]);
    }

    #[test]
    fn unchanged_fixed_volume_fails_before_mute_and_still_restores() {
        let original = state(40, false);
        let mut host = fake(vec![Ok(original), Ok(original)]);
        assert!(
            controlled_smoke(&mut host, original)
                .err()
                .unwrap()
                .contains("fixed-volume")
        );
        assert_eq!(host.actions.len(), 1);
        assert_eq!(host.restores, [original]);
    }

    #[test]
    fn one_point_target_deviation_keeps_observed_volume_for_mute_verification() {
        let original = state(75, false);
        let mut host = fake(vec![
            Ok(state(79, false)),
            Ok(state(79, true)),
            Ok(original),
        ]);
        let observations = controlled_smoke(&mut host, original).unwrap();
        assert_eq!(observations.after_volume, state(79, false));
        assert_eq!(observations.after_mute, state(79, true));
        assert_eq!(host.restores, [original]);
    }

    #[test]
    fn unchanged_or_wrong_direction_volume_fails_and_restores() {
        let original = state(75, false);
        for observed in [75, 74] {
            let mut host = fake(vec![Ok(state(observed, false)), Ok(original)]);
            assert!(controlled_smoke(&mut host, original).is_err());
            assert_eq!(host.actions, [Action::Volume { delta: 5 }]);
            assert_eq!(host.restores, [original]);
        }
    }

    #[test]
    fn larger_target_deviation_or_changed_volume_during_mute_fails() {
        let original = state(75, false);
        for observed in [78, 82] {
            let mut host = fake(vec![Ok(state(observed, false)), Ok(original)]);
            assert!(controlled_smoke(&mut host, original).is_err());
            assert_eq!(host.restores, [original]);
        }
        let mut host = fake(vec![
            Ok(state(79, false)),
            Ok(state(80, true)),
            Ok(original),
        ]);
        let error = controlled_smoke(&mut host, original).err().unwrap();
        assert!(error.contains("mute toggle was not verified: expected volume 79"));
        assert_eq!(host.restores, [original]);
    }

    #[test]
    fn restoration_errors_and_mismatches_are_not_false_successes() {
        let original = state(40, false);
        let mut host = fake(vec![
            Ok(state(45, false)),
            Ok(state(45, true)),
            Ok(state(50, true)),
        ]);
        host.restore_error = Some("synthetic restoration failure".into());
        let error = controlled_smoke(&mut host, original).err().unwrap();
        assert!(error.contains("synthetic restoration failure"));
        assert!(error.contains("restoration verification mismatch"));
        assert_eq!(host.restores, [original]);
    }

    #[test]
    fn invalid_original_snapshot_never_mutates_or_queries() {
        let mut host = fake(vec![]);
        assert!(controlled_smoke(&mut host, state(101, false)).is_err());
        assert!(host.actions.is_empty());
        assert!(host.restores.is_empty());
    }
}
