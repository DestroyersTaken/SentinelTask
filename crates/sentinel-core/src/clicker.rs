//! The built-in auto-clicker: configuration and drift-free scheduling.

use serde::{Deserialize, Serialize};

use crate::keys::Hotkey;
use crate::model::MouseButton;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClickAction {
    Mouse {
        button: MouseButton,
        double: bool,
    },
    /// Press a key (the `vk` of the hotkey, with its modifiers held).
    Key {
        key: Hotkey,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClickLocation {
    /// Wherever the cursor currently is.
    Cursor,
    /// A fixed point in physical virtual-desktop pixels.
    Fixed { x: i32, y: i32 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ClickLimit {
    Unlimited,
    Count { clicks: u64 },
    Duration { secs: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClickerConfig {
    pub action: ClickAction,
    /// Time between the starts of consecutive clicks.
    pub interval_ms: u64,
    /// How long the button/key is held for each click.
    pub hold_ms: u64,
    pub location: ClickLocation,
    pub limit: ClickLimit,
}

impl Default for ClickerConfig {
    fn default() -> Self {
        Self {
            action: ClickAction::Mouse {
                button: MouseButton::Left,
                double: false,
            },
            interval_ms: 100,
            hold_ms: 20,
            location: ClickLocation::Cursor,
            limit: ClickLimit::Unlimited,
        }
    }
}

pub const MIN_INTERVAL_MS: u64 = 1;

impl ClickerConfig {
    pub fn interval_us(&self) -> u64 {
        self.interval_ms.max(MIN_INTERVAL_MS) * 1000
    }

    /// Hold time, kept shorter than the interval so presses never overlap.
    pub fn hold_us(&self) -> u64 {
        let interval = self.interval_us();
        let per_press = if matches!(self.action, ClickAction::Mouse { double: true, .. }) {
            interval / 4
        } else {
            interval / 2
        };
        (self.hold_ms * 1000).min(per_press)
    }

    /// Run-clock time at which click number `k` (0-based) starts.
    pub fn due_us(&self, start_us: u64, k: u64) -> u64 {
        start_us + k * self.interval_us()
    }

    /// Whether click `k`, due at `due`, should still happen.
    pub fn allows(&self, start_us: u64, k: u64, due: u64) -> bool {
        match self.limit {
            ClickLimit::Unlimited => true,
            ClickLimit::Count { clicks } => k < clicks,
            ClickLimit::Duration { secs } => due.saturating_sub(start_us) < secs * 1_000_000,
        }
    }

    pub fn clicks_per_second(&self) -> f64 {
        1_000_000.0 / self.interval_us() as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schedule_is_absolute() {
        let c = ClickerConfig {
            interval_ms: 50,
            ..Default::default()
        };
        assert_eq!(c.due_us(1000, 0), 1000);
        assert_eq!(c.due_us(1000, 100), 1000 + 5_000_000);
        assert!((c.clicks_per_second() - 20.0).abs() < 1e-9);
    }

    #[test]
    fn limits() {
        let mut c = ClickerConfig {
            limit: ClickLimit::Count { clicks: 3 },
            ..Default::default()
        };
        assert!(c.allows(0, 2, 0));
        assert!(!c.allows(0, 3, 0));
        c.limit = ClickLimit::Duration { secs: 1 };
        assert!(c.allows(0, 9, 999_999));
        assert!(!c.allows(0, 10, 1_000_000));
    }

    #[test]
    fn hold_never_overlaps_next_click() {
        let c = ClickerConfig {
            interval_ms: 10,
            hold_ms: 500,
            ..Default::default()
        };
        assert_eq!(c.hold_us(), 5_000);
        let d = ClickerConfig {
            interval_ms: 0,
            action: ClickAction::Mouse {
                button: MouseButton::Left,
                double: true,
            },
            ..c
        };
        assert_eq!(d.interval_us(), 1000);
        assert_eq!(d.hold_us(), 250);
    }
}
