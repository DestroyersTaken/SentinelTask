//! Application state and the top-level frame.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::Receiver;
use eframe::egui::{self, pos2, vec2, Align2, Color32, CornerRadius, Id, Margin, RichText, Sense, Stroke, StrokeKind};

use sentinel_core::keys::Hotkey;
use sentinel_core::settings::{OverlayCorner, Settings, Theme};
use sentinel_core::util::{format_clock_us, format_duration_us, now_unix};
use sentinel_core::{Macro, ScreenInfo};

use crate::engine::clicker::{self as clicker_engine, ClickerHandle};
use crate::engine::player::{self, FinishReason, PauseReason, PlayRequest, PlaybackHandle, RunState};
use crate::engine::recorder::RecordingSession;
use crate::library::{self, Library};
use crate::paths::{write_atomic, Paths};
use crate::platform::{self, Captured, HotkeyAction, HotkeyRegistration, PlatformEvent, Service, WindowInfo};
use crate::{log_info, log_warn};

use super::editor::EditorState;
use super::theme::{self, Palette};
use super::widgets::{self, paint_icon, Icon};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    Recorder,
    Library,
    Editor,
    Clicker,
    Settings,
    About,
}

impl Page {
    const NAV: [(Page, Icon, &'static str); 6] = [
        (Page::Recorder, Icon::Record, "Recorder"),
        (Page::Library, Icon::Library, "Library"),
        (Page::Editor, Icon::Edit, "Editor"),
        (Page::Clicker, Icon::Clicker, "Auto Clicker"),
        (Page::Settings, Icon::Settings, "Settings"),
        (Page::About, Icon::Info, "About"),
    ];
}

/// The macro currently loaded.
pub struct Document {
    pub mac: Macro,
    pub path: Option<PathBuf>,
    pub dirty: bool,
}

pub enum Mode {
    Idle,
    RecordCountdown { start_at: Instant, by_hotkey: bool },
    Recording { session: RecordingSession },
    Playing { handle: PlaybackHandle },
    Clicking { handle: ClickerHandle },
}

impl Mode {
    pub fn is_idle(&self) -> bool {
        matches!(self, Mode::Idle)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Ok,
    Warn,
    Error,
}

struct Toast {
    text: String,
    kind: ToastKind,
    born: Instant,
}

pub enum Modal {
    ConfirmDelete(PathBuf, String),
    Rename { path: PathBuf, name: String },
    UnsavedOnClose,
    UnsavedBeforeOpen(OpenTarget),
    Message { title: String, text: String },
}

type ModalAction = Box<dyn FnOnce(&mut SentinelApp, &egui::Context)>;

pub enum OpenTarget {
    File(PathBuf),
    NewRecording,
}

/// Launch-time options from the command line / embedded payload.
#[derive(Default)]
pub struct LaunchOptions {
    pub open_file: Option<PathBuf>,
    pub embedded: Option<Macro>,
}

pub struct SentinelApp {
    pub settings: Settings,
    pub paths: Paths,
    pub service: Service,
    events_rx: Receiver<PlatformEvent>,
    capture_rx: Receiver<Captured>,
    pub hotkey_status: Vec<HotkeyRegistration>,
    pub page: Page,
    pub doc: Option<Document>,
    pub mode: Mode,
    pub library: Library,
    pub library_search: String,
    pub editor: EditorState,
    toasts: Vec<Toast>,
    pub modal: Option<Modal>,
    /// Which hotkey is being re-bound in Settings.
    pub capturing_hotkey: Option<HotkeyAction>,
    pub picking_point: bool,
    /// Last foreground window that isn't SentinelTask (for warnings and as
    /// the default focus-guard target).
    pub last_foreign_window: Option<WindowInfo>,
    last_fg_poll: Instant,
    pub elevated: bool,
    pub player_mode: bool,
    pub palette: Palette,
    applied_theme: Option<(Theme, u32)>,
    settings_saved_at: Instant,
    settings_dirty: bool,
    pub screen: ScreenInfo,
    last_screen_poll: Instant,
    allow_close: bool,
    pub sleeper_kind: &'static str,
    /// Result of the last finished run, shown on the Recorder page.
    pub last_run: Option<String>,
    window_restore_needed: bool,
    compact_applied: Option<bool>,
}

fn waker_for(ctx: &egui::Context) -> platform::Waker {
    let ctx = ctx.clone();
    Arc::new(move || ctx.request_repaint())
}

impl SentinelApp {
    pub fn new(cc: &eframe::CreationContext<'_>, launch: LaunchOptions) -> Self {
        let paths = Paths::detect();
        let settings = std::fs::read_to_string(paths.settings_file())
            .map(|s| Settings::from_json(&s))
            .unwrap_or_default();
        platform::set_virtual_key_injection(settings.safety.virtual_key_injection);
        let (service, events_rx, capture_rx) = Service::start(waker_for(&cc.egui_ctx));
        let hotkey_status = service.register_hotkeys(&settings.hotkeys);
        for r in &hotkey_status {
            if let Some(e) = &r.error {
                log_warn!("hotkey {} ({}) not registered: {e}", r.action.label(), r.hotkey);
            }
        }
        let library_dir = paths.library_dir(&settings.library_dir);
        let _ = std::fs::create_dir_all(&library_dir);
        let mut library = Library::new(library_dir);
        library.refresh();
        let palette = theme::palette(settings.ui.theme);
        let player_mode = launch.embedded.is_some();
        let mut app = Self {
            settings,
            paths,
            service,
            events_rx,
            capture_rx,
            hotkey_status,
            page: Page::Recorder,
            doc: None,
            mode: Mode::Idle,
            library,
            library_search: String::new(),
            editor: EditorState::default(),
            toasts: Vec::new(),
            modal: None,
            capturing_hotkey: None,
            picking_point: false,
            last_foreign_window: None,
            last_fg_poll: Instant::now() - Duration::from_secs(5),
            elevated: platform::process_is_elevated(),
            player_mode,
            palette,
            applied_theme: None,
            settings_saved_at: Instant::now(),
            settings_dirty: false,
            screen: platform::screen_info(),
            last_screen_poll: Instant::now(),
            allow_close: false,
            sleeper_kind: platform::Sleeper::new().kind(),
            last_run: None,
            window_restore_needed: false,
            compact_applied: None,
        };
        // Start on a specific page (used for documentation screenshots).
        if let Ok(page) = std::env::var("SENTINEL_PAGE") {
            app.page = match page.to_ascii_lowercase().as_str() {
                "library" => Page::Library,
                "editor" => Page::Editor,
                "clicker" => Page::Clicker,
                "settings" => Page::Settings,
                "about" => Page::About,
                _ => Page::Recorder,
            };
        }
        if let Some(m) = launch.embedded {
            app.doc = Some(Document {
                mac: m,
                path: None,
                dirty: false,
            });
        } else if let Some(p) = launch.open_file {
            app.open_path(&p);
        }
        app
    }

