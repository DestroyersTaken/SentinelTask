//! The auto-clicker thread.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;

use sentinel_core::clicker::{ClickAction, ClickLocation, ClickerConfig};
use sentinel_core::keys::{vk, Hotkey};
use sentinel_core::{EventKind, Key, MouseMode};

use crate::platform::{self, Injector, Sleeper, Waker};

pub struct ClickerHandle {
    stop: Arc<AtomicBool>,
    clicks: Arc<AtomicU64>,
    started_us: u64,
    thread: Option<JoinHandle<()>>,
}

impl ClickerHandle {
    pub fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }
    pub fn clicks(&self) -> u64 {
        self.clicks.load(Ordering::Relaxed)
    }
    pub fn elapsed_us(&self) -> u64 {
        platform::now_us().saturating_sub(self.started_us)
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    pub fn join(mut self) -> u64 {
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        self.clicks()
    }
}

impl Drop for ClickerHandle {
    fn drop(&mut self) {
        self.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn hotkey_keys(h: &Hotkey) -> Vec<Key> {
    let mut v = Vec::new();
    if h.ctrl {
        v.push(Key::new(vk::LCONTROL, 0x1D, false));
    }
    if h.alt {
        v.push(Key::new(vk::LMENU, 0x38, false));
    }
    if h.shift {
        v.push(Key::new(vk::LSHIFT, 0x2A, false));
    }
    if h.win {
        v.push(Key::new(vk::LWIN, 0x5B, true));
    }
    v
}

pub fn start(cfg: ClickerConfig, waker: Waker) -> ClickerHandle {
    let stop = Arc::new(AtomicBool::new(false));
    let clicks = Arc::new(AtomicU64::new(0));
    let (s2, c2) = (stop.clone(), clicks.clone());
    let started_us = platform::now_us();
    let thread = std::thread::Builder::new()
        .name("sentinel-clicker".into())
        .spawn(move || {
            platform::boost_thread_priority(true);
            let mut sleeper = Sleeper::new();
            // A fixed spot is clicked with the position inside the same input
            // event as the button (atomic). "Wherever the cursor is" never
            // moves the mouse, so it can't fight the user's hand.
            let fixed = match cfg.location {
                ClickLocation::Fixed { x, y } => Some((x, y)),
                ClickLocation::Cursor => None,
            };
            let mut inj = Injector::new(if fixed.is_some() {
                MouseMode::Absolute
            } else {
                MouseMode::Relative
            });
            // Wait for the user to let go of the hotkey's modifiers FIRST, then
            // anchor the schedule — otherwise the wait would be "caught up"
            // with a burst of clicks.
            platform::wait_modifiers_released(std::time::Duration::from_secs(2));
            let origin = platform::now_us() + 50_000;
            let interval = cfg.interval_us();
            let hold = cfg.hold_us();
            let mut due = origin;
            let mut k: u64 = 0;
            loop {
                if !cfg.allows(origin, k, due) || s2.load(Ordering::SeqCst) {
                    break;
                }
                // Sleep in short slices so Stop is instant.
                while platform::now_us() < due {
                    if s2.load(Ordering::SeqCst) {
                        return;
                    }
                    sleeper.wait_until(due.min(platform::now_us() + 20_000));
                }
                // If the PC stalled for more than one interval, resume from now
                // instead of firing the missed clicks in a burst.
                let now = platform::now_us();
                if now > due + interval {
                    due = now;
                }
                let (cx, cy) = fixed.unwrap_or_else(platform::cursor_pos);
                match cfg.action {
                    ClickAction::Mouse { button, double } => {
                        let presses = if double { 2 } else { 1 };
                        for _ in 0..presses {
                            inj.inject(
                                &EventKind::MouseButton {
                                    button,
                                    down: true,
                                    x: cx,
                                    y: cy,
                                },
                                false,
                            );
                            sleeper.wait_until(platform::now_us() + hold);
                            inj.inject(
                                &EventKind::MouseButton {
                                    button,
                                    down: false,
                                    x: cx,
                                    y: cy,
                                },
                                false,
                            );
                            if double {
                                sleeper.wait_until(platform::now_us() + hold);
                            }
                        }
                    }
                    ClickAction::Key { key } => {
                        let mods = hotkey_keys(&key);
                        for m in &mods {
                            inj.inject(&EventKind::Key { key: *m, down: true }, false);
                        }
                        let main = Key::from_vk(key.vk);
                        inj.inject(&EventKind::Key { key: main, down: true }, false);
                        sleeper.wait_until(platform::now_us() + hold);
                        inj.inject(&EventKind::Key { key: main, down: false }, false);
                        for m in mods.iter().rev() {
                            inj.inject(&EventKind::Key { key: *m, down: false }, false);
                        }
                    }
                }
                k += 1;
                due += interval;
                c2.store(k, Ordering::Relaxed);
                if k % 16 == 0 {
                    platform::pump_thread_messages();
                }
                if k % 10 == 0 {
                    waker();
                }
            }
            waker();
        })
        .expect("spawn clicker thread");
    ClickerHandle {
        stop,
        clicks,
        started_us,
        thread: Some(thread),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_core::clicker::ClickLimit;

    #[test]
    fn count_limit_is_respected() {
        let cfg = ClickerConfig {
            interval_ms: 5,
            hold_ms: 1,
            limit: ClickLimit::Count { clicks: 7 },
            ..Default::default()
        };
        let h = start(cfg, Arc::new(|| {}));
        assert_eq!(h.join(), 7);
    }

    #[test]
    fn stop_is_prompt() {
        let cfg = ClickerConfig {
            interval_ms: 1000,
            ..Default::default()
        };
        let h = start(cfg, Arc::new(|| {}));
        std::thread::sleep(std::time::Duration::from_millis(80));
        let t = std::time::Instant::now();
        h.stop();
        h.join();
        assert!(t.elapsed() < std::time::Duration::from_millis(100));
    }
}
