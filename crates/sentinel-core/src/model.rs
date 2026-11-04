//! The in-memory macro model.
//!
//! A macro is a list of timestamped input events. Time is stored in
//! microseconds from the start of the recording so that idle time is
//! preserved exactly (RESEARCH §1 #11) and so scheduling never has to deal
//! with tick wrap-around.

use serde::{Deserialize, Serialize};

use crate::coords::Rect;
use crate::playback::PlaybackOptions;

/// Mouse buttons we record and replay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    /// "Back" side button.
    X1,
    /// "Forward" side button.
    X2,
}

impl MouseButton {
    pub const ALL: [MouseButton; 5] = [
        MouseButton::Left,
        MouseButton::Right,
        MouseButton::Middle,
        MouseButton::X1,
        MouseButton::X2,
    ];

    pub fn code(self) -> u8 {
        match self {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
            MouseButton::X1 => 3,
            MouseButton::X2 => 4,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(code as usize).copied()
    }

    pub fn label(self) -> &'static str {
        match self {
            MouseButton::Left => "Left",
            MouseButton::Right => "Right",
            MouseButton::Middle => "Middle",
            MouseButton::X1 => "Back (X1)",
            MouseButton::X2 => "Forward (X2)",
        }
    }
}

/// A single keyboard key as Windows identifies it.
///
/// We keep both the virtual-key code (layout dependent, used for display and
/// for keys without a scan code such as some media keys) and the hardware
/// scan code plus extended flag (layout independent; what games read).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub vk: u16,
    pub scan: u16,
    pub extended: bool,
}

impl Key {
    pub fn new(vk: u16, scan: u16, extended: bool) -> Self {
        Self { vk, scan, extended }
    }

    /// A key known only by virtual-key code (scan code resolved at playback).
    pub fn from_vk(vk: u16) -> Self {
        Self {
            vk,
            scan: 0,
            extended: crate::keys::is_extended_vk(vk),
        }
    }

    /// Identity used for "is this key held" bookkeeping. Left and right
    /// variants and numpad/main Enter are distinct physical keys.
    pub fn identity(&self) -> (u16, bool) {
        if self.scan != 0 {
            (self.scan | 0x8000, self.extended)
        } else {
            (self.vk, self.extended)
        }
    }
}

/// What happened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventKind {
    /// Absolute cursor position in physical virtual-desktop pixels.
    MouseMove {
        x: i32,
        y: i32,
    },
    /// Raw relative mouse motion (Raw Input). Used by games / 3D cameras
    /// that lock the cursor and read deltas.
    MouseDelta {
        dx: i32,
        dy: i32,
    },
    MouseButton {
        button: MouseButton,
        down: bool,
        x: i32,
        y: i32,
    },
    /// Wheel rotation; `delta` is in Windows units (120 per notch).
    Wheel {
        delta: i32,
        horizontal: bool,
        x: i32,
        y: i32,
    },
    Key {
        key: Key,
        down: bool,
    },
}

impl EventKind {
    pub fn is_mouse_motion(&self) -> bool {
        matches!(self, EventKind::MouseMove { .. } | EventKind::MouseDelta { .. })
    }

    pub fn is_mouse(&self) -> bool {
        !matches!(self, EventKind::Key { .. })
    }

    /// Absolute position carried by the event, if any.
    pub fn position(&self) -> Option<(i32, i32)> {
        match *self {
            EventKind::MouseMove { x, y } | EventKind::MouseButton { x, y, .. } | EventKind::Wheel { x, y, .. } => {
                Some((x, y))
            }
            _ => None,
        }
    }

    pub fn set_position(&mut self, nx: i32, ny: i32) {
        match self {
            EventKind::MouseMove { x, y } | EventKind::MouseButton { x, y, .. } | EventKind::Wheel { x, y, .. } => {
                *x = nx;
                *y = ny;
            }
            _ => {}
        }
    }
}

/// A timestamped event. `t_us` is microseconds since the start of the macro.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Event {
    pub t_us: u64,
    pub kind: EventKind,
}

impl Event {
    pub fn new(t_us: u64, kind: EventKind) -> Self {
        Self { t_us, kind }
    }
}

/// Information about the window the macro was recorded against. Used by the
/// focus guard (RESEARCH §1 #14).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TargetWindow {
    /// Executable file name, e.g. `RobloxPlayerBeta.exe`.
    pub process: String,
    /// Window title at recording time (informational; titles change).
    #[serde(default)]
    pub title: String,
}

/// Screen layout the macro was recorded on (RESEARCH §1 #1, #2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScreenInfo {
    /// Virtual desktop bounds in physical pixels (may have negative origin).
    pub desktop: Rect,
    /// Primary monitor bounds in physical pixels.
    pub primary: Rect,
    /// Primary monitor DPI (96 = 100 %).
    #[serde(default = "default_dpi")]
    pub dpi: u32,
}

fn default_dpi() -> u32 {
    96
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MacroMeta {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Unix seconds.
    #[serde(default)]
    pub created: u64,
    /// Unix seconds.
    #[serde(default)]
    pub modified: u64,
    #[serde(default)]
    pub app_version: String,
    #[serde(default)]
    pub screen: Option<ScreenInfo>,
    #[serde(default)]
    pub target: Option<TargetWindow>,
    /// Where this macro came from ("recorded", "imported from file.rec", ...).
    #[serde(default)]
    pub source: String,
}

/// A complete macro: metadata, per-macro playback defaults and events.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Macro {
    pub meta: MacroMeta,
    pub playback: PlaybackOptions,
    pub events: Vec<Event>,
}