    // -----------------------------------------------------------------
    // Notifications
    // -----------------------------------------------------------------

    pub fn toast(&mut self, kind: ToastKind, text: impl Into<String>) {
        let text = text.into();
        if kind == ToastKind::Error {
            log_warn!("{text}");
        }
        self.toasts.push(Toast {
            text,
            kind,
            born: Instant::now(),
        });
        if self.toasts.len() > 4 {
            self.toasts.remove(0);
        }
    }

    pub fn mark_settings_dirty(&mut self) {
        self.settings_dirty = true;
    }

    fn save_settings_now(&mut self) {
        if self.player_mode {
            return;
        }
        let json = self.settings.to_json();
        if let Err(e) = write_atomic(&self.paths.settings_file(), json.as_bytes()) {
            log_warn!("saving settings failed: {e}");
        }
        self.settings_dirty = false;
        self.settings_saved_at = Instant::now();
    }

    pub fn reregister_hotkeys(&mut self) {
        self.hotkey_status = self.service.register_hotkeys(&self.settings.hotkeys);
    }

    pub fn hotkey_text(&self, a: HotkeyAction) -> String {
        a.of(&self.settings.hotkeys).to_string()
    }

    pub fn hotkey_error(&self, a: HotkeyAction) -> Option<&str> {
        self.hotkey_status
            .iter()
            .find(|r| r.action == a)
            .and_then(|r| r.error.as_deref())
    }

    // -----------------------------------------------------------------
    // Documents
    // -----------------------------------------------------------------

    pub fn has_unsaved_changes(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty)
    }

    pub fn open_path(&mut self, path: &Path) {
        match library::load_macro(path) {
            Ok(m) => {
                let is_rec = path
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_some_and(|x| x.eq_ignore_ascii_case("rec"));
                let n = m.events.len();
                self.doc = Some(Document {
                    mac: m,
                    // Imported .rec files get saved as .stmacro, never over the .rec.
                    path: if is_rec { None } else { Some(path.to_path_buf()) },
                    dirty: is_rec,
                });
                self.editor = EditorState::default();
                if !is_rec {
                    self.settings.push_recent(&path.to_string_lossy());
                    self.mark_settings_dirty();
                }
                self.toast(
                    ToastKind::Ok,
                    if is_rec {
                        format!("Imported TinyTask recording ({n} events). Save it to keep it.")
                    } else {
                        format!("Opened “{}”", library::file_stem(path))
                    },
                );
                log_info!("opened {}", path.display());
            }
            Err(e) => {
                self.modal = Some(Modal::Message {
                    title: "Couldn't open file".into(),
                    text: e,
                });
            }
        }
    }

    pub fn request_open(&mut self, path: PathBuf) {
        if self.has_unsaved_changes() {
            self.modal = Some(Modal::UnsavedBeforeOpen(OpenTarget::File(path)));
        } else {
            self.open_path(&path);
        }
    }

    pub fn open_dialog(&mut self) {
        let dir = self.library.dir.clone();
        if let Some(p) = platform::open_file_dialog(
            "Open macro",
            &[
                ("Macros", &["stmacro", "rec"]),
                ("SentinelTask macro", &["stmacro"]),
                ("TinyTask recording", &["rec"]),
            ],
            Some(&dir),
        ) {
            self.request_open(p);
        }
    }

    /// Save to the current path, or into the library if it has none.
    pub fn save(&mut self) -> bool {
        let Some(doc) = self.doc.as_mut() else { return false };
        let path = match &doc.path {
            Some(p) => p.clone(),
            None => {
                let name = if doc.mac.meta.name.is_empty() {
                    "Macro".to_string()
                } else {
                    doc.mac.meta.name.clone()
                };
                self.library.unique_path(&name)
            }
        };
        self.save_to(path)
    }

    pub fn save_as(&mut self) -> bool {
        let Some(doc) = self.doc.as_ref() else { return false };
        let name = sentinel_core::util::sanitize_file_name(&doc.mac.meta.name);
        let dir = self.library.dir.clone();
        match platform::save_file_dialog(
            "Save macro as",
            &[("SentinelTask macro", &["stmacro"])],
            Some(&dir),
            &format!("{name}.stmacro"),
        ) {
            Some(mut p) => {
                if p.extension().is_none() {
                    p.set_extension("stmacro");
                }
                self.save_to(p)
            }
            None => false,
        }
    }

    fn save_to(&mut self, path: PathBuf) -> bool {
        let Some(doc) = self.doc.as_mut() else { return false };
        doc.mac.meta.modified = now_unix();
        if doc.mac.meta.created == 0 {
            doc.mac.meta.created = doc.mac.meta.modified;
        }
        doc.mac.meta.app_version = sentinel_core::APP_VERSION.into();
        match library::save_macro(&path, &doc.mac) {
            Ok(()) => {
                doc.path = Some(path.clone());
                doc.dirty = false;
                self.settings.push_recent(&path.to_string_lossy());
                self.mark_settings_dirty();
                self.library.refresh();
                self.toast(ToastKind::Ok, format!("Saved “{}”", library::file_stem(&path)));
                true
            }
            Err(e) => {
                self.toast(ToastKind::Error, e);
                false
            }
        }
    }

    pub fn export_player(&mut self) {
        let Some(doc) = self.doc.as_ref() else { return };
        let name = sentinel_core::util::sanitize_file_name(&doc.mac.meta.name);
        let Some(mut out) = platform::save_file_dialog(
            "Export standalone player",
            &[("Program", &["exe"])],
            None,
            &format!("{name}.exe"),
        ) else {
            return;
        };
        if out.extension().is_none() {
            out.set_extension("exe");
        }
        let mac = doc.mac.clone();
        let result = std::env::current_exe().and_then(|exe| crate::payload::export_player(&exe, &out, &mac));
        match result {
            Ok(()) => {
                self.modal = Some(Modal::Message {
                    title: "Standalone player created".into(),
                    text: format!(
                        "{}\n\nDouble-click it on any Windows PC to play this macro — no install needed. \
                     Some antivirus programs distrust unsigned programs that send keystrokes; \
                     if yours complains, that is why (see the README).",
                        out.display()
                    ),
                })
            }
            Err(e) => self.toast(ToastKind::Error, format!("Export failed: {e}")),
        }
    }

