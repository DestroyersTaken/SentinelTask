//! Playback options and the drift-free scheduler (RESEARCH §1 #10, #22).
//!
//! The scheduler is a pure state machine: it is given "now" and answers
//! "fire event N", "wait until T" or "done". Every event's deadline is
//! computed from the start of the current pass — never from the previous
//! event — so sleeping inaccuracies cannot accumulate over long runs.

use serde::{Deserialize, Serialize};

use crate::model::Event;

/// How many times to run a macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum Repeat {
    /// Run exactly `count` passes (1 = play once).
    Times { count: u32 },
    /// Run until stopped.
    Forever,
    /// Keep starting new passes until `secs` have elapsed; the pass that is
    /// running when time is up is allowed to finish so a farm loop never
    /// stops half-way through an action.
    Duration { secs: u64 },
}

impl Default for Repeat {
    fn default() -> Self {
        Repeat::Times { count: 1 }
    }
}

impl Repeat {
    pub fn describe(&self) -> String {
        match *self {
            Repeat::Times { count: 1 } => "Once".into(),
            Repeat::Times { count } => format!("{count}×"),
            Repeat::Forever => "Forever".into(),
            Repeat::Duration { secs } => format!("For {}", crate::util::format_duration_secs(secs)),
        }
    }
}

/// How recorded mouse motion is replayed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseMode {
    /// Move the cursor to the exact recorded positions. Best for desktop
    /// apps, browsers, 2D games and anything with a visible cursor.
    #[default]
    Absolute,
    /// Replay raw mouse deltas. Required for games that lock the cursor and
    /// read relative motion (3D cameras). Clicks are sent where the cursor is.
    Relative,
}

/// Per-run playback options. Saved inside each macro as its defaults.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackOptions {
    /// Speed multiplier: 2.0 plays twice as fast.
    pub speed: f64,
    pub repeat: Repeat,
    /// Real-time pause between passes, in milliseconds (not speed-scaled).
    pub loop_delay_ms: u64,
    pub mouse_mode: MouseMode,
    /// Rescale positions when the current screen differs from the recording.
    pub scale_to_screen: bool,
    /// Pause automatically while the recorded target window is not in front.
    pub focus_guard: bool,
    /// Skip mouse-move events and only replay clicks/wheel at their positions.
    pub clicks_only: bool,
}

impl Default for PlaybackOptions {
    fn default() -> Self {
        Self {
            speed: 1.0,
            repeat: Repeat::default(),
            loop_delay_ms: 0,
            mouse_mode: MouseMode::Absolute,
            scale_to_screen: false,
            focus_guard: false,
            clicks_only: false,
        }
    }
}

pub const MIN_SPEED: f64 = 0.1;
pub const MAX_SPEED: f64 = 100.0;

impl PlaybackOptions {
    pub fn clamped_speed(&self) -> f64 {
        if self.speed.is_finite() {
            self.speed.clamp(MIN_SPEED, MAX_SPEED)
        } else {
            1.0
        }
    }

    /// Estimated total run time in microseconds, `None` if unbounded.
    pub fn estimated_total_us(&self, pass_us: u64) -> Option<u64> {
        let scaled = (pass_us as f64 / self.clamped_speed()) as u64;
        match self.repeat {
            Repeat::Times { count } => {
                let n = count.max(1) as u64;
                Some(scaled * n + self.loop_delay_ms * 1000 * (n - 1))
            }
            Repeat::Forever => None,
            Repeat::Duration { secs } => Some(secs * 1_000_000),
        }
    }
}

/// What the player wants the engine to do next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    /// Inject `events[index]` now.
    Fire { index: usize },
    /// Nothing is due; sleep until this time on the run clock.
    WaitUntil { t_us: u64 },
    /// A pass just finished. The engine should release anything still held
    /// before the next pass starts (RESEARCH §1 #5).
    PassComplete { completed: u32 },
    /// All passes are done.
    Done,
}