/// Quick statistics for display in the library and editor.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MacroStats {
    pub events: usize,
    pub moves: usize,
    pub deltas: usize,
    pub clicks: usize,
    pub wheels: usize,
    pub key_presses: usize,
    pub duration_us: u64,
}

impl Macro {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            meta: MacroMeta {
                name: name.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    /// Duration of one pass in microseconds (time of the last event).
    pub fn duration_us(&self) -> u64 {
        self.events.last().map(|e| e.t_us).unwrap_or(0)
    }

    pub fn has_mouse_events(&self) -> bool {
        self.events.iter().any(|e| e.kind.is_mouse())
    }

    pub fn has_absolute_mouse(&self) -> bool {
        self.events.iter().any(|e| e.kind.position().is_some())
    }

    pub fn has_deltas(&self) -> bool {
        self.events
            .iter()
            .any(|e| matches!(e.kind, EventKind::MouseDelta { .. }))
    }

    /// Guess the right mouse mode from the recording itself.
    ///
    /// Games with a locked cursor (3D cameras) produce plenty of raw motion
    /// while the reported cursor position barely changes, because the game
    /// keeps re-centring it. Replaying absolute positions there does
    /// nothing, so such recordings should default to relative mode.
    pub fn suggested_mouse_mode(&self) -> crate::playback::MouseMode {
        use crate::playback::MouseMode;
        let mut raw_travel: i64 = 0;
        let (mut min_x, mut min_y, mut max_x, mut max_y) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
        let mut saw_pos = false;
        for e in &self.events {
            match e.kind {
                EventKind::MouseDelta { dx, dy } => raw_travel += (dx.abs() + dy.abs()) as i64,
                EventKind::MouseMove { x, y } => {
                    saw_pos = true;
                    min_x = min_x.min(x);
                    max_x = max_x.max(x);
                    min_y = min_y.min(y);
                    max_y = max_y.max(y);
                }
                _ => {}
            }
        }
        if raw_travel < 600 {
            return MouseMode::Absolute;
        }
        let spread = if saw_pos {
            ((max_x - min_x) as i64).max((max_y - min_y) as i64)
        } else {
            0
        };
        if spread * 8 < raw_travel && spread < 120 {
            MouseMode::Relative
        } else {
            MouseMode::Absolute
        }
    }

    pub fn stats(&self) -> MacroStats {
        let mut s = MacroStats {
            events: self.events.len(),
            duration_us: self.duration_us(),
            ..Default::default()
        };
        for e in &self.events {
            match e.kind {
                EventKind::MouseMove { .. } => s.moves += 1,
                EventKind::MouseDelta { .. } => s.deltas += 1,
                EventKind::MouseButton { down: true, .. } => s.clicks += 1,
                EventKind::Wheel { .. } => s.wheels += 1,
                EventKind::Key { down: true, .. } => s.key_presses += 1,
                _ => {}
            }
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn button_codes_roundtrip() {
        for b in MouseButton::ALL {
            assert_eq!(MouseButton::from_code(b.code()), Some(b));
        }
        assert_eq!(MouseButton::from_code(9), None);
    }

    #[test]
    fn key_identity_distinguishes_numpad_enter() {
        let main_enter = Key::new(0x0D, 0x1C, false);
        let pad_enter = Key::new(0x0D, 0x1C, true);
        assert_ne!(main_enter.identity(), pad_enter.identity());
    }

    #[test]
    fn locked_cursor_recordings_suggest_relative_mode() {
        use crate::playback::MouseMode;
        let mut m = Macro::new("game");
        for i in 0..200u64 {
            m.events
                .push(Event::new(i * 1000, EventKind::MouseDelta { dx: 15, dy: -3 }));
            m.events.push(Event::new(
                i * 1000 + 1,
                EventKind::MouseMove {
                    x: 960 + (i % 3) as i32,
                    y: 540,
                },
            ));
        }
        assert_eq!(m.suggested_mouse_mode(), MouseMode::Relative);
        let mut d = Macro::new("desktop");
        for i in 0..200u64 {
            d.events
                .push(Event::new(i * 1000, EventKind::MouseDelta { dx: 5, dy: 0 }));
            d.events.push(Event::new(
                i * 1000 + 1,
                EventKind::MouseMove {
                    x: 10 + 5 * i as i32,
                    y: 540,
                },
            ));
        }
        assert_eq!(d.suggested_mouse_mode(), MouseMode::Absolute);
        assert_eq!(Macro::new("empty").suggested_mouse_mode(), MouseMode::Absolute);
    }

    #[test]
    fn stats_count_presses_not_releases() {
        let k = Key::from_vk(0x41);
        let mut m = Macro::new("t");
        m.events = vec![
            Event::new(0, EventKind::Key { key: k, down: true }),
            Event::new(10, EventKind::Key { key: k, down: false }),
            Event::new(
                20,
                EventKind::MouseButton {
                    button: MouseButton::Left,
                    down: true,
                    x: 1,
                    y: 1,
                },
            ),
            Event::new(
                30,
                EventKind::MouseButton {
                    button: MouseButton::Left,
                    down: false,
                    x: 1,
                    y: 1,
                },
            ),
        ];
        let s = m.stats();
        assert_eq!(s.key_presses, 1);
        assert_eq!(s.clicks, 1);
        assert_eq!(s.duration_us, 30);
    }
}