    // -----------------------------------------------------------------
    // Recording
    // -----------------------------------------------------------------

    pub fn start_recording(&mut self, ctx: &egui::Context, by_hotkey: bool) {
        if !self.mode.is_idle() || self.player_mode {
            return;
        }
        if self.has_unsaved_changes() && !by_hotkey {
            self.modal = Some(Modal::UnsavedBeforeOpen(OpenTarget::NewRecording));
            return;
        }
        if self.has_unsaved_changes() {
            // Hotkeys can't wait for a dialog: keep the old macro safe first.
            self.save();
        }
        let secs = self.settings.recording.countdown_secs;
        self.minimize_if_wanted(ctx);
        if secs > 0 {
            self.mode = Mode::RecordCountdown {
                start_at: Instant::now() + Duration::from_secs(secs as u64),
                by_hotkey,
            };
        } else {
            self.begin_capture(by_hotkey);
        }
    }

    fn begin_capture(&mut self, _by_hotkey: bool) {
        // Drop anything captured before now.
        while self.capture_rx.try_recv().is_ok() {}
        self.screen = platform::screen_info();
        let session = RecordingSession::new(platform::now_us(), &self.settings.recording);
        self.service
            .set_capture(Some(RecordingSession::capture_config(&self.settings.recording)));
        self.mode = Mode::Recording { session };
        log_info!("recording started");
    }

    fn drain_capture(&mut self) {
        let want_target = self.settings.recording.capture_target_window;
        if let Mode::Recording { session, .. } = &mut self.mode {
            let mut meaningful = false;
            while let Ok(c) = self.capture_rx.try_recv() {
                // Clicks and scrolls on SentinelTask itself (e.g. its Stop
                // button) are never part of a macro.
                let on_us = match c.kind {
                    sentinel_core::EventKind::MouseButton { x, y, .. }
                    | sentinel_core::EventKind::Wheel { x, y, .. } => platform::point_is_own_window(x, y),
                    _ => false,
                };
                if !on_us {
                    meaningful |= session.push(c);
                }
            }
            if meaningful && session.target.is_none() && want_target {
                if let Some(w) = platform::foreground_window() {
                    if w.pid != platform::own_pid() && !w.process.is_empty() {
                        session.target = Some(sentinel_core::TargetWindow {
                            process: w.process,
                            title: w.title,
                        });
                    }
                }
            }
        }
    }

    pub fn stop_recording(&mut self, ctx: &egui::Context, stop_hotkey: Option<Hotkey>) {
        if let Mode::RecordCountdown { .. } = self.mode {
            self.mode = Mode::Idle;
            self.restore_window(ctx);
            return;
        }
        if !matches!(self.mode, Mode::Recording { .. }) {
            return;
        }
        self.service.set_capture(None);
        // Give the hook thread a moment to flush the last events.
        std::thread::sleep(Duration::from_millis(15));
        self.drain_capture();
        let Mode::Recording { session, .. } = std::mem::replace(&mut self.mode, Mode::Idle) else {
            return;
        };
        self.restore_window(ctx);
        let (events, target) = session.finish(stop_hotkey.as_ref());
        log_info!("recording stopped: {} events", events.len());
        if events.is_empty() {
            self.toast(ToastKind::Warn, "Nothing was recorded.");
            return;
        }
        let ts = platform::local_timestamp();
        let mut mac = Macro::new(format!("Recording {ts}"));
        mac.meta.created = now_unix();
        mac.meta.modified = mac.meta.created;
        mac.meta.app_version = sentinel_core::APP_VERSION.into();
        mac.meta.screen = Some(self.screen);
        mac.meta.target = target;
        mac.meta.source = "Recorded".into();
        mac.playback = self.settings.playback_defaults;
        mac.events = events;
        let suggested = mac.suggested_mouse_mode();
        if suggested != mac.playback.mouse_mode {
            mac.playback.mouse_mode = suggested;
            self.toast(
                ToastKind::Info,
                "The cursor was locked during recording (a 3D game?) — switched to Relative mouse mode.",
            );
        }
        // Auto-save into the library so a recording is never lost.
        let path = self.library.unique_path(&mac.meta.name);
        self.doc = Some(Document {
            mac,
            path: None,
            dirty: true,
        });
        self.editor = EditorState::default();
        self.save_to(path);
    }

    // -----------------------------------------------------------------
    // Playback
    // -----------------------------------------------------------------

    pub fn start_playback(&mut self, ctx: &egui::Context, by_hotkey: bool) {
        if !self.mode.is_idle() {
            return;
        }
        let Some(doc) = self.doc.as_ref() else {
            self.toast(ToastKind::Warn, "Record or open a macro first.");
            return;
        };
        if doc.mac.events.is_empty() {
            self.toast(ToastKind::Warn, "This macro has no events.");
            return;
        }
        let mut events = doc.mac.events.clone();
        sentinel_core::normalize::balance(&mut events);
        let req = PlayRequest {
            events: Arc::new(events),
            opts: doc.mac.playback,
            recorded_screen: doc.mac.meta.screen,
            focus_process: doc.mac.meta.target.as_ref().map(|t| t.process.clone()),
            countdown_secs: self.settings.safety.play_countdown_secs,
            keep_awake: self.settings.safety.prevent_sleep,
            wait_for_modifiers: by_hotkey,
        };
        self.service.set_activity_watch(
            self.settings.safety.stop_on_user_mouse,
            self.settings.safety.stop_on_user_key,
        );
        self.minimize_if_wanted(ctx);
        let handle = player::start(req, waker_for(ctx));
        self.mode = Mode::Playing { handle };
        self.last_run = None;
        log_info!("playback started");
    }

    pub fn stop_playback(&mut self) {
        if let Mode::Playing { handle } = &self.mode {
            handle.stop();
        }
    }