/// When the engine falls behind by more than this (e.g. the PC stalled),
/// the remaining events are shifted later instead of being fired in a
/// burst. A burst would collapse the gaps between clicks and key presses
/// that the target application depends on.
pub const DEFAULT_MAX_LATENESS_US: u64 = 150_000;

/// The scheduler. Time is a monotonic "run clock" in microseconds that the
/// engine pauses while playback is paused.
#[derive(Debug, Clone)]
pub struct Player {
    times: Vec<u64>,
    pass_len_us: u64,
    speed: f64,
    repeat: Repeat,
    loop_delay_us: u64,
    max_lateness_us: u64,
    /// Run-clock time at which the current pass started.
    pass_start: u64,
    next_index: usize,
    completed: u32,
    started_at: u64,
    done: bool,
    /// Total time we shifted the schedule because the engine was late.
    total_shift_us: u64,
    /// Set after `PassComplete` until the next pass begins.
    between_passes: bool,
}

impl Player {
    pub fn new(events: &[Event], opts: &PlaybackOptions, start_us: u64) -> Self {
        let times: Vec<u64> = events.iter().map(|e| e.t_us).collect();
        debug_assert!(times.windows(2).all(|w| w[0] <= w[1]), "events must be sorted");
        let pass_len_us = times.last().copied().unwrap_or(0);
        let done = times.is_empty() || matches!(opts.repeat, Repeat::Times { count: 0 });
        Self {
            times,
            pass_len_us,
            speed: opts.clamped_speed(),
            repeat: opts.repeat,
            loop_delay_us: opts.loop_delay_ms.saturating_mul(1000),
            max_lateness_us: DEFAULT_MAX_LATENESS_US,
            pass_start: start_us,
            next_index: 0,
            completed: 0,
            started_at: start_us,
            done,
            total_shift_us: 0,
            between_passes: false,
        }
    }

    pub fn with_max_lateness(mut self, us: u64) -> Self {
        self.max_lateness_us = us;
        self
    }

    fn scaled(&self, t: u64) -> u64 {
        (t as f64 / self.speed).round() as u64
    }

    fn deadline(&self, index: usize) -> u64 {
        self.pass_start + self.scaled(self.times[index])
    }

    pub fn completed_passes(&self) -> u32 {
        self.completed
    }

    /// 1-based number of the pass currently running.
    pub fn current_pass(&self) -> u32 {
        self.completed + 1
    }

    pub fn total_passes(&self) -> Option<u32> {
        match self.repeat {
            Repeat::Times { count } => Some(count),
            _ => None,
        }
    }

    pub fn total_shift_us(&self) -> u64 {
        self.total_shift_us
    }

    pub fn is_done(&self) -> bool {
        self.done
    }

    /// Progress through the current pass, 0.0..=1.0.
    pub fn pass_progress(&self, now: u64) -> f32 {
        let len = self.scaled(self.pass_len_us).max(1);
        (now.saturating_sub(self.pass_start) as f64 / len as f64).clamp(0.0, 1.0) as f32
    }

    pub fn elapsed_us(&self, now: u64) -> u64 {
        now.saturating_sub(self.started_at)
    }

    /// Stop after the current pass (used by "stop at end of loop").
    pub fn finish_after_current_pass(&mut self) {
        self.repeat = Repeat::Times {
            count: self.completed + 1,
        };
    }

    fn more_passes(&self, now: u64) -> bool {
        match self.repeat {
            Repeat::Times { count } => self.completed < count,
            Repeat::Forever => true,
            Repeat::Duration { secs } => now.saturating_sub(self.started_at) < secs * 1_000_000,
        }
    }

