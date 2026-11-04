//! The playback thread.

use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use parking_lot::Mutex;

use sentinel_core::edit::rescale_positions;
use sentinel_core::state::PressedState;
use sentinel_core::{Event, PlaybackOptions, Player, ScreenInfo, Step};

use crate::platform::{self, Injector, Sleeper, Waker};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PauseReason {
    User,
    /// Focus guard: the target program isn't in front.
    WaitingForWindow(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FinishReason {
    Completed,
    Stopped,
    UserActivity(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum RunState {
    Countdown { remaining_ms: u64 },
    WaitingForModifiers,
    Playing,
    Paused(PauseReason),
    Finished(FinishReason),
}

#[derive(Debug, Clone, PartialEq)]
pub struct PlayStatus {
    pub state: RunState,
    /// 1-based pass currently running.
    pub pass: u32,
    pub total_passes: Option<u32>,
    pub elapsed_us: u64,
    pub pass_progress: f32,
    pub injected: u64,
    /// Time the schedule was shifted because the PC stalled.
    pub shifted_us: u64,
    pub blocked_inputs: u32,
    pub estimated_total_us: Option<u64>,
    pub timer: &'static str,
}

impl Default for PlayStatus {
    fn default() -> Self {
        Self {
            state: RunState::Playing,
            pass: 1,
            total_passes: None,
            elapsed_us: 0,
            pass_progress: 0.0,
            injected: 0,
            shifted_us: 0,
            blocked_inputs: 0,
            estimated_total_us: None,
            timer: "",
        }
    }
}

#[derive(Default)]
struct Control {
    stop: AtomicBool,
    pause: AtomicBool,
    finish_after_pass: AtomicBool,
    /// 0 = none, 1 = user activity.
    abort_reason: AtomicU8,
    activity_text: Mutex<String>,
}

/// Everything needed to start a run.
#[derive(Debug, Clone)]
pub struct PlayRequest {
    pub events: Arc<Vec<Event>>,
    pub opts: PlaybackOptions,
    pub recorded_screen: Option<ScreenInfo>,
    /// Process name for the focus guard (`opts.focus_guard` must be set).
    pub focus_process: Option<String>,
    pub countdown_secs: u32,
    pub keep_awake: bool,
    /// Wait for Ctrl/Alt/Shift/Win to be released first (started by hotkey).
    pub wait_for_modifiers: bool,
}

pub struct PlaybackHandle {
    ctl: Arc<Control>,
    status: Arc<Mutex<PlayStatus>>,
    thread: Option<JoinHandle<()>>,
}

impl PlaybackHandle {
    pub fn status(&self) -> PlayStatus {
        self.status.lock().clone()
    }

    pub fn stop(&self) {
        self.ctl.stop.store(true, Ordering::SeqCst);
    }

    /// Stop because the user touched the mouse/keyboard.
    pub fn abort_for_activity(&self, what: &str) {
        *self.ctl.activity_text.lock() = what.to_string();
        self.ctl.abort_reason.store(1, Ordering::SeqCst);
        self.ctl.stop.store(true, Ordering::SeqCst);
    }

    pub fn toggle_pause(&self) {
        let p = self.ctl.pause.load(Ordering::SeqCst);
        self.ctl.pause.store(!p, Ordering::SeqCst);
    }

    pub fn is_user_paused(&self) -> bool {
        self.ctl.pause.load(Ordering::SeqCst)
    }

    pub fn finish_after_pass(&self) {
        self.ctl.finish_after_pass.store(true, Ordering::SeqCst);
    }

    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }

    /// Wait for the thread to end and return the final status.
    pub fn join(mut self) -> PlayStatus {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.status.lock().clone()
    }
}

impl Drop for PlaybackHandle {
    fn drop(&mut self) {
        self.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

/// Start playback on a new thread.
pub fn start(req: PlayRequest, waker: Waker) -> PlaybackHandle {
    let ctl = Arc::new(Control::default());
    let status = Arc::new(Mutex::new(PlayStatus {
        total_passes: match req.opts.repeat {
            sentinel_core::Repeat::Times { count } => Some(count),
            _ => None,
        },
        state: if req.countdown_secs > 0 {
            RunState::Countdown {
                remaining_ms: req.countdown_secs as u64 * 1000,
            }
        } else {
            RunState::Playing
        },
        ..PlayStatus::default()
    }));
    let (c2, s2) = (ctl.clone(), status.clone());
    let thread = std::thread::Builder::new()
        .name("sentinel-playback".into())
        .spawn(move || run(req, c2, s2, waker))
        .expect("spawn playback thread");
    PlaybackHandle {
        ctl,
        status,
        thread: Some(thread),
    }
}

fn set_state(status: &Mutex<PlayStatus>, waker: &Waker, state: RunState) {
    status.lock().state = state;
    waker();
}

fn run(req: PlayRequest, ctl: Arc<Control>, status: Arc<Mutex<PlayStatus>>, waker: Waker) {
    // Prepare a private copy of the events (rescaled if requested).
    let mut events: Vec<Event> = (*req.events).clone();
    if req.opts.scale_to_screen {
        if let Some(rec) = req.recorded_screen {
            let now = platform::screen_info();
            if rec.desktop != now.desktop {
                rescale_positions(&mut events, rec.desktop, now.desktop);
            }
        }
    }

    // Countdown.
    let mut remaining = req.countdown_secs as u64 * 1000;
    while remaining > 0 {
        if ctl.stop.load(Ordering::SeqCst) {
            set_state(&status, &waker, RunState::Finished(FinishReason::Stopped));
            return;
        }
        set_state(
            &status,
            &waker,
            RunState::Countdown {
                remaining_ms: remaining,
            },
        );
        let step = remaining.min(100);
        std::thread::sleep(Duration::from_millis(step));
        remaining -= step;
    }

    if req.wait_for_modifiers {
        set_state(&status, &waker, RunState::WaitingForModifiers);
        platform::wait_modifiers_released(Duration::from_secs(3));
    }

    if req.keep_awake {
        platform::keep_awake(true);
    }
    platform::boost_thread_priority(true);

    let reason = play_loop(&events, &req, &ctl, &status, &waker);

    if req.keep_awake {
        platform::keep_awake(false);
    }
    let reason = if ctl.abort_reason.load(Ordering::SeqCst) == 1 {
        FinishReason::UserActivity(ctl.activity_text.lock().clone())
    } else {
        reason
    };
    set_state(&status, &waker, RunState::Finished(reason));
}

fn play_loop(
    events: &[Event],
    req: &PlayRequest,
    ctl: &Control,
    status: &Mutex<PlayStatus>,
    waker: &Waker,
) -> FinishReason {
    let mut sleeper = Sleeper::new();
    // The injector releases everything it pressed when dropped — including
    // when this thread unwinds from a panic.
    let mut inj = Injector::new(req.opts.mouse_mode);
    let clicks_only = req.opts.clicks_only;
    let focus = if req.opts.focus_guard {
        req.focus_process.clone().filter(|p| !p.is_empty())
    } else {
        None
    };

    let real_start = platform::now_us();
    // Run clock = real clock minus time spent paused.
    let mut paused_total: u64 = 0;
    let mut player = Player::new(events, &req.opts, 0);
    {
        let mut s = status.lock();
        s.state = RunState::Playing;
        s.total_passes = player.total_passes();
        s.estimated_total_us = req.opts.estimated_total_us(events.last().map(|e| e.t_us).unwrap_or(0));
        s.timer = sleeper.kind();
    }
    waker();

    let mut injected: u64 = 0;
    let mut last_status = 0u64;
    let mut last_wake = 0u64;
    let mut last_focus_check = 0u64;
    let mut focus_ok = true;

    loop {
        if ctl.stop.load(Ordering::SeqCst) {
            return FinishReason::Stopped;
        }
        if ctl.finish_after_pass.swap(false, Ordering::SeqCst) {
            player.finish_after_current_pass();
        }

        // Focus guard check (cheap, but no need to do it per event).
        let real_now = platform::now_us();
        if let Some(proc_name) = &focus {
            if real_now.saturating_sub(last_focus_check) >= 100_000 {
                last_focus_check = real_now;
                focus_ok = platform::foreground_window().is_some_and(|w| w.matches_process(proc_name));
            }
        }

        let user_paused = ctl.pause.load(Ordering::SeqCst);
        if user_paused || !focus_ok {
            // Pause: release what we hold, remember it, re-press on resume
            // so e.g. a held "W" keeps walking after the pause.
            let held: PressedState = inj.pressed().clone();
            inj.release_all();
            let reason = if user_paused {
                PauseReason::User
            } else {
                PauseReason::WaitingForWindow(focus.clone().unwrap_or_default())
            };
            set_state(status, waker, RunState::Paused(reason));
            let pause_start = platform::now_us();
            loop {
                if ctl.stop.load(Ordering::SeqCst) {
                    return FinishReason::Stopped;
                }
                std::thread::sleep(Duration::from_millis(20));
                platform::pump_thread_messages();
                let still_user = ctl.pause.load(Ordering::SeqCst);
                if let Some(p) = &focus {
                    focus_ok = platform::foreground_window().is_some_and(|w| w.matches_process(p));
                }
                if !still_user && focus_ok {
                    break;
                }
                // Switch displayed reason if it changed.
                let want = if still_user {
                    PauseReason::User
                } else {
                    PauseReason::WaitingForWindow(focus.clone().unwrap_or_default())
                };
                let mut s = status.lock();
                if s.state != RunState::Paused(want.clone()) {
                    s.state = RunState::Paused(want);
                    drop(s);
                    waker();
                }
            }
            paused_total += platform::now_us() - pause_start;
            inj.repress(&held);
            set_state(status, waker, RunState::Playing);
            continue;
        }

        let run_now = platform::now_us() - real_start - paused_total;
        match player.step(run_now) {
            Step::Fire { index } => {
                inj.inject(&events[index].kind, clicks_only);
                injected += 1;
            }
            Step::WaitUntil { t_us } => {
                // Never sleep longer than 50 ms so stop/pause stay responsive.
                let target = (t_us + real_start + paused_total).min(platform::now_us() + 50_000);
                sleeper.wait_until(target);
            }
            Step::PassComplete { .. } => {
                // Nothing may stay held between passes (RESEARCH §1 #5).
                inj.release_all();
            }
            Step::Done => {
                update_status(status, &player, run_now, injected, inj.failures());
                return FinishReason::Completed;
            }
        }

        let real_now = platform::now_us();
        if real_now.saturating_sub(last_status) >= 16_000 {
            last_status = real_now;
            // Injecting from a thread that never pumps messages lets per-
            // thread input bookkeeping pile up over a long run (measured:
            // ~220 bytes per injected move). Drain it regularly.
            platform::pump_thread_messages();
            update_status(status, &player, run_now, injected, inj.failures());
            if real_now.saturating_sub(last_wake) >= 100_000 {
                last_wake = real_now;
                waker();
            }
        }
    }
}

fn update_status(status: &Mutex<PlayStatus>, player: &Player, run_now: u64, injected: u64, failures: u32) {
    let mut s = status.lock();
    s.pass = player.current_pass().min(player.total_passes().unwrap_or(u32::MAX));
    s.elapsed_us = player.elapsed_us(run_now);
    s.pass_progress = player.pass_progress(run_now);
    s.injected = injected;
    s.shifted_us = player.total_shift_us();
    s.blocked_inputs = failures;
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_core::{EventKind, Key, Repeat};

    fn waker() -> Waker {
        Arc::new(|| {})
    }

    fn sample() -> Arc<Vec<Event>> {
        let k = Key::from_vk(0x41);
        Arc::new(vec![
            Event::new(0, EventKind::Key { key: k, down: true }),
            Event::new(20_000, EventKind::Key { key: k, down: false }),
            Event::new(40_000, EventKind::MouseMove { x: 5, y: 5 }),
        ])
    }

    fn req(opts: PlaybackOptions) -> PlayRequest {
        PlayRequest {
            events: sample(),
            opts,
            recorded_screen: None,
            focus_process: None,
            countdown_secs: 0,
            keep_awake: false,
            wait_for_modifiers: false,
        }
    }

    #[test]
    fn plays_all_passes_and_reports_completion() {
        let opts = PlaybackOptions {
            repeat: Repeat::Times { count: 3 },
            ..Default::default()
        };
        let h = start(req(opts), waker());
        let s = h.join();
        assert_eq!(s.state, RunState::Finished(FinishReason::Completed));
        assert_eq!(s.injected, 9);
        assert_eq!(s.pass, 3);
        // 3 passes of 40 ms.
        assert!(s.elapsed_us >= 120_000, "{}", s.elapsed_us);
        assert!(s.elapsed_us < 400_000, "{}", s.elapsed_us);
    }

    #[test]
    fn stop_ends_forever_run_quickly() {
        let opts = PlaybackOptions {
            repeat: Repeat::Forever,
            ..Default::default()
        };
        let h = start(req(opts), waker());
        std::thread::sleep(Duration::from_millis(150));
        let t = std::time::Instant::now();
        h.stop();
        let s = h.join();
        assert!(t.elapsed() < Duration::from_millis(200));
        assert_eq!(s.state, RunState::Finished(FinishReason::Stopped));
    }

    #[test]
    fn pause_freezes_the_run_clock() {
        let opts = PlaybackOptions::default();
        let mut r = req(opts);
        r.events = Arc::new(vec![
            Event::new(0, EventKind::MouseMove { x: 0, y: 0 }),
            Event::new(100_000, EventKind::MouseMove { x: 1, y: 1 }),
        ]);
        let h = start(r, waker());
        std::thread::sleep(Duration::from_millis(30));
        h.toggle_pause();
        std::thread::sleep(Duration::from_millis(60));
        assert!(matches!(h.status().state, RunState::Paused(PauseReason::User)));
        std::thread::sleep(Duration::from_millis(150));
        // Still paused: the second event must not have fired.
        assert_eq!(h.status().injected, 1);
        h.toggle_pause();
        let s = h.join();
        assert_eq!(s.state, RunState::Finished(FinishReason::Completed));
        // Elapsed excludes the pause.
        assert!(s.elapsed_us < 180_000, "{}", s.elapsed_us);
    }

    #[test]
    fn countdown_can_be_cancelled() {
        let mut r = req(PlaybackOptions::default());
        r.countdown_secs = 5;
        let h = start(r, waker());
        std::thread::sleep(Duration::from_millis(120));
        assert!(matches!(h.status().state, RunState::Countdown { .. }));
        h.stop();
        assert_eq!(h.join().state, RunState::Finished(FinishReason::Stopped));
    }

    #[test]
    fn activity_abort_reports_reason() {
        let opts = PlaybackOptions {
            repeat: Repeat::Forever,
            ..Default::default()
        };
        let h = start(req(opts), waker());
        std::thread::sleep(Duration::from_millis(50));
        h.abort_for_activity("the mouse was moved");
        assert_eq!(
            h.join().state,
            RunState::Finished(FinishReason::UserActivity("the mouse was moved".into()))
        );
    }
}