    fn finish_playback_if_done(&mut self, ctx: &egui::Context) {
        let done = matches!(&self.mode, Mode::Playing { handle } if handle.is_finished());
        if !done {
            return;
        }
        let Mode::Playing { handle } = std::mem::replace(&mut self.mode, Mode::Idle) else {
            return;
        };
        self.service.set_activity_watch(false, false);
        let s = handle.join();
        self.restore_window(ctx);
        let summary = format!(
            "{} pass{} in {}",
            s.pass,
            if s.pass == 1 { "" } else { "es" },
            format_duration_us(s.elapsed_us)
        );
        let msg = match &s.state {
            RunState::Finished(FinishReason::Completed) => format!("Finished — {summary}."),
            RunState::Finished(FinishReason::Stopped) => format!("Stopped — {summary}."),
            RunState::Finished(FinishReason::UserActivity(what)) => {
                format!("Stopped because {what} — {summary}. (Safety setting)")
            }
            _ => summary,
        };
        log_info!("playback ended: {msg}");
        if s.blocked_inputs > 0 {
            self.toast(
                ToastKind::Warn,
                "Windows blocked some input. If the target runs as administrator, run SentinelTask as administrator too.",
            );
        }
        self.last_run = Some(msg.clone());
        self.toast(ToastKind::Info, msg);
    }

    // -----------------------------------------------------------------
    // Auto-clicker
    // -----------------------------------------------------------------

    pub fn toggle_clicker(&mut self, ctx: &egui::Context) {
        match &self.mode {
            Mode::Clicking { handle } => handle.stop(),
            Mode::Idle => {
                let cfg = self.settings.clicker;
                let handle = clicker_engine::start(cfg, waker_for(ctx));
                self.mode = Mode::Clicking { handle };
            }
            _ => self.toast(
                ToastKind::Warn,
                "Stop recording/playback before starting the auto-clicker.",
            ),
        }
    }

    fn finish_clicker_if_done(&mut self) {
        let done = matches!(&self.mode, Mode::Clicking { handle } if handle.is_finished());
        if done {
            if let Mode::Clicking { handle } = std::mem::replace(&mut self.mode, Mode::Idle) {
                let n = handle.join();
                self.toast(ToastKind::Info, format!("Auto-clicker stopped after {n} clicks."));
            }
        }
    }

    pub fn emergency_stop(&mut self, ctx: &egui::Context) {
        match &self.mode {
            Mode::Playing { handle } => handle.stop(),
            Mode::Clicking { handle } => handle.stop(),
            Mode::Recording { .. } | Mode::RecordCountdown { .. } => {
                let hk = self.settings.hotkeys.emergency_stop;
                self.stop_recording(ctx, Some(hk));
            }
            Mode::Idle => {}
        }
        platform::emergency_release_all();
    }

    // -----------------------------------------------------------------
    // Window helpers
    // -----------------------------------------------------------------

    fn minimize_if_wanted(&mut self, ctx: &egui::Context) {
        if self.settings.ui.minimize_while_running && !self.settings.ui.compact {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
            self.window_restore_needed = true;
        }
    }

    fn restore_window(&mut self, ctx: &egui::Context) {
        if self.window_restore_needed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            self.window_restore_needed = false;
        }
    }

    // -----------------------------------------------------------------
    // Event pump
    // -----------------------------------------------------------------

    fn handle_platform_events(&mut self, ctx: &egui::Context) {
        while let Ok(ev) = self.events_rx.try_recv() {
            match ev {
                PlatformEvent::Hotkey(a) => self.on_hotkey(ctx, a),
                PlatformEvent::UserActivity(what) => {
                    if let Mode::Playing { handle } = &self.mode {
                        handle.abort_for_activity(what);
                    }
                }
                PlatformEvent::KeyCaptured(result) => self.finish_key_capture(result),
                PlatformEvent::PointPicked(p) => {
                    self.picking_point = false;
                    if let Some((x, y)) = p {
                        self.settings.clicker.location = sentinel_core::clicker::ClickLocation::Fixed { x, y };
                        self.mark_settings_dirty();
                        self.toast(ToastKind::Ok, format!("Clicks will go to ({x}, {y})."));
                    }
                }
            }
        }
    }

    /// Start choosing a new key for `action` (Settings).
    pub fn begin_key_capture(&mut self, action: HotkeyAction) {
        self.capturing_hotkey = Some(action);
        self.service.suspend_hotkeys(true);
        self.service.begin_key_capture(true);
    }

    /// End hotkey choosing. `None` cancels. Safe to call more than once —
    /// the system hook and the in-window fallback may both report.
    pub fn finish_key_capture(&mut self, result: Option<Hotkey>) {
        let Some(action) = self.capturing_hotkey.take() else {
            return;
        };
        self.service.begin_key_capture(false);
        self.service.suspend_hotkeys(false);
        match result {
            Some(hk) => self.set_hotkey(action, hk),
            None => self.reregister_hotkeys(),
        }
    }

    pub fn cancel_point_pick(&mut self) {
        if self.picking_point {
            self.picking_point = false;
            self.service.begin_point_pick(false);
        }
    }

    pub fn set_hotkey(&mut self, action: HotkeyAction, hk: Hotkey) {
        if let Err(e) = hk.validate() {
            self.toast(ToastKind::Warn, e);
            self.reregister_hotkeys();
            return;
        }
        let h = &mut self.settings.hotkeys;
        match action {
            HotkeyAction::Record => h.record = hk,
            HotkeyAction::Play => h.play = hk,
            HotkeyAction::Pause => h.pause = hk,
            HotkeyAction::EmergencyStop => h.emergency_stop = hk,
            HotkeyAction::Clicker => h.clicker = hk,
        }
        self.mark_settings_dirty();
        self.reregister_hotkeys();
        match self.hotkey_error(action).map(str::to_string) {
            Some(e) => self.toast(ToastKind::Warn, format!("{hk}: {e}")),
            None => self.toast(ToastKind::Ok, format!("{} is now {hk}", action.label())),
        }
    }

    fn on_hotkey(&mut self, ctx: &egui::Context, a: HotkeyAction) {
        if self.capturing_hotkey.is_some() {
            return;
        }
        match a {
            HotkeyAction::Record => match self.mode {
                Mode::Idle => self.start_recording(ctx, true),
                Mode::Recording { .. } | Mode::RecordCountdown { .. } => {
                    let hk = self.settings.hotkeys.record;
                    self.stop_recording(ctx, Some(hk));
                }
                _ => {}
            },
            HotkeyAction::Play => match &self.mode {
                Mode::Idle => self.start_playback(ctx, true),
                Mode::Playing { handle } => handle.stop(),
                Mode::Recording { .. } | Mode::RecordCountdown { .. } => {
                    let hk = self.settings.hotkeys.play;
                    self.stop_recording(ctx, Some(hk));
                }
                Mode::Clicking { .. } => {}
            },
            HotkeyAction::Pause => {
                if let Mode::Playing { handle } = &self.mode {
                    handle.toggle_pause();
                }
            }
            HotkeyAction::EmergencyStop => self.emergency_stop(ctx),
            HotkeyAction::Clicker => {
                if matches!(self.mode, Mode::Idle | Mode::Clicking { .. }) {
                    self.toggle_clicker(ctx);
                }
            }
        }
    }