    /// Ask what to do at run-clock time `now`.
    pub fn step(&mut self, now: u64) -> Step {
        if self.done {
            return Step::Done;
        }
        if self.between_passes {
            if !self.more_passes(now) {
                self.done = true;
                return Step::Done;
            }
            let next_start = self.pass_start + self.scaled(self.pass_len_us) + self.loop_delay_us;
            if now < next_start {
                return Step::WaitUntil { t_us: next_start };
            }
            self.between_passes = false;
            self.next_index = 0;
            self.pass_start = next_start;
        }
        if self.next_index >= self.times.len() {
            self.completed += 1;
            self.between_passes = true;
            return Step::PassComplete {
                completed: self.completed,
            };
        }
        let due = self.deadline(self.next_index);
        if now < due {
            return Step::WaitUntil { t_us: due };
        }
        let late = now - due;
        if late > self.max_lateness_us {
            // Re-anchor so this event fires now and every later event keeps
            // its recorded spacing relative to it.
            self.pass_start += late;
            self.total_shift_us += late;
        }
        let index = self.next_index;
        self.next_index += 1;
        Step::Fire { index }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{EventKind, Key};

    fn evs(times: &[u64]) -> Vec<Event> {
        times
            .iter()
            .map(|&t| {
                Event::new(
                    t,
                    EventKind::Key {
                        key: Key::from_vk(0x41),
                        down: true,
                    },
                )
            })
            .collect()
    }

    /// Simulate an engine with a perfect clock that jumps to each wait
    /// target, optionally oversleeping by `overshoot` each time.
    fn simulate(player: &mut Player, mut now: u64, overshoot: u64) -> Vec<(usize, u64, u32)> {
        let mut fired = Vec::new();
        let mut pass = 0;
        for _ in 0..1_000_000 {
            match player.step(now) {
                Step::Fire { index } => fired.push((index, now, pass)),
                Step::WaitUntil { t_us } => {
                    assert!(t_us > now, "wait must be in the future");
                    now = t_us + overshoot;
                }
                Step::PassComplete { completed } => pass = completed,
                Step::Done => return fired,
            }
        }
        panic!("player never finished");
    }

    #[test]
    fn plays_once_at_recorded_times() {
        let e = evs(&[0, 1000, 5000]);
        let mut p = Player::new(&e, &PlaybackOptions::default(), 100);
        let fired = simulate(&mut p, 100, 0);
        assert_eq!(fired, vec![(0, 100, 0), (1, 1100, 0), (2, 5100, 0)]);
        assert!(p.is_done());
    }

    #[test]
    fn oversleeping_never_accumulates_drift() {
        // 10 000 events 10 ms apart, engine oversleeps 2 ms on every wait.
        let times: Vec<u64> = (0..10_000).map(|i| i * 10_000).collect();
        let e = evs(&times);
        let mut p = Player::new(&e, &PlaybackOptions::default(), 0);
        let fired = simulate(&mut p, 0, 2_000);
        for (i, at, _) in fired {
            let err = at as i64 - times[i] as i64;
            assert!((0..=2_000).contains(&err), "event {i} off by {err}us");
        }
    }

    #[test]
    fn speed_scales_deadlines() {
        let e = evs(&[0, 1000, 2000]);
        let opts = PlaybackOptions {
            speed: 2.0,
            ..Default::default()
        };
        let mut p = Player::new(&e, &opts, 0);
        let at: Vec<u64> = simulate(&mut p, 0, 0).into_iter().map(|f| f.1).collect();
        assert_eq!(at, vec![0, 500, 1000]);
    }

    #[test]
    fn speed_is_clamped_and_nan_safe() {
        let mut o = PlaybackOptions {
            speed: 0.0,
            ..Default::default()
        };
        assert_eq!(o.clamped_speed(), MIN_SPEED);
        o.speed = f64::NAN;
        assert_eq!(o.clamped_speed(), 1.0);
        o.speed = 1e9;
        assert_eq!(o.clamped_speed(), MAX_SPEED);
    }

    #[test]
    fn repeats_with_loop_delay_and_reports_passes() {
        let e = evs(&[0, 1000]);
        let opts = PlaybackOptions {
            repeat: Repeat::Times { count: 3 },
            loop_delay_ms: 2,
            ..Default::default()
        };
        let mut p = Player::new(&e, &opts, 0);
        let fired = simulate(&mut p, 0, 0);
        let at: Vec<u64> = fired.iter().map(|f| f.1).collect();
        // Pass length 1000 us + 2000 us delay => passes start at 0, 3000, 6000.
        assert_eq!(at, vec![0, 1000, 3000, 4000, 6000, 7000]);
        assert_eq!(p.completed_passes(), 3);
    }

    #[test]
    fn duration_mode_finishes_running_pass() {
        let e = evs(&[0, 400_000]);
        let opts = PlaybackOptions {
            repeat: Repeat::Duration { secs: 1 },
            ..Default::default()
        };
        let mut p = Player::new(&e, &opts, 0);
        let fired = simulate(&mut p, 0, 0);
        // Passes start at 0, 0.4 s, 0.8 s (still < 1 s) — third pass completes.
        assert_eq!(p.completed_passes(), 3);
        assert_eq!(fired.len(), 6);
    }

    #[test]
    fn forever_keeps_going_until_finish_requested() {
        let e = evs(&[0, 10]);
        let opts = PlaybackOptions {
            repeat: Repeat::Forever,
            ..Default::default()
        };
        let mut p = Player::new(&e, &opts, 0);
        let mut now = 0;
        let mut passes = 0;
        while passes < 50 {
            match p.step(now) {
                Step::WaitUntil { t_us } => now = t_us,
                Step::PassComplete { completed } => passes = completed,
                Step::Done => panic!("forever ended"),
                Step::Fire { .. } => {}
            }
        }
        p.finish_after_current_pass();
        let rest = simulate(&mut p, now, 0);
        assert_eq!(rest.len(), 2);
        assert_eq!(p.completed_passes(), 51);
    }

    #[test]
    fn stall_shifts_schedule_instead_of_bursting() {
        let e = evs(&[0, 100_000, 200_000, 300_000]);
        let mut p = Player::new(&e, &PlaybackOptions::default(), 0);
        assert_eq!(p.step(0), Step::Fire { index: 0 });
        // PC freezes for 2 s.
        assert_eq!(p.step(2_000_000), Step::Fire { index: 1 });
        // Next event keeps its 100 ms spacing rather than firing immediately.
        assert_eq!(p.step(2_000_000), Step::WaitUntil { t_us: 2_100_000 });
        assert!(p.total_shift_us() >= 1_900_000);
    }

    #[test]
    fn small_lateness_is_caught_up_without_shift() {
        let e = evs(&[0, 100_000, 200_000]);
        let mut p = Player::new(&e, &PlaybackOptions::default(), 0);
        p.step(0);
        assert_eq!(p.step(150_000), Step::Fire { index: 1 });
        assert_eq!(p.step(150_000), Step::WaitUntil { t_us: 200_000 });
        assert_eq!(p.total_shift_us(), 0);
    }

    #[test]
    fn empty_macro_and_zero_count_are_done_immediately() {
        let mut p = Player::new(&[], &PlaybackOptions::default(), 0);
        assert_eq!(p.step(0), Step::Done);
        let opts = PlaybackOptions {
            repeat: Repeat::Times { count: 0 },
            ..Default::default()
        };
        let mut p = Player::new(&evs(&[0]), &opts, 0);
        assert_eq!(p.step(0), Step::Done);
    }

    #[test]
    fn estimates() {
        let o = PlaybackOptions {
            repeat: Repeat::Times { count: 3 },
            loop_delay_ms: 1000,
            speed: 2.0,
            ..Default::default()
        };
        assert_eq!(o.estimated_total_us(10_000_000), Some(15_000_000 + 2_000_000));
        let f = PlaybackOptions {
            repeat: Repeat::Forever,
            ..Default::default()
        };
        assert_eq!(f.estimated_total_us(1), None);
    }

    #[test]
    fn progress_and_pass_counters() {
        let e = evs(&[0, 1000]);
        let opts = PlaybackOptions {
            repeat: Repeat::Times { count: 2 },
            ..Default::default()
        };
        let p = Player::new(&e, &opts, 0);
        assert_eq!(p.current_pass(), 1);
        assert_eq!(p.total_passes(), Some(2));
        assert!((p.pass_progress(500) - 0.5).abs() < 1e-3);
    }
}
