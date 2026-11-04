//! Operating-system integration.
//!
//! The real implementation is Windows-only (`windows.rs`). A small stub
//! (`stub.rs`) lets the UI and engine build and run on other systems for
//! development, screenshots and CI tests; it records nothing and injects
//! nothing.

use sentinel_core::keys::Hotkey;
use sentinel_core::EventKind;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use self::windows::*;

#[cfg(not(windows))]
mod stub;
#[cfg(not(windows))]
pub use self::stub::*;

/// `dwExtraInfo` tag on every event SentinelTask injects, so our own hooks
/// can tell our input apart from the user's and from other programs'
/// (RESEARCH §1 #7).
#[cfg_attr(not(windows), allow(dead_code))]
pub const INJECT_MAGIC: usize = 0x5354_4B21; // "STK!"

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HotkeyAction {
    Record,
    Play,
    Pause,
    EmergencyStop,
    Clicker,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl HotkeyAction {
    pub const ALL: [HotkeyAction; 5] = [
        HotkeyAction::Record,
        HotkeyAction::Play,
        HotkeyAction::Pause,
        HotkeyAction::EmergencyStop,
        HotkeyAction::Clicker,
    ];

    pub fn id(self) -> i32 {
        match self {
            HotkeyAction::Record => 1,
            HotkeyAction::Play => 2,
            HotkeyAction::Pause => 3,
            HotkeyAction::EmergencyStop => 4,
            HotkeyAction::Clicker => 5,
        }
    }

    pub fn from_id(id: i32) -> Option<Self> {
        Self::ALL.into_iter().find(|a| a.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            HotkeyAction::Record => "Record",
            HotkeyAction::Play => "Play",
            HotkeyAction::Pause => "Pause / resume",
            HotkeyAction::EmergencyStop => "Emergency stop",
            HotkeyAction::Clicker => "Auto-clicker",
        }
    }

    pub fn of(self, h: &sentinel_core::settings::Hotkeys) -> Hotkey {
        match self {
            HotkeyAction::Record => h.record,
            HotkeyAction::Play => h.play,
            HotkeyAction::Pause => h.pause,
            HotkeyAction::EmergencyStop => h.emergency_stop,
            HotkeyAction::Clicker => h.clicker,
        }
    }
}

/// An input event captured while recording; `t_us` is on the [`now_us`] clock.
#[derive(Debug, Clone, Copy)]
pub struct Captured {
    pub t_us: u64,
    pub kind: EventKind,
}

#[derive(Debug, Clone)]
#[cfg_attr(not(windows), allow(dead_code))]
pub enum PlatformEvent {
    Hotkey(HotkeyAction),
    /// The user physically used the mouse or keyboard during playback.
    UserActivity(&'static str),
    /// Result of [`Service::begin_key_capture`]; `None` = cancelled.
    KeyCaptured(Option<Hotkey>),
    /// Result of [`Service::begin_point_pick`]; `None` = cancelled.
    PointPicked(Option<(i32, i32)>),
}

/// What to capture while recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CaptureConfig {
    pub mouse_moves: bool,
    pub raw_deltas: bool,
    pub buttons: bool,
    pub keyboard: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyRegistration {
    pub action: HotkeyAction,
    pub hotkey: Hotkey,
    /// `None` when registered successfully.
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowInfo {
    pub pid: u32,
    /// Executable file name, e.g. `notepad.exe`.
    pub process: String,
    pub title: String,
    /// `Some(true)` if the window's process runs elevated (as administrator).
    pub elevated: Option<bool>,
}

impl WindowInfo {
    pub fn matches_process(&self, process: &str) -> bool {
        !process.is_empty() && self.process.eq_ignore_ascii_case(process)
    }
}

/// Something that can be woken (the UI) when platform events arrive.
pub type Waker = std::sync::Arc<dyn Fn() + Send + Sync>;