    fn poll_environment(&mut self) {
        if self.last_fg_poll.elapsed() >= Duration::from_millis(600) {
            self.last_fg_poll = Instant::now();
            if let Some(w) = platform::foreground_window() {
                if w.pid != platform::own_pid() && !w.process.is_empty() {
                    self.last_foreign_window = Some(w);
                }
            }
        }
        if self.last_screen_poll.elapsed() >= Duration::from_secs(2) && self.mode.is_idle() {
            self.last_screen_poll = Instant::now();
            self.screen = platform::screen_info();
        }
    }

    /// Process the elevation problem (RESEARCH §1 #4): is the window we will
    /// likely play into running as admin while we are not?
    pub fn elevation_problem(&self) -> Option<String> {
        if self.elevated || !self.settings.safety.warn_elevation {
            return None;
        }
        let w = self.last_foreign_window.as_ref()?;
        if w.elevated == Some(true) {
            Some(if w.process.is_empty() {
                "a program running as administrator".into()
            } else {
                w.process.clone()
            })
        } else {
            None
        }
    }

    pub fn restart_elevated(&mut self) {
        let mut args = Vec::new();
        if let Some(p) = self.doc.as_ref().and_then(|d| d.path.as_ref()) {
            args.push(p.to_string_lossy().into_owned());
        }
        if self.has_unsaved_changes() {
            self.save();
        }
        self.save_settings_now();
        match platform::restart_as_admin(&args) {
            Ok(()) => std::process::exit(0),
            Err(e) => self.toast(ToastKind::Warn, e),
        }
    }

    // -----------------------------------------------------------------
    // Frame
    // -----------------------------------------------------------------

    fn apply_theme_if_needed(&mut self, ctx: &egui::Context) {
        let key = (self.settings.ui.theme, (self.settings.ui.zoom * 100.0) as u32);
        if self.applied_theme != Some(key) {
            self.palette = theme::palette(self.settings.ui.theme);
            theme::apply(ctx, &self.palette, self.settings.ui.zoom);
            self.applied_theme = Some(key);
        }
        let compact = self.settings.ui.compact;
        if self.compact_applied != Some(compact) {
            if compact {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(560.0, 92.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(egui::WindowLevel::AlwaysOnTop));
            } else if self.compact_applied.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(vec2(1040.0, 720.0)));
                ctx.send_viewport_cmd(egui::ViewportCommand::WindowLevel(if self.settings.ui.always_on_top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
            }
            self.compact_applied = Some(compact);
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.modal.is_some() || !self.mode.is_idle() {
            return;
        }
        let (save, open, undo, redo) = ctx.input_mut(|i| {
            (
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S)),
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::O)),
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Z)),
                i.consume_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::Y)),
            )
        });
        if save && self.doc.is_some() {
            self.save();
        }
        if open {
            self.open_dialog();
        }
        if undo && self.page == Page::Editor {
            self.editor_undo();
        }
        if redo && self.page == Page::Editor {
            self.editor_redo();
        }
    }

    fn handle_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .map(|f| f.path().to_path_buf())
                .filter(|p| !p.as_os_str().is_empty())
                .collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            if self.mode.is_idle() {
                self.request_open(p);
            }
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && !self.allow_close {
            if self.has_unsaved_changes() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.modal = Some(Modal::UnsavedOnClose);
            } else {
                self.shutdown();
            }
        }
    }

    fn shutdown(&mut self) {
        self.stop_playback();
        if let Mode::Clicking { handle } = &self.mode {
            handle.stop();
        }
        self.service.set_capture(None);
        self.service.set_activity_watch(false, false);
        platform::emergency_release_all();
        if self.settings_dirty {
            self.save_settings_now();
        }
    }
}

impl eframe::App for SentinelApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.apply_theme_if_needed(&ctx);
        self.handle_platform_events(&ctx);
        self.poll_environment();

        // Countdown → start recording.
        if let Mode::RecordCountdown {
            start_at, by_hotkey, ..
        } = self.mode
        {
            if Instant::now() >= start_at {
                self.begin_capture(by_hotkey);
            }
        }
        self.drain_capture();
        self.finish_playback_if_done(&ctx);
        self.finish_clicker_if_done();
        self.handle_shortcuts(&ctx);
        self.handle_dropped_files(&ctx);
        self.handle_close(&ctx);

        let p = self.palette;
        if self.settings.ui.compact {
            self.compact_ui(ui);
        } else {
            self.full_ui(ui);
        }

        self.overlay(&ctx);
        self.show_modal(&ctx);
        self.show_toasts(&ctx, &p);

        if self.settings_dirty && self.settings_saved_at.elapsed() > Duration::from_secs(1) {
            self.save_settings_now();
        }

        let busy = !self.mode.is_idle();
        ctx.request_repaint_after(if busy {
            Duration::from_millis(100)
        } else {
            Duration::from_millis(700)
        });
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.shutdown();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.palette.bg.to_normalized_gamma_f32()
    }
}

// ---------------------------------------------------------------------
// Layout pieces
// ---------------------------------------------------------------------

impl SentinelApp {
    fn full_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        egui::Panel::left("nav")
            .exact_size(212.0)
            .resizable(false)
            .frame(
                egui::Frame::new()
                    .fill(p.sidebar)
                    .inner_margin(Margin::symmetric(12, 16)),
            )
            .show(ui, |ui| self.sidebar(ui));

