//! Non-Windows stand-in used for development and CI. It mirrors the
//! Windows API surface but captures and injects nothing.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};

use sentinel_core::coords::Rect;
use sentinel_core::settings::Hotkeys;
use sentinel_core::state::PressedState;
use sentinel_core::{EventKind, MouseMode, ScreenInfo};

use super::{CaptureConfig, Captured, HotkeyAction, HotkeyRegistration, PlatformEvent, Waker, WindowInfo};

pub fn init_process() {
    let _ = epoch();
}

pub fn attach_parent_console() -> bool {
    true
}

pub fn show_error_box(title: &str, text: &str) {
    eprintln!("{title}: {text}");
}

fn epoch() -> Instant {
    static E: OnceLock<Instant> = OnceLock::new();
    *E.get_or_init(Instant::now)
}

pub fn now_us() -> u64 {
    epoch().elapsed().as_micros() as u64
}

pub struct Sleeper;

impl Sleeper {
    pub fn new() -> Self {
        Sleeper
    }
    pub fn kind(&self) -> &'static str {
        "std::thread::sleep (development build)"
    }
    pub fn wait_until(&mut self, deadline: u64) {
        let now = now_us();
        if deadline > now {
            std::thread::sleep(Duration::from_micros(deadline - now));
        }
    }
}

impl Default for Sleeper {
    fn default() -> Self {
        Self::new()
    }
}

pub fn boost_thread_priority(_critical: bool) {}

pub fn screen_info() -> ScreenInfo {
    ScreenInfo {
        desktop: Rect::new(0, 0, 1920, 1080),
        primary: Rect::new(0, 0, 1920, 1080),
        dpi: 96,
    }
}

pub fn cursor_pos() -> (i32, i32) {
    (960, 540)
}

pub fn process_is_elevated() -> bool {
    false
}

pub fn foreground_window() -> Option<WindowInfo> {
    Some(WindowInfo {
        pid: std::process::id(),
        process: "SentinelTask".into(),
        title: "SentinelTask".into(),
        elevated: Some(false),
    })
}

pub fn own_pid() -> u32 {
    std::process::id()
}

pub fn restart_as_admin(_args: &[String]) -> Result<(), String> {
    Err("Only available on Windows".into())
}

pub fn keep_awake(_on: bool) {}

pub fn wait_modifiers_released(_timeout: Duration) -> bool {
    true
}

pub fn local_timestamp() -> String {
    let secs = sentinel_core::util::now_unix();
    format!("{secs}")
}

pub fn open_file_dialog(_t: &str, _f: &[(&str, &[&str])], _d: Option<&Path>) -> Option<PathBuf> {
    None
}

pub fn save_file_dialog(_t: &str, _f: &[(&str, &[&str])], _d: Option<&Path>, _n: &str) -> Option<PathBuf> {
    None
}

pub fn pick_folder_dialog(_t: &str, _d: Option<&Path>) -> Option<PathBuf> {
    None
}

pub fn reveal_in_explorer(_path: &Path) {}

pub fn open_folder(_path: &Path) {}

/// Records what it would have injected (useful in tests).
pub struct Injector {
    mode: MouseMode,
    pressed: PressedState,
    pub sent: Vec<EventKind>,
}

impl Injector {
    pub fn new(mode: MouseMode) -> Self {
        Self {
            mode,
            pressed: PressedState::new(),
            sent: Vec::new(),
        }
    }
    pub fn failures(&self) -> u32 {
        0
    }
    pub fn pressed(&self) -> &PressedState {
        &self.pressed
    }
    pub fn inject(&mut self, kind: &EventKind, clicks_only: bool) {
        let skip = match kind {
            EventKind::MouseMove { .. } => self.mode != MouseMode::Absolute || clicks_only,
            EventKind::MouseDelta { .. } => self.mode != MouseMode::Relative || clicks_only,
            _ => false,
        };
        if !skip {
            self.sent.push(*kind);
        }
        self.pressed.apply(kind);
    }
    pub fn release_all(&mut self) {
        let r = self.pressed.drain_releases();
        self.sent.extend(r);
    }
    pub fn repress(&mut self, held: &PressedState) {
        for k in held.held_keys() {
            let e = EventKind::Key { key: *k, down: true };
            self.sent.push(e);
            self.pressed.apply(&e);
        }
    }
}

impl Drop for Injector {
    fn drop(&mut self) {
        self.release_all();
    }
}

pub fn emergency_release_all() {}

pub struct Service {
    _event_tx: Sender<PlatformEvent>,
    _capture_tx: Sender<Captured>,
}

impl Service {
    pub fn start(_waker: Waker) -> (Service, Receiver<PlatformEvent>, Receiver<Captured>) {
        let (event_tx, event_rx) = unbounded();
        let (capture_tx, capture_rx) = unbounded();
        (
            Service {
                _event_tx: event_tx,
                _capture_tx: capture_tx,
            },
            event_rx,
            capture_rx,
        )
    }

    pub fn register_hotkeys(&self, hk: &Hotkeys) -> Vec<HotkeyRegistration> {
        HotkeyAction::ALL
            .into_iter()
            .map(|a| HotkeyRegistration {
                action: a,
                hotkey: a.of(hk),
                error: a.of(hk).validate().err(),
            })
            .collect()
    }
    pub fn suspend_hotkeys(&self, _on: bool) {}
    pub fn set_capture(&self, _cfg: Option<CaptureConfig>) {}
    pub fn set_activity_watch(&self, _mouse: bool, _keys: bool) {}
    pub fn begin_key_capture(&self, _on: bool) {}
    pub fn begin_point_pick(&self, _on: bool) {}
    pub fn is_functional(&self) -> bool {
        false
    }
}

pub fn install_console_handler() {}

pub fn console_interrupted() -> bool {
    false
}

pub fn set_virtual_key_injection(_on: bool) {}

pub fn point_is_own_window(_x: i32, _y: i32) -> bool {
    false
}

pub fn simulate_user_keys(_keys: &[(u16, bool)]) {}

pub const OVERLAY_TITLE: &str = "SentinelTask status";

pub fn make_overlay_unfocusable() {}
