//! Windows implementation of input capture, injection and system services.
//!
//! Design notes (see docs/RESEARCH.md):
//! * Capture uses low-level hooks (`WH_KEYBOARD_LL`, `WH_MOUSE_LL`) plus Raw
//!   Input for relative mouse motion. Never journaling hooks (§3).
//! * Hooks run on a dedicated service thread with its own message loop.
//!   Callbacks only copy data into a lock-free channel and return, so
//!   Windows never removes them for exceeding `LowLevelHooksTimeout` (§1 #8).
//! * Injection uses `SendInput` with hardware scan codes (§1 #3) and tags
//!   every event with [`INJECT_MAGIC`] so our hooks can ignore it (§1 #7).
//! * Hotkeys use `RegisterHotKey`, which is device independent and reports
//!   conflicts with other applications (§1 #13, #21).

#![allow(clippy::upper_case_acronyms)]

use std::cell::RefCell;
use std::ffi::c_void;
use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicIsize, AtomicU32, AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Duration;

use crossbeam_channel::{unbounded, Receiver, Sender};

use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Media::{timeBeginPeriod, timeEndPeriod};
use windows_sys::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};
use windows_sys::Win32::System::Power::{
    SetThreadExecutionState, ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::System::Threading::{
    CreateWaitableTimerExW, GetCurrentProcess, GetCurrentProcessId, GetCurrentThread, OpenProcess, OpenProcessToken,
    QueryFullProcessImageNameW, SetThreadPriority, SetWaitableTimer, WaitForSingleObject,
    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    THREAD_PRIORITY_HIGHEST, THREAD_PRIORITY_TIME_CRITICAL,
};
use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK,
    RIDEV_REMOVE, RID_INPUT, RIM_TYPEMOUSE,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

use sentinel_core::coords::{to_absolute, Rect};
use sentinel_core::keys::{vk as vkc, Hotkey};
use sentinel_core::model::{EventKind, Key, MouseButton};
use sentinel_core::settings::Hotkeys;
use sentinel_core::state::PressedState;
use sentinel_core::{MouseMode, ScreenInfo};

use super::{
    CaptureConfig, Captured, HotkeyAction, HotkeyRegistration, PlatformEvent, Waker, WindowInfo, INJECT_MAGIC,
};

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

// ---------------------------------------------------------------------------
// Process setup
// ---------------------------------------------------------------------------

/// Must run before any window is created: become Per-Monitor-V2 DPI aware so
/// every coordinate we see and send is in physical pixels (RESEARCH §1 #1).
pub fn init_process() {
    unsafe {
        SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        // Named mutex the installer/uninstaller checks so it can ask the
        // user to close SentinelTask before replacing its files. The handle
        // is intentionally never closed; Windows releases it at exit.
        let name = wide("SentinelTask.Running");
        windows_sys::Win32::System::Threading::CreateMutexW(null(), 0, name.as_ptr());
    }
    let _ = qpc_freq();
}

/// Attach to the console of the parent process (for CLI use from a GUI-
/// subsystem executable). Returns true if attached.
pub fn attach_parent_console() -> bool {
    unsafe { AttachConsole(ATTACH_PARENT_PROCESS) != 0 }
}

pub fn show_error_box(title: &str, text: &str) {
    let t = wide(title);
    let m = wide(text);
    unsafe {
        MessageBoxW(null_mut(), m.as_ptr(), t.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

fn qpc_freq() -> i64 {
    static FREQ: OnceLock<i64> = OnceLock::new();
    *FREQ.get_or_init(|| {
        let mut f = 0i64;
        unsafe { QueryPerformanceFrequency(&mut f) };
        f.max(1)
    })
}

/// Monotonic microseconds from the high-resolution performance counter.
pub fn now_us() -> u64 {
    let mut c = 0i64;
    unsafe { QueryPerformanceCounter(&mut c) };
    let f = qpc_freq();
    // Split to avoid overflow of c * 1_000_000.
    ((c / f) * 1_000_000 + (c % f) * 1_000_000 / f) as u64
}

/// Precise sleeping (RESEARCH §1 #10): a high-resolution waitable timer for
/// the bulk of the wait and a short spin for the last stretch. Falls back to
/// `timeBeginPeriod(1)` + `Sleep` on systems older than Windows 10 1803.
pub struct Sleeper {
    timer: HANDLE,
    raised_period: bool,
}

unsafe impl Send for Sleeper {}

impl Sleeper {
    pub fn new() -> Self {
        let timer = unsafe {
            CreateWaitableTimerExW(
                null(),
                null(),
                CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                0x1F0003, // TIMER_ALL_ACCESS
            )
        };
        let raised_period = if timer.is_null() {
            unsafe { timeBeginPeriod(1) == 0 }
        } else {
            false
        };
        Self { timer, raised_period }
    }

    pub fn kind(&self) -> &'static str {
        if !self.timer.is_null() {
            "high-resolution waitable timer"
        } else if self.raised_period {
            "1 ms multimedia timer"
        } else {
            "default system timer"
        }
    }

    /// Sleep until `deadline` on the [`now_us`] clock.
    pub fn wait_until(&mut self, deadline: u64) {
        const SPIN_US: u64 = 1_200;
        loop {
            let now = now_us();
            if now >= deadline {
                return;
            }
            let remaining = deadline - now;
            if remaining <= SPIN_US {
                while now_us() < deadline {
                    std::hint::spin_loop();
                }
                return;
            }
            let sleep_us = remaining - SPIN_US;
            if !self.timer.is_null() {
                // Negative = relative, in 100 ns units.
                let due: i64 = -((sleep_us * 10) as i64);
                unsafe {
                    if SetWaitableTimer(self.timer, &due, 0, None, null(), 0) != 0 {
                        WaitForSingleObject(self.timer, u32::MAX); // INFINITE
                        continue;
                    }
                }
            }
            std::thread::sleep(Duration::from_micros(sleep_us.min(50_000)));
        }
    }
}

impl Default for Sleeper {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Sleeper {
    fn drop(&mut self) {
        unsafe {
            if !self.timer.is_null() {
                CloseHandle(self.timer);
            }
            if self.raised_period {
                timeEndPeriod(1);
            }
        }
    }
}

/// Raise the current thread's priority for steadier playback timing.
pub fn boost_thread_priority(critical: bool) {
    unsafe {
        SetThreadPriority(
            GetCurrentThread(),
            if critical {
                THREAD_PRIORITY_TIME_CRITICAL
            } else {
                THREAD_PRIORITY_HIGHEST
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Screen, cursor, windows, elevation
// ---------------------------------------------------------------------------

pub fn screen_info() -> ScreenInfo {
    unsafe {
        let desktop = Rect::new(
            GetSystemMetrics(SM_XVIRTUALSCREEN),
            GetSystemMetrics(SM_YVIRTUALSCREEN),
            GetSystemMetrics(SM_CXVIRTUALSCREEN),
            GetSystemMetrics(SM_CYVIRTUALSCREEN),
        );
        let primary = Rect::new(0, 0, GetSystemMetrics(SM_CXSCREEN), GetSystemMetrics(SM_CYSCREEN));
        let dpi = windows_sys::Win32::UI::HiDpi::GetDpiForSystem();
        ScreenInfo {
            desktop,
            primary,
            dpi: if dpi == 0 { 96 } else { dpi },
        }
    }
}

pub fn cursor_pos() -> (i32, i32) {
    let mut p = POINT { x: 0, y: 0 };
    unsafe { GetCursorPos(&mut p) };
    (p.x, p.y)
}

fn process_name(pid: u32) -> (String, Option<bool>) {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            // Can't even query it: almost always an elevated or protected
            // process seen from a non-elevated one.
            return (String::new(), if process_is_elevated() { None } else { Some(true) });
        }
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let name = if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
            let full = String::from_utf16_lossy(&buf[..len as usize]);
            Path::new(&full)
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(full)
        } else {
            String::new()
        };
        let elevated = token_elevated(h);
        CloseHandle(h);
        (name, elevated)
    }
}

unsafe fn token_elevated(process: HANDLE) -> Option<bool> {
    let mut token: HANDLE = null_mut();
    if OpenProcessToken(process, TOKEN_QUERY, &mut token) == 0 {
        return if GetLastError() == ERROR_ACCESS_DENIED {
            Some(true)
        } else {
            None
        };
    }
    let mut elev: TOKEN_ELEVATION = zeroed();
    let mut ret = 0u32;
    let ok = GetTokenInformation(
        token,
        TokenElevation,
        &mut elev as *mut _ as *mut c_void,
        size_of::<TOKEN_ELEVATION>() as u32,
        &mut ret,
    );
    CloseHandle(token);
    if ok != 0 {
        Some(elev.TokenIsElevated != 0)
    } else {
        None
    }
}

/// Whether SentinelTask itself runs as administrator.
pub fn process_is_elevated() -> bool {
    static ELEVATED: OnceLock<bool> = OnceLock::new();
    *ELEVATED.get_or_init(|| unsafe { token_elevated(GetCurrentProcess()).unwrap_or(false) })
}

pub fn foreground_window() -> Option<WindowInfo> {
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_null() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        let mut buf = [0u16; 512];
        let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        let title = String::from_utf16_lossy(&buf[..n.max(0) as usize]);
        let (process, elevated) = process_name(pid);
        Some(WindowInfo {
            pid,
            process,
            title,
            elevated,
        })
    }
}

pub fn own_pid() -> u32 {
    unsafe { GetCurrentProcessId() }
}

/// Relaunch elevated ("Run as administrator"). Returns `Ok` if the new
/// process was started; the caller should then exit.
pub fn restart_as_admin(args: &[String]) -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let exe_w = wide(&exe.to_string_lossy());
    let params = args
        .iter()
        .map(|a| format!("\"{}\"", a.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(" ");
    let params_w = wide(&params);
    let verb = wide("runas");
    let r = unsafe {
        ShellExecuteW(
            null_mut(),
            verb.as_ptr(),
            exe_w.as_ptr(),
            params_w.as_ptr(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if (r as isize) > 32 {
        Ok(())
    } else {
        Err("Windows did not start the elevated copy (the UAC prompt may have been cancelled).".into())
    }
}

/// Keep the PC and display awake while playing (RESEARCH §1 #12). Applies
/// to the calling thread; call `keep_awake(false)` from the same thread.
pub fn keep_awake(on: bool) {
    unsafe {
        if on {
            SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED);
        } else {
            SetThreadExecutionState(ES_CONTINUOUS);
        }
    }
}

fn key_down_now(code: u16) -> bool {
    unsafe { (GetAsyncKeyState(code as i32) as u16 & 0x8000) != 0 }
}

/// Wait (up to `timeout`) until the user has let go of Ctrl/Alt/Shift/Win,
/// so a macro started with a hotkey like Ctrl+Alt+Shift+P doesn't replay
/// its first keys as Ctrl+Alt+Shift+<key>.
pub fn wait_modifiers_released(timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    loop {
        let held = [vkc::SHIFT, vkc::CONTROL, vkc::MENU, vkc::LWIN, vkc::RWIN]
            .iter()
            .any(|&k| key_down_now(k));
        if !held {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

pub fn local_timestamp() -> String {
    unsafe {
        let mut t: SYSTEMTIME = zeroed();
        GetLocalTime(&mut t);
        format!(
            "{:04}-{:02}-{:02} {:02}.{:02}.{:02}",
            t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
        )
    }
}

// ---------------------------------------------------------------------------
// File dialogs and shell
// ---------------------------------------------------------------------------

pub fn open_file_dialog(title: &str, filters: &[(&str, &[&str])], dir: Option<&Path>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(title);
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.pick_file()
}

pub fn save_file_dialog(
    title: &str,
    filters: &[(&str, &[&str])],
    dir: Option<&Path>,
    file_name: &str,
) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(title).set_file_name(file_name);
    for (name, exts) in filters {
        d = d.add_filter(*name, exts);
    }
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.save_file()
}

pub fn pick_folder_dialog(title: &str, dir: Option<&Path>) -> Option<PathBuf> {
    let mut d = rfd::FileDialog::new().set_title(title);
    if let Some(dir) = dir {
        d = d.set_directory(dir);
    }
    d.pick_folder()
}

pub fn reveal_in_explorer(path: &Path) {
    let _ = std::process::Command::new("explorer.exe")
        .arg(format!("/select,{}", path.display()))
        .spawn();
}

pub fn open_folder(path: &Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

// ---------------------------------------------------------------------------
// Injection
// ---------------------------------------------------------------------------

/// Sends input with `SendInput`. Tracks everything it presses so it can
/// always release it (RESEARCH §1 #5).
pub struct Injector {
    desktop: Rect,
    mode: MouseMode,
    pressed: PressedState,
    last_abs: Option<(i32, i32)>,
    failures: u32,
}

/// Held state of the active injector, for emergency release from a panic
/// hook or at exit even if the playback thread is gone.
static GLOBAL_HELD: parking_lot::Mutex<Option<PressedState>> = parking_lot::const_mutex(None);

impl Injector {
    pub fn new(mode: MouseMode) -> Self {
        Self {
            desktop: screen_info().desktop,
            mode,
            pressed: PressedState::new(),
            last_abs: None,
            failures: 0,
        }
    }

    pub fn failures(&self) -> u32 {
        self.failures
    }

    pub fn pressed(&self) -> &PressedState {
        &self.pressed
    }

    fn send(&mut self, inputs: &[INPUT]) {
        if inputs.is_empty() {
            return;
        }
        let n = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
        if n as usize != inputs.len() {
            self.failures += 1;
        }
    }

    fn mouse_input(dx: i32, dy: i32, data: u32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: INJECT_MAGIC,
                },
            },
        }
    }

    fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: INJECT_MAGIC,
                },
            },
        }
    }

    fn move_abs(&mut self, x: i32, y: i32) {
        let (ax, ay) = to_absolute(x, y, self.desktop);
        let i = Self::mouse_input(
            ax,
            ay,
            0,
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
        );
        self.send(&[i]);
        self.last_abs = Some((x, y));
    }

    /// Put the cursor exactly on (x, y) before a click. `SendInput`'s
    /// normalised coordinates can land one pixel off on unusual layouts;
    /// correct small errors with `SetCursorPos` (which is pixel exact).
    fn ensure_at(&mut self, x: i32, y: i32) {
        if self.last_abs != Some((x, y)) {
            self.move_abs(x, y);
        }
        let (cx, cy) = cursor_pos();
        if (cx, cy) != (x, y) && (cx - x).abs() <= 2 && (cy - y).abs() <= 2 {
            unsafe { SetCursorPos(x, y) };
        }
    }

    fn sync_global(&self) {
        *GLOBAL_HELD.lock() = Some(self.pressed.clone());
    }

    /// Inject one event.
    pub fn inject(&mut self, kind: &EventKind, clicks_only: bool) {
        match *kind {
            EventKind::MouseMove { x, y } => {
                if self.mode == MouseMode::Absolute && !clicks_only {
                    self.move_abs(x, y);
                }
            }
            EventKind::MouseDelta { dx, dy } => {
                if self.mode == MouseMode::Relative && !clicks_only {
                    self.send(&[Self::mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE)]);
                }
            }
            EventKind::MouseButton { button, down, x, y } => {
                if self.mode == MouseMode::Absolute {
                    self.ensure_at(x, y);
                }
                self.button(button, down);
            }
            EventKind::Wheel {
                delta,
                horizontal,
                x,
                y,
            } => {
                if self.mode == MouseMode::Absolute && self.last_abs != Some((x, y)) {
                    self.move_abs(x, y);
                }
                let flag = if horizontal {
                    MOUSEEVENTF_HWHEEL
                } else {
                    MOUSEEVENTF_WHEEL
                };
                self.send(&[Self::mouse_input(0, 0, delta as u32, flag)]);
            }
            EventKind::Key { key, down } => self.key(key, down),
        }
        self.pressed.apply(kind);
        self.sync_global();
    }

    fn button(&mut self, button: MouseButton, down: bool) {
        let (flag, data) = match (button, down) {
            (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (MouseButton::X1, true) => (MOUSEEVENTF_XDOWN, XBUTTON1 as u32),
            (MouseButton::X1, false) => (MOUSEEVENTF_XUP, XBUTTON1 as u32),
            (MouseButton::X2, true) => (MOUSEEVENTF_XDOWN, XBUTTON2 as u32),
            (MouseButton::X2, false) => (MOUSEEVENTF_XUP, XBUTTON2 as u32),
        };
        self.send(&[Self::mouse_input(0, 0, data, flag)]);
    }

    fn key(&mut self, key: Key, down: bool) {
        self.send(&[key_to_input(key, down)]);
    }

    /// Release everything this injector pressed. Never touches the mouse
    /// position.
    pub fn release_all(&mut self) {
        let releases = self.pressed.drain_releases();
        for r in &releases {
            match *r {
                EventKind::MouseButton { button, .. } => self.button(button, false),
                EventKind::Key { key, .. } => self.key(key, false),
                _ => {}
            }
        }
        self.sync_global();
    }

    /// Re-press keys/buttons (after an automatic pause).
    pub fn repress(&mut self, held: &PressedState) {
        for k in held.held_keys() {
            self.key(*k, true);
            self.pressed.apply(&EventKind::Key { key: *k, down: true });
        }
        for b in held.held_buttons() {
            self.button(*b, true);
            self.pressed.apply(&EventKind::MouseButton {
                button: *b,
                down: true,
                x: 0,
                y: 0,
            });
        }
        self.sync_global();
    }

    /// Current cursor position (for relative-mode clicks and the clicker).
    pub fn move_to(&mut self, x: i32, y: i32) {
        self.ensure_at(x, y);
    }
}

impl Drop for Injector {
    fn drop(&mut self) {
        self.release_all();
        *GLOBAL_HELD.lock() = None;
    }
}

fn key_to_input(key: Key, down: bool) -> INPUT {
    let up = if down { 0 } else { KEYEVENTF_KEYUP };
    // Keys whose scan code sequences are special: send by virtual key.
    let by_vk = matches!(key.vk, vkc::PAUSE | vkc::SNAPSHOT | vkc::CANCEL)
        || (0xA6..=0xB7).contains(&key.vk)
        || force_vk_injection();
    let mut scan = key.scan;
    let mut extended = key.extended;
    if scan == 0 && !by_vk {
        let ex = unsafe { MapVirtualKeyW(key.vk as u32, MAPVK_VK_TO_VSC_EX) };
        scan = (ex & 0xFF) as u16;
        if ex & 0xFF00 == 0xE000 {
            extended = true;
        }
    }
    let ext = if extended { KEYEVENTF_EXTENDEDKEY } else { 0 };
    if by_vk || scan == 0 {
        Injector::key_input(key.vk, scan, up | ext)
    } else {
        // Hardware scan code: what DirectInput / Raw Input games read, and
        // independent of the keyboard layout (RESEARCH §1 #3).
        // wVk is ignored by Windows when KEYEVENTF_SCANCODE is set, but
        // filling it in helps compatibility layers that look at it.
        Injector::key_input(key.vk, scan, KEYEVENTF_SCANCODE | up | ext)
    }
}

/// Diagnostic switch: `SENTINEL_KEY_MODE=vk` sends virtual-key codes instead
/// of hardware scan codes (for troubleshooting unusual keyboard drivers).
fn force_vk_injection() -> bool {
    static ENV_VK: OnceLock<bool> = OnceLock::new();
    VK_INJECTION.load(Ordering::Relaxed)
        || *ENV_VK.get_or_init(|| std::env::var("SENTINEL_KEY_MODE").is_ok_and(|v| v.eq_ignore_ascii_case("vk")))
}

static VK_INJECTION: AtomicBool = AtomicBool::new(false);

/// Choose virtual-key (true) or hardware scan-code (false) key injection.
pub fn set_virtual_key_injection(on: bool) {
    VK_INJECTION.store(on, Ordering::Relaxed);
}

/// Release whatever the active injector holds (panic hook / exit path).
pub fn emergency_release_all() {
    let held = GLOBAL_HELD.lock().take();
    if let Some(mut held) = held {
        let releases = held.drain_releases();
        let mut inputs = Vec::new();
        for r in releases {
            match r {
                EventKind::MouseButton { button, .. } => {
                    let (flag, data) = match button {
                        MouseButton::Left => (MOUSEEVENTF_LEFTUP, 0),
                        MouseButton::Right => (MOUSEEVENTF_RIGHTUP, 0),
                        MouseButton::Middle => (MOUSEEVENTF_MIDDLEUP, 0),
                        MouseButton::X1 => (MOUSEEVENTF_XUP, XBUTTON1 as u32),
                        MouseButton::X2 => (MOUSEEVENTF_XUP, XBUTTON2 as u32),
                    };
                    inputs.push(Injector::mouse_input(0, 0, data, flag));
                }
                EventKind::Key { key, .. } => inputs.push(key_to_input(key, false)),
                _ => {}
            }
        }
        if !inputs.is_empty() {
            unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
        }
    }
}

// ---------------------------------------------------------------------------
// Service thread: hotkeys, hooks, raw input
// ---------------------------------------------------------------------------

/// Shared between the service thread and the hook callbacks (which have no
/// user-data pointer, so this must be global).
struct HookShared {
    capture_tx: Sender<Captured>,
    event_tx: Sender<PlatformEvent>,
    waker: Waker,
}

static SHARED: OnceLock<HookShared> = OnceLock::new();

static CAPTURING: AtomicBool = AtomicBool::new(false);
static CAP_MOVES: AtomicBool = AtomicBool::new(false);
static CAP_BUTTONS: AtomicBool = AtomicBool::new(false);
static CAP_KEYS: AtomicBool = AtomicBool::new(false);
static CAP_DELTAS: AtomicBool = AtomicBool::new(false);
static WATCH_MOUSE: AtomicBool = AtomicBool::new(false);
static WATCH_KEYS: AtomicBool = AtomicBool::new(false);
static KEY_CAPTURE: AtomicBool = AtomicBool::new(false);
static POINT_PICK: AtomicBool = AtomicBool::new(false);
static SWALLOW_UP: AtomicU32 = AtomicU32::new(0);
static ACTIVITY_ACCUM: AtomicI64 = AtomicI64::new(0);
static OWN_PID: AtomicU32 = AtomicU32::new(0);
/// Bitset of virtual keys used by our hotkeys (ignored by the activity watch).
static HOTKEY_VKS: [AtomicU64; 4] = [
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
    AtomicU64::new(0),
];
static KB_HOOK: AtomicIsize = AtomicIsize::new(0);
static MS_HOOK: AtomicIsize = AtomicIsize::new(0);

const ACTIVITY_THRESHOLD: i64 = 40;
/// `RAWMOUSE.usFlags` bit for absolute (tablet / remote desktop) motion.
const MOUSE_MOVE_ABSOLUTE: u16 = 0x01;
const WM_APP_CMD: u32 = WM_APP + 1;

fn hotkey_vk_set(code: u16) -> bool {
    let c = (code & 0xFF) as usize;
    HOTKEY_VKS[c / 64].load(Ordering::Relaxed) & (1u64 << (c % 64)) != 0
}

fn emit(ev: PlatformEvent) {
    if let Some(s) = SHARED.get() {
        let _ = s.event_tx.send(ev);
        (s.waker)();
    }
}

fn capture(kind: EventKind) {
    if let Some(s) = SHARED.get() {
        let _ = s.capture_tx.send(Captured { t_us: now_us(), kind });
    }
}

/// Root window under the point belongs to SentinelTask (and isn't a
/// click-through overlay)?
unsafe fn point_is_own_window(pt: POINT) -> bool {
    let hwnd = WindowFromPoint(pt);
    if hwnd.is_null() {
        return false;
    }
    let root = GetAncestor(hwnd, GA_ROOT);
    let root = if root.is_null() { hwnd } else { root };
    let mut pid = 0u32;
    GetWindowThreadProcessId(root, &mut pid);
    if pid != OWN_PID.load(Ordering::Relaxed) {
        return false;
    }
    let ex = GetWindowLongPtrW(root, GWL_EXSTYLE) as u32;
    ex & WS_EX_TRANSPARENT == 0
}

unsafe fn foreground_is_own() -> bool {
    let hwnd = GetForegroundWindow();
    if hwnd.is_null() {
        return false;
    }
    let mut pid = 0u32;
    GetWindowThreadProcessId(hwnd, &mut pid);
    pid == OWN_PID.load(Ordering::Relaxed)
}

unsafe extern "system" fn keyboard_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let k = &*(lparam as *const KBDLLHOOKSTRUCT);
        let ours = k.dwExtraInfo == INJECT_MAGIC;
        let msg = wparam as u32;
        let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
        let vk_code = k.vkCode as u16;
        if !ours {
            if KEY_CAPTURE.load(Ordering::Relaxed) && down {
                if sentinel_core::keys::is_modifier_vk(vk_code) {
                    return CallNextHookEx(null_mut(), code, wparam, lparam);
                }
                KEY_CAPTURE.store(false, Ordering::Relaxed);
                let any_mod = key_down_now(vkc::CONTROL)
                    || key_down_now(vkc::MENU)
                    || key_down_now(vkc::SHIFT)
                    || key_down_now(vkc::LWIN)
                    || key_down_now(vkc::RWIN);
                let result = if vk_code == vkc::ESCAPE && !any_mod {
                    None
                } else {
                    Some(Hotkey {
                        ctrl: key_down_now(vkc::CONTROL),
                        alt: key_down_now(vkc::MENU),
                        shift: key_down_now(vkc::SHIFT),
                        win: key_down_now(vkc::LWIN) || key_down_now(vkc::RWIN),
                        vk: vk_code,
                    })
                };
                emit(PlatformEvent::KeyCaptured(result));
                return 1; // swallow
            }
            if POINT_PICK.load(Ordering::Relaxed) && down && vk_code == vkc::ESCAPE {
                POINT_PICK.store(false, Ordering::Relaxed);
                emit(PlatformEvent::PointPicked(None));
                return 1;
            }
            if WATCH_KEYS.load(Ordering::Relaxed)
                && down
                && !hotkey_vk_set(vk_code)
                && !sentinel_core::keys::is_modifier_vk(vk_code)
            {
                WATCH_KEYS.store(false, Ordering::Relaxed);
                emit(PlatformEvent::UserActivity("a key was pressed"));
            }
            if CAPTURING.load(Ordering::Relaxed) && CAP_KEYS.load(Ordering::Relaxed) && !foreground_is_own() {
                capture(EventKind::Key {
                    key: Key::new(vk_code, k.scanCode as u16, k.flags & LLKHF_EXTENDED != 0),
                    down,
                });
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

unsafe extern "system" fn mouse_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let m = &*(lparam as *const MSLLHOOKSTRUCT);
        let ours = m.dwExtraInfo == INJECT_MAGIC;
        let msg = wparam as u32;
        if !ours {
            let (x, y) = (m.pt.x, m.pt.y);
            let hi = (m.mouseData >> 16) as u16;
            let button = match msg {
                WM_LBUTTONDOWN | WM_LBUTTONUP => Some(MouseButton::Left),
                WM_RBUTTONDOWN | WM_RBUTTONUP => Some(MouseButton::Right),
                WM_MBUTTONDOWN | WM_MBUTTONUP => Some(MouseButton::Middle),
                WM_XBUTTONDOWN | WM_XBUTTONUP => Some(if hi == XBUTTON2 {
                    MouseButton::X2
                } else {
                    MouseButton::X1
                }),
                _ => None,
            };
            let is_down = matches!(msg, WM_LBUTTONDOWN | WM_RBUTTONDOWN | WM_MBUTTONDOWN | WM_XBUTTONDOWN);

            // Point picking for the auto-clicker: swallow the click.
            if let Some(b) = button {
                let swallow = SWALLOW_UP.load(Ordering::Relaxed);
                if !is_down && swallow != 0 && swallow == b.code() as u32 + 1 {
                    SWALLOW_UP.store(0, Ordering::Relaxed);
                    return 1;
                }
                if is_down && POINT_PICK.load(Ordering::Relaxed) {
                    POINT_PICK.store(false, Ordering::Relaxed);
                    SWALLOW_UP.store(b.code() as u32 + 1, Ordering::Relaxed);
                    emit(PlatformEvent::PointPicked(if b == MouseButton::Left {
                        Some((x, y))
                    } else {
                        None
                    }));
                    return 1;
                }
            }

            if WATCH_MOUSE.load(Ordering::Relaxed) && is_down {
                WATCH_MOUSE.store(false, Ordering::Relaxed);
                emit(PlatformEvent::UserActivity("a mouse button was clicked"));
            }

            if CAPTURING.load(Ordering::Relaxed) {
                match msg {
                    WM_MOUSEMOVE => {
                        if CAP_MOVES.load(Ordering::Relaxed) {
                            capture(EventKind::MouseMove { x, y });
                        }
                    }
                    WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                        if CAP_BUTTONS.load(Ordering::Relaxed) && !point_is_own_window(m.pt) {
                            capture(EventKind::Wheel {
                                delta: hi as i16 as i32,
                                horizontal: msg == WM_MOUSEHWHEEL,
                                x,
                                y,
                            });
                        }
                    }
                    _ => {
                        if let Some(b) = button {
                            if CAP_BUTTONS.load(Ordering::Relaxed) && !point_is_own_window(m.pt) {
                                capture(EventKind::MouseButton {
                                    button: b,
                                    down: is_down,
                                    x,
                                    y,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    CallNextHookEx(null_mut(), code, wparam, lparam)
}

enum Cmd {
    RegisterHotkeys(Hotkeys, Sender<Vec<HotkeyRegistration>>),
    SuspendHotkeys(bool),
    Refresh,
}

struct ServiceState {
    hwnd: HWND,
    cmd_rx: Receiver<Cmd>,
    hotkeys: Option<Hotkeys>,
    suspended: bool,
    raw_registered: bool,
}

thread_local! {
    static STATE: RefCell<Option<ServiceState>> = const { RefCell::new(None) };
}

fn need_hooks() -> bool {
    CAPTURING.load(Ordering::Relaxed)
        || WATCH_MOUSE.load(Ordering::Relaxed)
        || WATCH_KEYS.load(Ordering::Relaxed)
        || KEY_CAPTURE.load(Ordering::Relaxed)
        || POINT_PICK.load(Ordering::Relaxed)
        || SWALLOW_UP.load(Ordering::Relaxed) != 0
}

fn need_raw() -> bool {
    (CAPTURING.load(Ordering::Relaxed) && CAP_DELTAS.load(Ordering::Relaxed)) || WATCH_MOUSE.load(Ordering::Relaxed)
}

/// Install / remove hooks and raw input to match the current flags.
/// Hooks are (re)installed fresh for each session, so even a hook Windows
/// removed silently earlier is replaced.
unsafe fn sync_hooks(st: &mut ServiceState) {
    let hmod = GetModuleHandleW(null());
    if need_hooks() {
        if KB_HOOK.load(Ordering::SeqCst) == 0 {
            let h = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_proc), hmod, 0);
            if h.is_null() {
                crate::log_warn!("keyboard hook failed: error {}", GetLastError());
            }
            KB_HOOK.store(h as isize, Ordering::SeqCst);
        }
        if MS_HOOK.load(Ordering::SeqCst) == 0 {
            let h = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), hmod, 0);
            if h.is_null() {
                crate::log_warn!("mouse hook failed: error {}", GetLastError());
            }
            MS_HOOK.store(h as isize, Ordering::SeqCst);
        }
    } else {
        let k = KB_HOOK.swap(0, Ordering::SeqCst);
        if k != 0 {
            UnhookWindowsHookEx(k as HHOOK);
        }
        let m = MS_HOOK.swap(0, Ordering::SeqCst);
        if m != 0 {
            UnhookWindowsHookEx(m as HHOOK);
        }
    }
    let want_raw = need_raw();
    if want_raw != st.raw_registered {
        let dev = RAWINPUTDEVICE {
            usUsagePage: 0x01,
            usUsage: 0x02,
            dwFlags: if want_raw { RIDEV_INPUTSINK } else { RIDEV_REMOVE },
            hwndTarget: if want_raw { st.hwnd } else { null_mut() },
        };
        if RegisterRawInputDevices(&dev, 1, size_of::<RAWINPUTDEVICE>() as u32) != 0 {
            st.raw_registered = want_raw;
        }
    }
}

fn hotkey_mods(h: &Hotkey) -> HOT_KEY_MODIFIERS {
    let mut m = MOD_NOREPEAT;
    if h.ctrl {
        m |= MOD_CONTROL;
    }
    if h.alt {
        m |= MOD_ALT;
    }
    if h.shift {
        m |= MOD_SHIFT;
    }
    if h.win {
        m |= MOD_WIN;
    }
    m
}

unsafe fn apply_hotkeys(st: &mut ServiceState) -> Vec<HotkeyRegistration> {
    for a in HotkeyAction::ALL {
        UnregisterHotKey(st.hwnd, a.id());
    }
    for slot in HOTKEY_VKS.iter() {
        slot.store(0, Ordering::Relaxed);
    }
    let mut out = Vec::new();
    let Some(hk) = st.hotkeys.clone() else {
        return out;
    };
    let mut seen: Vec<Hotkey> = Vec::new();
    for a in HotkeyAction::ALL {
        let h = a.of(&hk);
        let c = (h.vk & 0xFF) as usize;
        HOTKEY_VKS[c / 64].fetch_or(1u64 << (c % 64), Ordering::Relaxed);
        let error = if st.suspended {
            None
        } else if let Err(e) = h.validate() {
            Some(e)
        } else if seen.contains(&h) {
            Some("Same as another SentinelTask hotkey".into())
        } else if RegisterHotKey(st.hwnd, a.id(), hotkey_mods(&h), h.vk as u32) == 0 {
            let err = GetLastError();
            Some(if err == ERROR_HOTKEY_ALREADY_REGISTERED {
                "Already used by another program — choose a different combination".into()
            } else {
                format!("Windows refused this hotkey (error {err})")
            })
        } else {
            None
        };
        seen.push(h);
        out.push(HotkeyRegistration {
            action: a,
            hotkey: h,
            error,
        });
    }
    out
}

unsafe fn handle_raw_input(lparam: LPARAM) {
    let mut size = 0u32;
    GetRawInputData(
        lparam as HRAWINPUT,
        RID_INPUT,
        null_mut(),
        &mut size,
        size_of::<RAWINPUTHEADER>() as u32,
    );
    if size == 0 || size as usize > 256 {
        return;
    }
    let mut buf = [0u64; 32]; // 256 bytes, 8-byte aligned
    let got = GetRawInputData(
        lparam as HRAWINPUT,
        RID_INPUT,
        buf.as_mut_ptr() as *mut c_void,
        &mut size,
        size_of::<RAWINPUTHEADER>() as u32,
    );
    if got == u32::MAX || got == 0 {
        return;
    }
    let raw = &*(buf.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEMOUSE {
        return;
    }
    // Injected input (ours or other software's) has no device handle.
    if raw.header.hDevice.is_null() {
        return;
    }
    let mouse = &raw.data.mouse;
    if mouse.usFlags & MOUSE_MOVE_ABSOLUTE != 0 {
        return; // tablets / remote desktop report absolute positions
    }
    let (dx, dy) = (mouse.lLastX, mouse.lLastY);
    if dx == 0 && dy == 0 {
        return;
    }
    if WATCH_MOUSE.load(Ordering::Relaxed) {
        let acc = ACTIVITY_ACCUM.fetch_add((dx.abs() + dy.abs()) as i64, Ordering::Relaxed);
        if acc + (dx.abs() + dy.abs()) as i64 >= ACTIVITY_THRESHOLD {
            WATCH_MOUSE.store(false, Ordering::Relaxed);
            emit(PlatformEvent::UserActivity("the mouse was moved"));
        }
    }
    if CAPTURING.load(Ordering::Relaxed) && CAP_DELTAS.load(Ordering::Relaxed) {
        capture(EventKind::MouseDelta { dx, dy });
    }
}

unsafe extern "system" fn service_wndproc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_HOTKEY => {
            if let Some(a) = HotkeyAction::from_id(wparam as i32) {
                emit(PlatformEvent::Hotkey(a));
            }
            0
        }
        WM_INPUT => {
            handle_raw_input(lparam);
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_APP_CMD => {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                if let Some(st) = s.as_mut() {
                    while let Ok(cmd) = st.cmd_rx.try_recv() {
                        match cmd {
                            Cmd::RegisterHotkeys(h, reply) => {
                                st.hotkeys = Some(h);
                                let r = apply_hotkeys(st);
                                let _ = reply.send(r);
                            }
                            Cmd::SuspendHotkeys(on) => {
                                st.suspended = on;
                                apply_hotkeys(st);
                            }
                            Cmd::Refresh => {}
                        }
                    }
                    sync_hooks(st);
                }
            });
            0
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Handle to the background service thread.
pub struct Service {
    hwnd: isize,
    cmd_tx: Sender<Cmd>,
}

impl Service {
    /// Start the service thread. Returns the service plus receivers for
    /// platform events and captured input.
    pub fn start(waker: Waker) -> (Service, Receiver<PlatformEvent>, Receiver<Captured>) {
        let (event_tx, event_rx) = unbounded();
        let (capture_tx, capture_rx) = unbounded();
        let (cmd_tx, cmd_rx) = unbounded::<Cmd>();
        OWN_PID.store(own_pid(), Ordering::Relaxed);
        let _ = SHARED.set(HookShared {
            capture_tx,
            event_tx,
            waker,
        });
        let (hwnd_tx, hwnd_rx) = crossbeam_channel::bounded::<isize>(1);
        std::thread::Builder::new()
            .name("sentinel-input".into())
            .spawn(move || unsafe {
                boost_thread_priority(false);
                let hinst = GetModuleHandleW(null());
                let class = wide("SentinelTaskService");
                let wc = WNDCLASSEXW {
                    cbSize: size_of::<WNDCLASSEXW>() as u32,
                    lpfnWndProc: Some(service_wndproc),
                    hInstance: hinst,
                    lpszClassName: class.as_ptr(),
                    ..zeroed()
                };
                RegisterClassExW(&wc);
                let hwnd = CreateWindowExW(
                    0,
                    class.as_ptr(),
                    null(),
                    0,
                    0,
                    0,
                    0,
                    0,
                    HWND_MESSAGE,
                    null_mut(),
                    hinst,
                    null(),
                );
                STATE.with(|s| {
                    *s.borrow_mut() = Some(ServiceState {
                        hwnd,
                        cmd_rx,
                        hotkeys: None,
                        suspended: false,
                        raw_registered: false,
                    })
                });
                let _ = hwnd_tx.send(hwnd as isize);
                let mut msg: MSG = zeroed();
                while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
            })
            .expect("spawn input thread");
        let hwnd = hwnd_rx.recv_timeout(Duration::from_secs(5)).unwrap_or(0);
        if hwnd == 0 {
            crate::log_warn!("input service window could not be created");
        }
        (Service { hwnd, cmd_tx }, event_rx, capture_rx)
    }

    fn poke(&self) {
        unsafe {
            PostMessageW(self.hwnd as HWND, WM_APP_CMD, 0, 0);
        }
    }

    /// Register all hotkeys, returning per-hotkey success or the reason it
    /// failed (e.g. already taken by another program).
    pub fn register_hotkeys(&self, hk: &Hotkeys) -> Vec<HotkeyRegistration> {
        let (tx, rx) = crossbeam_channel::bounded(1);
        let _ = self.cmd_tx.send(Cmd::RegisterHotkeys(hk.clone(), tx));
        self.poke();
        rx.recv_timeout(Duration::from_secs(3)).unwrap_or_default()
    }

    /// Temporarily release our hotkeys (while the user is choosing a new one).
    pub fn suspend_hotkeys(&self, on: bool) {
        let _ = self.cmd_tx.send(Cmd::SuspendHotkeys(on));
        self.poke();
    }

    pub fn set_capture(&self, cfg: Option<CaptureConfig>) {
        match cfg {
            Some(c) => {
                CAP_MOVES.store(c.mouse_moves, Ordering::SeqCst);
                CAP_DELTAS.store(c.raw_deltas, Ordering::SeqCst);
                CAP_BUTTONS.store(c.buttons, Ordering::SeqCst);
                CAP_KEYS.store(c.keyboard, Ordering::SeqCst);
                CAPTURING.store(true, Ordering::SeqCst);
            }
            None => CAPTURING.store(false, Ordering::SeqCst),
        }
        let _ = self.cmd_tx.send(Cmd::Refresh);
        self.poke();
    }

    /// Watch for physical user input during playback.
    pub fn set_activity_watch(&self, mouse: bool, keys: bool) {
        ACTIVITY_ACCUM.store(0, Ordering::SeqCst);
        WATCH_MOUSE.store(mouse, Ordering::SeqCst);
        WATCH_KEYS.store(keys, Ordering::SeqCst);
        let _ = self.cmd_tx.send(Cmd::Refresh);
        self.poke();
    }

    /// Report the next key chord pressed anywhere (for choosing a hotkey).
    pub fn begin_key_capture(&self, on: bool) {
        KEY_CAPTURE.store(on, Ordering::SeqCst);
        let _ = self.cmd_tx.send(Cmd::Refresh);
        self.poke();
    }

    /// Report the next left click anywhere (for picking a screen point).
    pub fn begin_point_pick(&self, on: bool) {
        POINT_PICK.store(on, Ordering::SeqCst);
        let _ = self.cmd_tx.send(Cmd::Refresh);
        self.poke();
    }

    /// Whether this platform can actually capture/inject.
    pub fn is_functional(&self) -> bool {
        self.hwnd != 0
    }
}

// ---------------------------------------------------------------------------
// Console Ctrl+C (CLI playback): stop cleanly instead of being killed with
// keys held down.
// ---------------------------------------------------------------------------

static CTRL_C: AtomicBool = AtomicBool::new(false);

unsafe extern "system" fn console_handler(_ctrl: u32) -> windows_sys::core::BOOL {
    CTRL_C.store(true, Ordering::SeqCst);
    1
}

pub fn install_console_handler() {
    unsafe {
        windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(console_handler), 1);
    }
}

pub fn console_interrupted() -> bool {
    CTRL_C.load(Ordering::SeqCst)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flags(i: &INPUT) -> (u16, u16, u32) {
        unsafe { (i.Anonymous.ki.wVk, i.Anonymous.ki.wScan, i.Anonymous.ki.dwFlags) }
    }

    #[test]
    fn arrow_keys_use_extended_scan_codes() {
        let up = Key::new(vkc::UP, 0x48, true);
        let (vk, scan, f) = flags(&key_to_input(up, true));
        assert_eq!((vk, scan), (vkc::UP, 0x48));
        assert_eq!(f & KEYEVENTF_SCANCODE, KEYEVENTF_SCANCODE);
        assert_eq!(f & KEYEVENTF_EXTENDEDKEY, KEYEVENTF_EXTENDEDKEY);
        assert_eq!(f & KEYEVENTF_KEYUP, 0);
        let (_, _, f) = flags(&key_to_input(up, false));
        // Key-up must keep the scan-code flag or DirectInput games see a stuck key.
        assert_eq!(
            f & (KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP),
            KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP
        );
    }

    #[test]
    fn plain_letters_are_not_extended() {
        let (_, scan, f) = flags(&key_to_input(Key::new(0x41, 0x1E, false), true));
        assert_eq!(scan, 0x1E);
        assert_eq!(f & KEYEVENTF_EXTENDEDKEY, 0);
    }

    #[test]
    fn special_keys_go_by_virtual_key() {
        for code in [vkc::PAUSE, vkc::SNAPSHOT, 0xAF /* volume up */] {
            let (vk, _, f) = flags(&key_to_input(Key::from_vk(code), true));
            assert_eq!(vk, code);
            assert_eq!(f & KEYEVENTF_SCANCODE, 0, "vk {code:#x}");
        }
    }

    #[test]
    fn missing_scan_codes_are_looked_up() {
        let (_, scan, f) = flags(&key_to_input(Key::from_vk(0x41), true));
        assert_ne!(scan, 0);
        assert_eq!(f & KEYEVENTF_SCANCODE, KEYEVENTF_SCANCODE);
    }

    #[test]
    fn sleeper_is_precise() {
        let mut s = Sleeper::new();
        let mut total_err = 0u64;
        let mut worst = 0u64;
        for _ in 0..40 {
            let target = now_us() + 3_000;
            s.wait_until(target);
            let err = now_us() - target;
            total_err += err;
            worst = worst.max(err);
        }
        let mean = total_err / 40;
        // Generous bounds so a busy CI machine doesn't flake; a plain
        // Sleep() would be ~10-15 ms late here.
        assert!(mean < 2_000, "mean lateness {mean} us ({})", s.kind());
        assert!(worst < 20_000, "worst lateness {worst} us");
    }

    #[test]
    fn clock_is_monotonic() {
        let mut prev = now_us();
        for _ in 0..10_000 {
            let n = now_us();
            assert!(n >= prev);
            prev = n;
        }
    }
}