        egui::Panel::bottom("status")
            .exact_size(34.0)
            .frame(
                egui::Frame::new()
                    .fill(p.sidebar)
                    .stroke(Stroke::new(1.0, p.border))
                    .inner_margin(Margin::symmetric(14, 6)),
            )
            .show(ui, |ui| self.status_bar(ui));

        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(26, 20)))
            .show(ui, |ui| {
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    ui.set_max_width(ui.available_width().min(1100.0));
                    if self.player_mode {
                        self.page_player(ui);
                        return;
                    }
                    match self.page {
                        Page::Recorder => self.page_recorder(ui),
                        Page::Library => self.page_library(ui),
                        Page::Editor => self.page_editor(ui),
                        Page::Clicker => self.page_clicker(ui),
                        Page::Settings => self.page_settings(ui),
                        Page::About => self.page_about(ui),
                    }
                });
            });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal(|ui| {
            widgets::logo(ui, &p, 34.0);
            ui.vertical(|ui| {
                ui.add_space(1.0);
                ui.label(RichText::new("SentinelTask").size(16.5).strong().color(p.text));
                ui.add_space(-7.0);
                ui.label(
                    RichText::new(format!("v{}", sentinel_core::APP_VERSION))
                        .size(11.0)
                        .color(p.text_faint),
                );
            });
        });
        ui.add_space(18.0);
        if self.player_mode {
            ui.label(RichText::new("Standalone player").color(p.text_dim));
        } else {
            for (page, icon, label) in Page::NAV {
                let selected = self.page == page;
                let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
                let hovered = resp.hovered();
                if selected || hovered {
                    ui.painter().rect_filled(
                        rect,
                        CornerRadius::same(8),
                        if selected { p.accent_soft } else { p.card_hover },
                    );
                }
                if selected {
                    let bar = egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 9.0), vec2(3.0, 20.0));
                    ui.painter().rect_filled(bar, CornerRadius::same(2), p.accent);
                }
                let col = if selected { p.accent } else { p.text_dim };
                paint_icon(
                    ui.painter(),
                    egui::Rect::from_center_size(pos2(rect.left() + 24.0, rect.center().y), vec2(18.0, 18.0)),
                    icon,
                    if icon == Icon::Record && matches!(self.mode, Mode::Recording { .. }) {
                        p.record
                    } else {
                        col
                    },
                );
                ui.painter().text(
                    pos2(rect.left() + 44.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    label,
                    egui::FontId::proportional(14.0),
                    if selected { p.text } else { p.text_dim },
                );
                if page == Page::Editor && self.has_unsaved_changes() {
                    ui.painter()
                        .circle_filled(pos2(rect.right() - 14.0, rect.center().y), 3.5, p.pause);
                }
                if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                    self.page = page;
                }
                ui.add_space(2.0);
            }
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
            let (txt, fg, bg) = if self.elevated {
                ("Administrator", p.play, p.ok_bg)
            } else {
                ("Standard user", p.text_dim, p.raised)
            };
            let r = widgets::pill(ui, txt, fg, bg);
            r.on_hover_text(if self.elevated {
                "SentinelTask is running as administrator and can control every window."
            } else {
                "Windows doesn't let normal programs control windows that run as administrator. \
                 If a target app is elevated, restart SentinelTask as administrator."
            });
            ui.add_space(6.0);
            if widgets::tool_button(ui, &p, Icon::Compact, "Compact mode", true)
                .on_hover_text("A tiny always-on-top toolbar, like TinyTask")
                .clicked()
            {
                self.settings.ui.compact = true;
                self.mark_settings_dirty();
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        ui.horizontal_centered(|ui| {
            let (dot, text) = match &self.mode {
                Mode::Idle => (p.text_faint, "Ready".to_string()),
                Mode::RecordCountdown { start_at, .. } => {
                    let s = start_at.saturating_duration_since(Instant::now()).as_secs() + 1;
                    (p.record, format!("Recording starts in {s}…"))
                }
                Mode::Recording { session, .. } => (
                    p.record,
                    format!(
                        "Recording · {} · {} events",
                        format_clock_us(platform::now_us().saturating_sub(session.start_us())),
                        session.len()
                    ),
                ),
                Mode::Playing { handle } => {
                    let s = handle.status();
                    match s.state {
                        RunState::Paused(PauseReason::User) => (p.pause, "Paused".into()),
                        RunState::Paused(PauseReason::WaitingForWindow(w)) => (p.pause, format!("Waiting for {w}")),
                        RunState::Countdown { remaining_ms } => {
                            (p.play, format!("Starting in {}…", remaining_ms / 1000 + 1))
                        }
                        _ => (
                            p.play,
                            format!("Playing · pass {} · {}", s.pass, format_clock_us(s.elapsed_us)),
                        ),
                    }
                }
                Mode::Clicking { handle } => (p.play, format!("Auto-clicking · {} clicks", handle.clicks())),
            };
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 4.5, dot);
            ui.label(RichText::new(text).size(12.5).color(p.text_dim));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let record = if self.player_mode {
                    String::new()
                } else {
                    format!("{} record   ·   ", self.hotkey_text(HotkeyAction::Record))
                };
                let hint = format!(
                    "{record}{} play/stop   ·   {} pause   ·   {} emergency stop",
                    self.hotkey_text(HotkeyAction::Play),
                    self.hotkey_text(HotkeyAction::Pause),
                    self.hotkey_text(HotkeyAction::EmergencyStop),
                );
                ui.label(RichText::new(hint).size(11.5).color(p.text_faint));
            });
        });
    }

    /// TinyTask-style mini toolbar.
    fn compact_ui(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(10, 10)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    let idle = self.mode.is_idle();
                    let recording = matches!(self.mode, Mode::Recording { .. } | Mode::RecordCountdown { .. });
                    let playing = matches!(self.mode, Mode::Playing { .. });
                    if widgets::tool_button(ui, &p, Icon::Folder, "", idle)
                        .on_hover_text("Open")
                        .clicked()
                    {
                        self.open_dialog();
                    }
                    if widgets::tool_button(ui, &p, Icon::Save, "", idle && self.doc.is_some())
                        .on_hover_text("Save")
                        .clicked()
                    {
                        self.save();
                    }
                    ui.separator();
                    let rec_label = if recording { "Stop" } else { "Rec" };
                    if widgets::tool_button_tinted(
                        ui,
                        &p,
                        if recording { Icon::Stop } else { Icon::Record },
                        rec_label,
                        idle || recording,
                        Some(p.record),
                    )
                    .on_hover_text(self.hotkey_text(HotkeyAction::Record))
                    .clicked()
                    {
                        if recording {
                            self.stop_recording(&ctx, None);
                        } else {
                            self.start_recording(&ctx, false);
                        }
                    }
                    let play_label = if playing { "Stop" } else { "Play" };
                    if widgets::tool_button_tinted(
                        ui,
                        &p,
                        if playing { Icon::Stop } else { Icon::Play },
                        play_label,
                        (idle && self.doc.is_some()) || playing,
                        Some(p.play),
                    )
                    .on_hover_text(self.hotkey_text(HotkeyAction::Play))
                    .clicked()
                    {
                        if playing {
                            self.stop_playback();
                        } else {
                            self.start_playback(&ctx, false);
                        }
                    }
                    ui.separator();
                    if let Some(doc) = self.doc.as_mut() {
                        let mut speed = doc.mac.playback.speed;
                        let r = ui.add_enabled(
                            idle,
                            egui::DragValue::new(&mut speed)
                                .range(0.1..=100.0)
                                .speed(0.05)
                                .suffix("×")
                                .max_decimals(2),
                        );
                        if r.changed() {
                            doc.mac.playback.speed = speed;
                            doc.dirty = true;
                        }
                        r.on_hover_text("Playback speed");
                        let mut forever = matches!(doc.mac.playback.repeat, sentinel_core::Repeat::Forever);
                        if ui
                            .add_enabled(idle, egui::Checkbox::new(&mut forever, "Loop"))
                            .changed()
                        {
                            doc.mac.playback.repeat = if forever {
                                sentinel_core::Repeat::Forever
                            } else {
                                sentinel_core::Repeat::Times { count: 1 }
                            };
                            doc.dirty = true;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::tool_button(ui, &p, Icon::Expand, "", true)
                            .on_hover_text("Full window")
                            .clicked()
                        {
                            self.settings.ui.compact = false;
                            self.mark_settings_dirty();
                        }
                    });
                });
                ui.add_space(4.0);
                let line = match (&self.mode, &self.doc) {
                    (Mode::Idle, Some(d)) => format!(
                        "{} · {} · {}",
                        d.mac.meta.name,
                        format_duration_us(d.mac.duration_us()),
                        d.mac.playback.repeat.describe()
                    ),
                    (Mode::Idle, None) => format!("Press {} to record", self.hotkey_text(HotkeyAction::Record)),
                    _ => self.mode_summary(),
                };
                ui.label(RichText::new(line).size(11.5).color(p.text_faint));
            });
    }

    pub fn mode_summary(&self) -> String {
        match &self.mode {
            Mode::Idle => "Ready".into(),
            Mode::RecordCountdown { start_at, .. } => {
                format!(
                    "Recording in {}…",
                    start_at.saturating_duration_since(Instant::now()).as_secs() + 1
                )
            }
            Mode::Recording { session, .. } => format!(
                "REC {} · {} clicks · {} keys",
                format_clock_us(platform::now_us().saturating_sub(session.start_us())),
                session.clicks,
                session.keys
            ),
            Mode::Playing { handle } => {
                let s = handle.status();
                let total = s.total_passes.map(|t| format!("/{t}")).unwrap_or_default();
                match s.state {
                    RunState::Paused(PauseReason::User) => format!("Paused · pass {}{total}", s.pass),
                    RunState::Paused(PauseReason::WaitingForWindow(w)) => format!("Waiting for {w} to be in front"),
                    RunState::Countdown { remaining_ms } => format!("Starting in {}…", remaining_ms / 1000 + 1),
                    RunState::WaitingForModifiers => "Release Ctrl/Alt/Shift to start…".into(),
                    _ => format!("Playing · pass {}{total} · {}", s.pass, format_clock_us(s.elapsed_us)),
                }
            }
            Mode::Clicking { handle } => format!(
                "Auto-clicking · {} clicks · {}",
                handle.clicks(),
                format_clock_us(handle.elapsed_us())
            ),
        }
    }

    /// Click-through, always-on-top status overlay (never steals clicks or
    /// focus from the macro's target).
    fn overlay(&mut self, ctx: &egui::Context) {
        let corner = self.settings.ui.overlay;
        if self.mode.is_idle() || corner == OverlayCorner::Hidden {
            return;
        }
        let p = self.palette;
        let summary = self.mode_summary();
        let stop_hint = format!("{} to stop", self.hotkey_text(HotkeyAction::EmergencyStop));
        let accent = match &self.mode {
            Mode::Recording { .. } | Mode::RecordCountdown { .. } => p.record,
            Mode::Playing { handle } if matches!(handle.status().state, RunState::Paused(_)) => p.pause,
            _ => p.play,
        };
        let progress = match &self.mode {
            Mode::Playing { handle } => Some(handle.status().pass_progress),
            _ => None,
        };
        let size = vec2(300.0, 58.0);
        let scale = (self.screen.dpi as f32 / 96.0).max(0.5);
        let prim = self.screen.primary;
        let (sw, sh) = (prim.w as f32 / scale, prim.h as f32 / scale);
        let margin = 18.0;
        let pos = match corner {
            OverlayCorner::TopLeft => pos2(margin, margin),
            OverlayCorner::TopRight | OverlayCorner::Hidden => pos2(sw - size.x - margin, margin),
            OverlayCorner::BottomLeft => pos2(margin, sh - size.y - margin - 48.0),
            OverlayCorner::BottomRight => pos2(sw - size.x - margin, sh - size.y - margin - 48.0),
        };
        let builder = egui::ViewportBuilder::default()
            .with_title("SentinelTask status")
            .with_inner_size(size)
            .with_position(pos)
            .with_decorations(false)
            .with_resizable(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_active(false);
        ctx.show_viewport_immediate(
            egui::ViewportId::from_hash_of("sentinel-overlay"),
            builder,
            move |ui, _class| {
                egui::CentralPanel::default()
                    .frame(egui::Frame::new().fill(Color32::TRANSPARENT))
                    .show(ui, |ui| {
                        let rect = ui.max_rect();
                        let painter = ui.painter();
                        painter.rect(
                            rect.shrink(1.0),
                            CornerRadius::same(12),
                            p.card.gamma_multiply(0.96),
                            Stroke::new(1.0, accent.gamma_multiply(0.8)),
                            StrokeKind::Inside,
                        );
                        let bar = egui::Rect::from_min_size(
                            rect.left_top() + vec2(1.0, 10.0),
                            vec2(4.0, rect.height() - 20.0),
                        );
                        painter.rect_filled(bar, CornerRadius::same(2), accent);
                        painter.text(
                            rect.left_top() + vec2(16.0, 18.0),
                            Align2::LEFT_CENTER,
                            &summary,
                            egui::FontId::proportional(14.0),
                            p.text,
                        );
                        painter.text(
                            rect.left_top() + vec2(16.0, 38.0),
                            Align2::LEFT_CENTER,
                            &stop_hint,
                            egui::FontId::proportional(11.5),
                            p.text_faint,
                        );
                        if let Some(pr) = progress {
                            let track = egui::Rect::from_min_size(
                                pos2(rect.left() + 16.0, rect.bottom() - 9.0),
                                vec2(rect.width() - 32.0, 3.0),
                            );
                            painter.rect_filled(track, CornerRadius::same(2), p.raised);
                            let mut fill = track;
                            fill.set_width(track.width() * pr);
                            painter.rect_filled(fill, CornerRadius::same(2), accent);
                        }
                    });
            },
        );
    }

    fn show_toasts(&mut self, ctx: &egui::Context, p: &Palette) {
        self.toasts.retain(|t| t.born.elapsed() < Duration::from_secs(5));
        if self.toasts.is_empty() {
            return;
        }
        ctx.request_repaint_after(Duration::from_millis(250));
        egui::Area::new(Id::unique("toasts"))
            .anchor(Align2::RIGHT_BOTTOM, vec2(-18.0, -48.0))
            .order(egui::Order::Foreground)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_max_width(380.0);
                for t in &self.toasts {
                    let col = match t.kind {
                        ToastKind::Info => p.accent,
                        ToastKind::Ok => p.play,
                        ToastKind::Warn => p.pause,
                        ToastKind::Error => p.danger,
                    };
                    let age = t.born.elapsed().as_secs_f32();
                    let alpha = if age > 4.4 {
                        ((5.0 - age) / 0.6).clamp(0.0, 1.0)
                    } else {
                        1.0
                    };
                    egui::Frame::new()
                        .fill(p.raised.gamma_multiply(alpha))
                        .stroke(Stroke::new(1.0, col.gamma_multiply(0.6 * alpha)))
                        .corner_radius(CornerRadius::same(10))
                        .inner_margin(Margin::symmetric(14, 10))
                        .shadow(egui::epaint::Shadow {
                            offset: [0, 4],
                            blur: 16,
                            spread: 0,
                            color: Color32::from_black_alpha((70.0 * alpha) as u8),
                        })
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                                ui.painter().circle_filled(r.center(), 4.0, col.gamma_multiply(alpha));
                                ui.label(RichText::new(&t.text).color(p.text.gamma_multiply(alpha)));
                            });
                        });
                    ui.add_space(6.0);
                }
            });
    }

    fn show_modal(&mut self, ctx: &egui::Context) {
        let Some(modal) = self.modal.take() else { return };
        let p = self.palette;
        let mut keep = true;
        let mut next: Option<Modal> = None;
        let mut action: Option<ModalAction> = None;
        let mut modal = modal;
        let resp = egui::Modal::new(Id::unique("modal")).show(ctx, |ui| {
            ui.set_width(420.0);
            match &mut modal {
                Modal::Message { title, text } => {
                    ui.label(RichText::new(title.as_str()).size(17.0).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new(text.as_str()).color(p.text_dim));
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, &p, "OK").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::ConfirmDelete(path, name) => {
                    ui.label(RichText::new("Delete macro?").size(17.0).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new(format!("“{name}” will be permanently deleted.")).color(p.text_dim));
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::danger_button(ui, &p, "Delete").clicked() {
                            let path = path.clone();
                            action = Some(Box::new(move |app, _| app.delete_file(&path)));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::Rename { path, name } => {
                    ui.label(RichText::new("Rename macro").size(17.0).strong());
                    ui.add_space(6.0);
                    let r = ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    r.request_focus();
                    let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::primary_button(ui, &p, "Rename").clicked() || enter {
                            let (path, name) = (path.clone(), name.clone());
                            action = Some(Box::new(move |app, _| app.rename_file(&path, &name)));
                            keep = false;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                }
                Modal::UnsavedOnClose | Modal::UnsavedBeforeOpen(_) => {
                    ui.label(RichText::new("Save changes?").size(17.0).strong());
                    ui.add_space(6.0);
                    ui.label(RichText::new("The current macro has unsaved changes.").color(p.text_dim));
                    ui.add_space(12.0);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let mut choice = None;
                        if widgets::primary_button(ui, &p, "Save").clicked() {
                            choice = Some(true);
                        }
                        if ui.button("Don't save").clicked() {
                            choice = Some(false);
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                        if let Some(save) = choice {
                            keep = false;
                            let m = std::mem::replace(
                                &mut modal,
                                Modal::Message {
                                    title: String::new(),
                                    text: String::new(),
                                },
                            );
                            action = Some(Box::new(move |app, ctx| {
                                if save && !app.save() {
                                    return;
                                }
                                if let Some(d) = app.doc.as_mut() {
                                    d.dirty = false;
                                }
                                match m {
                                    Modal::UnsavedOnClose => {
                                        app.allow_close = true;
                                        app.shutdown();
                                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                                    }
                                    Modal::UnsavedBeforeOpen(OpenTarget::File(p)) => app.open_path(&p),
                                    Modal::UnsavedBeforeOpen(OpenTarget::NewRecording) => {
                                        app.start_recording(ctx, false)
                                    }
                                    _ => {}
                                }
                            }));
                        }
                    });
                }
            }
        });
        if resp.should_close() && keep {
            keep = false;
        }
        if keep {
            next = Some(modal);
        }
        self.modal = next;
        if let Some(a) = action {
            a(self, ctx);
        }
    }

    pub fn delete_file(&mut self, path: &Path) {
        match std::fs::remove_file(path) {
            Ok(()) => {
                if self.doc.as_ref().and_then(|d| d.path.as_deref()) == Some(path) {
                    if let Some(d) = self.doc.as_mut() {
                        d.path = None;
                        d.dirty = true;
                    }
                }
                self.library.refresh();
                self.toast(ToastKind::Info, "Deleted.");
            }
            Err(e) => self.toast(ToastKind::Error, format!("Couldn't delete: {e}")),
        }
    }

    pub fn rename_file(&mut self, path: &Path, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        let mut m = match library::load_macro(path) {
            Ok(m) => m,
            Err(e) => {
                self.toast(ToastKind::Error, e);
                return;
            }
        };
        m.meta.name = name.to_string();
        let new_path = {
            let candidate = path.with_file_name(format!(
                "{}.{}",
                sentinel_core::util::sanitize_file_name(name),
                sentinel_core::format::FILE_EXTENSION
            ));
            if candidate != path && candidate.exists() {
                self.library.unique_path(name)
            } else {
                candidate
            }
        };
        if let Err(e) = library::save_macro(&new_path, &m) {
            self.toast(ToastKind::Error, e);
            return;
        }
        if new_path != path {
            let _ = std::fs::remove_file(path);
        }
        if let Some(d) = self.doc.as_mut() {
            if d.path.as_deref() == Some(path) {
                d.path = Some(new_path.clone());
                d.mac.meta.name = name.to_string();
            }
        }
        self.library.refresh();
    }
}
