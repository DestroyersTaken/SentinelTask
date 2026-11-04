//! Turning captured input into a clean macro.

use sentinel_core::keys::Hotkey;
use sentinel_core::normalize::finish_recording;
use sentinel_core::settings::RecordingSettings;
use sentinel_core::{Event, EventKind, TargetWindow};

use crate::platform::{CaptureConfig, Captured};

pub struct RecordingSession {
    start_us: u64,
    events: Vec<Event>,
    pub target: Option<TargetWindow>,
    pub clicks: usize,
    pub keys: usize,
    pub moves: usize,
    motion_interval_us: u64,
}

impl RecordingSession {
    pub fn new(start_us: u64, settings: &RecordingSettings) -> Self {
        Self {
            start_us,
            events: Vec::with_capacity(4096),
            target: None,
            clicks: 0,
            keys: 0,
            moves: 0,
            motion_interval_us: settings.motion_interval_us(),
        }
    }

    pub fn capture_config(settings: &RecordingSettings) -> CaptureConfig {
        CaptureConfig {
            mouse_moves: settings.record_mouse_moves,
            raw_deltas: settings.record_raw_deltas,
            buttons: true,
            keyboard: settings.record_keyboard,
        }
    }

    pub fn start_us(&self) -> u64 {
        self.start_us
    }

    /// Add a captured event; returns true if it was a "meaningful" event
    /// (click or key) — the caller may use that to identify the target window.
    pub fn push(&mut self, c: Captured) -> bool {
        if c.t_us < self.start_us {
            return false; // captured before the session officially started
        }
        let t = c.t_us - self.start_us;
        let meaningful = match c.kind {
            EventKind::MouseButton { down: true, .. } => {
                self.clicks += 1;
                true
            }
            EventKind::Key { down: true, .. } => {
                self.keys += 1;
                true
            }
            EventKind::MouseMove { .. } | EventKind::MouseDelta { .. } => {
                self.moves += 1;
                false
            }
            _ => false,
        };
        self.events.push(Event::new(t, c.kind));
        meaningful
    }

    pub fn len(&self) -> usize {
        self.events.len()
    }

    /// Finish: strip the stop hotkey, coalesce motion, balance presses.
    pub fn finish(mut self, stop_hotkey: Option<&Hotkey>) -> (Vec<Event>, Option<TargetWindow>) {
        finish_recording(&mut self.events, stop_hotkey, self.motion_interval_us);
        (self.events, self.target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_core::keys::vk;
    use sentinel_core::Key;

    #[test]
    fn session_rebases_times_and_strips_hotkey() {
        let s = RecordingSettings::default();
        let mut r = RecordingSession::new(1_000_000, &s);
        let k = |t, code, down| Captured {
            t_us: t,
            kind: EventKind::Key {
                key: Key::from_vk(code),
                down,
            },
        };
        assert!(!r.push(k(999_000, 0x41, true))); // before start: ignored
        assert!(r.push(k(1_100_000, 0x42, true)));
        r.push(k(1_200_000, 0x42, false));
        r.push(k(1_300_000, vk::PAUSE, true));
        let (ev, _) = r.finish(Some(&Hotkey::key(vk::PAUSE)));
        assert_eq!(ev.len(), 2);
        assert_eq!(ev[0].t_us, 100_000);
    }
}
