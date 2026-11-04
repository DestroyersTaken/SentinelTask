//! Library, Auto Clicker, Settings and About pages.

use eframe::egui::{self, vec2, RichText};

use sentinel_core::clicker::{ClickAction, ClickLimit, ClickLocation};
use sentinel_core::settings::{OverlayCorner, Theme};
use sentinel_core::util::format_duration_us;
use sentinel_core::MouseButton;

use crate::platform::{self, HotkeyAction};

use super::app::{Modal, Mode, Page, SentinelApp, ToastKind};
use super::widgets::{self, BannerKind, Icon};

/// Map an egui key to a Windows virtual-key code.
pub fn egui_key_to_vk(k: egui::Key) -> Option<u16> {
    use egui::Key as K;
    use sentinel_core::keys::vk;
    let code = match k {
        K::ArrowDown => vk::DOWN,
        K::ArrowLeft => vk::LEFT,
        K::ArrowRight => vk::RIGHT,
        K::ArrowUp => vk::UP,
        K::Escape => vk::ESCAPE,
        K::Tab => vk::TAB,
        K::Backspace => vk::BACK,
        K::Enter => vk::RETURN,
        K::Space => vk::SPACE,
        K::Insert => vk::INSERT,
        K::Delete => vk::DELETE,
        K::Home => vk::HOME,
        K::End => vk::END,
        K::PageUp => vk::PRIOR,
        K::PageDown => vk::NEXT,
        K::Comma => 0xBC,
        K::Minus => 0xBD,
        K::Period => 0xBE,
        K::Slash => 0xBF,
        K::Backtick => 0xC0,
        K::OpenBracket => 0xDB,
        K::Backslash => 0xDC,
        K::CloseBracket => 0xDD,
        K::Quote => 0xDE,
        K::Semicolon => 0xBA,
        K::Equals => 0xBB,
        K::Pause => vk::PAUSE,
        K::ScrollLock => vk::SCROLL,
        K::PrintScreen => vk::SNAPSHOT,
        K::CapsLock => vk::CAPITAL,
        K::NumLock => vk::NUMLOCK,
        K::Menu => vk::APPS,
        _ => {
            let name = k.name();
            let b = name.as_bytes();
            if b.len() == 1 && (b[0].is_ascii_uppercase() || b[0].is_ascii_digit()) {
                b[0] as u16
            } else {
                let n = name.strip_prefix('F').and_then(|n| n.parse::<u16>().ok())?;
                if !(1..=24).contains(&n) {
                    return None;
                }
                vk::F1 + n - 1
            }
        }
    };
    Some(code)
}

/// The hotkey chosen by a key press in SentinelTask's own window, if any:
/// `Some(None)` = Esc (cancel), `Some(Some(hk))` = chosen, `None` = nothing yet.
fn captured_from_events(events: &[egui::Event]) -> Option<Option<sentinel_core::keys::Hotkey>> {
    for e in events {
        if let egui::Event::Key {
            key,
            pressed: true,
            repeat: false,
            modifiers,
            ..
        } = e
        {
            let Some(code) = egui_key_to_vk(*key) else { continue };
            if code == sentinel_core::keys::vk::ESCAPE && !modifiers.any() {
                return Some(None);
            }
            return Some(Some(sentinel_core::keys::Hotkey {
                ctrl: modifiers.ctrl,
                alt: modifiers.alt,
                shift: modifiers.shift,
                win: false,
                vk: code,
            }));
        }
    }
    None
}

fn ago(t: std::time::SystemTime) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(t)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} days ago", secs / 86_400),
    }
}

impl SentinelApp {
    // -----------------------------------------------------------------
    // Library
    // -----------------------------------------------------------------

    pub fn page_library(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        widgets::page_header(
            ui,
            &p,
            "Library",
            "All your saved macros. Recordings are saved here automatically.",
        );
        let idle = self.mode.is_idle();
        let mut open_dialog = false;
        let mut refresh = false;
        let mut open_folder = false;
        ui.horizontal(|ui| {
            ui.add(
                egui::TextEdit::singleline(&mut self.library_search)
                    .hint_text("Search macros…")
                    .desired_width(260.0),
            );
            open_dialog = widgets::tool_button(ui, &p, Icon::Import, "Open / import .rec…", idle).clicked();
            open_folder = widgets::tool_button(ui, &p, Icon::Folder, "Show folder", true).clicked();
            refresh = ui.button("Refresh").clicked();
        });
        ui.label(
            RichText::new(self.library.dir.display().to_string())
                .size(12.0)
                .color(p.text_faint),
        );
        ui.add_space(8.0);
        if open_dialog {
            self.open_dialog();
        }
        if open_folder {
            let _ = std::fs::create_dir_all(&self.library.dir);
            platform::open_folder(&self.library.dir);
        }
        if refresh {
            self.library.refresh();
        }

        let q = self.library_search.to_lowercase();
        let entries: Vec<_> = self
            .library
            .entries
            .iter()
            .filter(|e| q.is_empty() || e.name.to_lowercase().contains(&q) || e.description.to_lowercase().contains(&q))
            .cloned()
            .collect();
        if entries.is_empty() {
            widgets::card(ui, &p, |ui| {
                widgets::card_title(
                    ui,
                    &p,
                    if self.library.entries.is_empty() {
                        "Your library is empty"
                    } else {
                        "No matches"
                    },
                    Some("Recordings appear here automatically. You can also import TinyTask .rec files."),
                );
            });
            return;
        }
        let current = self.doc.as_ref().and_then(|d| d.path.clone());
        let mut open: Option<std::path::PathBuf> = None;
        let mut play: Option<std::path::PathBuf> = None;
        for e in entries {
            let is_current = current.as_deref() == Some(e.path.as_path());
            egui::Frame::new()
                .fill(if is_current { p.accent_soft } else { p.card })
                .stroke(egui::Stroke::new(
                    1.0,
                    if is_current {
                        p.accent.gamma_multiply(0.6)
                    } else {
                        p.border
                    },
                ))
                .corner_radius(egui::CornerRadius::same(10))
                .inner_margin(egui::Margin::symmetric(14, 10))
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - 330.0).max(180.0));
                            ui.label(RichText::new(&e.name).size(15.0).strong().color(p.text));
                            ui.add_space(-5.0);
                            let line = match &e.error {
                                Some(err) => err.clone(),
                                None => format!(
                                    "{} · {} clicks · {} keys · {}",
                                    format_duration_us(e.stats.duration_us),
                                    e.stats.clicks,
                                    e.stats.key_presses,
                                    ago(e.modified)
                                ),
                            };
                            ui.label(RichText::new(line).size(12.0).color(if e.error.is_some() {
                                p.danger
                            } else {
                                p.text_faint
                            }));
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if widgets::tool_button(ui, &p, Icon::Trash, "", idle)
                                .on_hover_text("Delete")
                                .clicked()
                            {
                                self.modal = Some(Modal::ConfirmDelete(e.path.clone(), e.name.clone()));
                            }
                            if widgets::tool_button(ui, &p, Icon::Folder, "", true)
                                .on_hover_text("Show in Explorer")
                                .clicked()
                            {
                                platform::reveal_in_explorer(&e.path);
                            }
                            if widgets::tool_button(ui, &p, Icon::Edit, "", idle && e.error.is_none())
                                .on_hover_text("Rename")
                                .clicked()
                            {
                                self.modal = Some(Modal::Rename {
                                    path: e.path.clone(),
                                    name: e.name.clone(),
                                });
                            }
                            if widgets::tool_button(ui, &p, Icon::Play, "Play", idle && e.error.is_none()).clicked() {
                                play = Some(e.path.clone());
                            }
                            if widgets::tool_button(ui, &p, Icon::Expand, "Open", idle && e.error.is_none()).clicked() {
                                open = Some(e.path.clone());
                            }
                        });
                    });
                });
            ui.add_space(6.0);
        }
        if let Some(path) = open {
            self.request_open(path);
            if self.modal.is_none() {
                self.page = Page::Recorder;
            }
        }
        if let Some(path) = play {
            self.request_open(path);
            if self.modal.is_none() {
                self.page = Page::Recorder;
                let ctx = ui.ctx().clone();
                self.start_playback(&ctx, false);
            }
        }
    }

    // -----------------------------------------------------------------
    // Auto clicker
    // -----------------------------------------------------------------

    pub fn page_clicker(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        widgets::page_header(
            ui,
            &p,
            "Auto Clicker",
            "Click or press a key repeatedly at a steady rate — no recording needed.",
        );
        let running = matches!(self.mode, Mode::Clicking { .. });
        let idle = self.mode.is_idle();
        widgets::card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                let r = widgets::big_action(
                    ui,
                    &p,
                    if running { Icon::Stop } else { Icon::Play },
                    if running { "Stop" } else { "Start" },
                    &self.hotkey_text(HotkeyAction::Clicker),
                    p.play,
                    running,
                    idle || running,
                );
                if r.clicked() {
                    self.toggle_clicker(&ctx);
                }
                ui.add_space(10.0);
                ui.vertical(|ui| {
                    if let Mode::Clicking { handle } = &self.mode {
                        ui.label(RichText::new("Clicking…").size(22.0).strong().color(p.play));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 26.0;
                            widgets::stat(ui, &p, &handle.clicks().to_string(), "clicks");
                            widgets::stat(
                                ui,
                                &p,
                                &sentinel_core::util::format_clock_us(handle.elapsed_us()),
                                "elapsed",
                            );
                        });
                    } else {
                        let c = &self.settings.clicker;
                        ui.label(
                            RichText::new(format!("{:.1} clicks per second", c.clicks_per_second()))
                                .size(20.0)
                                .strong()
                                .color(p.text),
                        );
                        ui.label(
                            RichText::new(format!(
                                "Press {} anywhere to start or stop. {} stops everything.",
                                self.hotkey_text(HotkeyAction::Clicker),
                                self.hotkey_text(HotkeyAction::EmergencyStop)
                            ))
                            .color(p.text_dim),
                        );
                    }
                });
            });
        });
        ui.add_space(12.0);

        let mut changed = false;
        let mut pick = false;
        let mut cancel_pick = self.picking_point && ui.input(|i| i.key_pressed(egui::Key::Escape));
        ui.add_enabled_ui(idle, |ui| {
            widgets::card(ui, &p, |ui| {
                widgets::card_title(ui, &p, "What to click", None);
                let c = &mut self.settings.clicker;
                #[derive(Clone, Copy, PartialEq)]
                enum A {
                    Mouse,
                    Key,
                }
                let mut a = match c.action {
                    ClickAction::Mouse { .. } => A::Mouse,
                    ClickAction::Key { .. } => A::Key,
                };
                widgets::form_row(ui, &p, "Action", |ui| {
                    if widgets::segmented(ui, &p, &mut a, &[(A::Mouse, "Mouse button"), (A::Key, "Keyboard key")]) {
                        c.action = match a {
                            A::Mouse => ClickAction::Mouse {
                                button: MouseButton::Left,
                                double: false,
                            },
                            A::Key => ClickAction::Key {
                                key: sentinel_core::keys::Hotkey::key(sentinel_core::keys::vk::SPACE),
                            },
                        };
                        changed = true;
                    }
                });
                match &mut c.action {
                    ClickAction::Mouse { button, double } => {
                        widgets::form_row(ui, &p, "Button", |ui| {
                            changed |= widgets::segmented(
                                ui,
                                &p,
                                button,
                                &[
                                    (MouseButton::Left, "Left"),
                                    (MouseButton::Right, "Right"),
                                    (MouseButton::Middle, "Middle"),
                                    (MouseButton::X1, "Back"),
                                    (MouseButton::X2, "Forward"),
                                ],
                            );
                        });
                        widgets::form_row(ui, &p, "Click type", |ui| {
                            changed |= widgets::segmented(ui, &p, double, &[(false, "Single"), (true, "Double")]);
                        });
                    }
                    ClickAction::Key { key } => {
                        widgets::form_row(ui, &p, "Key", |ui| {
                            let mut text = key.to_string();
                            let r = ui.add(
                                egui::TextEdit::singleline(&mut text)
                                    .desired_width(200.0)
                                    .hint_text("e.g. Space, E, Ctrl+F"),
                            );
                            if r.changed() {
                                if let Some(h) = sentinel_core::keys::Hotkey::parse(&text) {
                                    *key = h;
                                    changed = true;
                                }
                            }
                            ui.label(
                                RichText::new("Type a key name, e.g. Space, E, F5, Ctrl+V")
                                    .size(12.0)
                                    .color(p.text_faint),
                            );
                        });
                    }
                }
                widgets::form_row(ui, &p, "Where", |ui| {
                    let mut fixed = matches!(c.location, ClickLocation::Fixed { .. });
                    if widgets::segmented(
                        ui,
                        &p,
                        &mut fixed,
                        &[(false, "Wherever the cursor is"), (true, "A fixed spot")],
                    ) {
                        c.location = if fixed {
                            let (x, y) = platform::cursor_pos();
                            ClickLocation::Fixed { x, y }
                        } else {
                            ClickLocation::Cursor
                        };
                        changed = true;
                    }
                    if let ClickLocation::Fixed { x, y } = &mut c.location {
                        changed |= ui.add(egui::DragValue::new(x).prefix("x ")).changed();
                        changed |= ui.add(egui::DragValue::new(y).prefix("y ")).changed();
                        if self.picking_point {
                            cancel_pick = ui.button("Cancel picking").clicked();
                            ui.label(RichText::new("Left-click the spot…").color(p.accent));
                        } else {
                            pick = ui
                                .button("Pick on screen…")
                                .on_hover_text("Then left-click the spot (Esc cancels)")
                                .clicked();
                        }
                    }
                });
            });
            ui.add_space(12.0);
            widgets::card(ui, &p, |ui| {
                widgets::card_title(ui, &p, "Timing", None);
                let c = &mut self.settings.clicker;
                widgets::form_row(ui, &p, "Every", |ui| {
                    changed |= ui
                        .add(
                            egui::DragValue::new(&mut c.interval_ms)
                                .range(1..=86_400_000)
                                .speed(1.0)
                                .suffix(" ms"),
                        )
                        .changed();
                    for (label, ms) in [("10/s", 100), ("20/s", 50), ("1/s", 1000), ("Every 5 s", 5000)] {
                        if ui.selectable_label(c.interval_ms == ms, label).clicked() {
                            c.interval_ms = ms;
                            changed = true;
                        }
                    }
                });
                widgets::form_row(ui, &p, "Hold for", |ui| {
                    changed |= ui
                        .add(egui::DragValue::new(&mut c.hold_ms).range(1..=60_000).suffix(" ms"))
                        .changed();
                    ui.label(
                        RichText::new("how long each press lasts")
                            .size(12.0)
                            .color(p.text_faint),
                    );
                });
                #[derive(Clone, Copy, PartialEq)]
                enum L {
                    Unlimited,
                    Count,
                    Duration,
                }
                let mut l = match c.limit {
                    ClickLimit::Unlimited => L::Unlimited,
                    ClickLimit::Count { .. } => L::Count,
                    ClickLimit::Duration { .. } => L::Duration,
                };
                widgets::form_row(ui, &p, "Stop", |ui| {
                    if widgets::segmented(
                        ui,
                        &p,
                        &mut l,
                        &[
                            (L::Unlimited, "When I stop it"),
                            (L::Count, "After N clicks"),
                            (L::Duration, "After a time"),
                        ],
                    ) {
                        c.limit = match l {
                            L::Unlimited => ClickLimit::Unlimited,
                            L::Count => ClickLimit::Count { clicks: 100 },
                            L::Duration => ClickLimit::Duration { secs: 600 },
                        };
                        changed = true;
                    }
                    match &mut c.limit {
                        ClickLimit::Count { clicks } => {
                            changed |= ui
                                .add(egui::DragValue::new(clicks).range(1..=100_000_000).suffix(" clicks"))
                                .changed();
                        }
                        ClickLimit::Duration { secs } => {
                            let mut m = *secs / 60;
                            if ui
                                .add(egui::DragValue::new(&mut m).range(1..=100_000).suffix(" min"))
                                .changed()
                            {
                                *secs = m.max(1) * 60;
                                changed = true;
                            }
                        }
                        ClickLimit::Unlimited => {}
                    }
                });
            });
        });
        if changed {
            self.mark_settings_dirty();
        }
        if cancel_pick {
            self.cancel_point_pick();
        }
        if pick {
            self.picking_point = true;
            self.service.begin_point_pick(true);
            self.toast(ToastKind::Info, "Left-click the spot to use (Esc cancels).");
        }
    }

    // -----------------------------------------------------------------
    // Settings
    // -----------------------------------------------------------------

    pub fn page_settings(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        // Fallback for choosing a hotkey: if the system-wide keyboard hook
        // can't deliver (blocked by security software, etc.), read the keys
        // straight from SentinelTask's own window.
        if self.capturing_hotkey.is_some() {
            if let Some(result) = ui.input(|i| captured_from_events(&i.events)) {
                self.finish_key_capture(result);
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(100));
        }
        widgets::page_header(
            ui,
            &p,
            "Settings",
            "Hotkeys, recording, safety and appearance. Changes save automatically.",
        );
        let idle = self.mode.is_idle();

        // Hotkeys.
        widgets::card(ui, &p, |ui| {
            widgets::card_title(
                ui,
                &p,
                "Hotkeys",
                Some("Global shortcuts that work in any program. Click one and press the new combination."),
            );
            for a in HotkeyAction::ALL {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(150.0, 30.0), egui::Sense::hover());
                    ui.painter().text(
                        r.left_center(),
                        egui::Align2::LEFT_CENTER,
                        a.label(),
                        egui::FontId::proportional(14.0),
                        p.text,
                    );
                    let capturing = self.capturing_hotkey == Some(a);
                    let label = if capturing {
                        "Press the new keys…".to_string()
                    } else {
                        self.hotkey_text(a)
                    };
                    let btn = egui::Button::new(RichText::new(label).monospace())
                        .min_size(vec2(240.0, 30.0))
                        .selected(capturing);
                    if ui
                        .add_enabled(idle && (self.capturing_hotkey.is_none() || capturing), btn)
                        .clicked()
                        && !capturing
                    {
                        self.begin_key_capture(a);
                    }
                    if capturing {
                        if ui.button("Cancel").clicked() {
                            self.finish_key_capture(None);
                        }
                        ui.label(RichText::new("or press Esc").size(12.0).color(p.text_faint));
                        return;
                    }
                    match self.hotkey_error(a) {
                        Some(e) if e.starts_with(crate::platform::SHARED_HOTKEY_PREFIX) => {
                            widgets::pill(ui, "Active · shared", p.warn_fg, p.warn_bg).on_hover_text(e);
                            ui.label(
                                RichText::new("Another program uses this too")
                                    .size(12.0)
                                    .color(p.warn_fg),
                            )
                            .on_hover_text(e);
                        }
                        Some(e) => {
                            widgets::pill(ui, "Unavailable", p.danger, p.err_bg).on_hover_text(e);
                            ui.label(RichText::new(e).size(12.0).color(p.danger));
                        }
                        None => {
                            widgets::pill(ui, "Active", p.play, p.ok_bg);
                        }
                    }
                });
            }
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui
                    .add_enabled(
                        idle && self.capturing_hotkey.is_none(),
                        egui::Button::new("Restore TinyTask-style defaults"),
                    )
                    .clicked()
                {
                    self.settings.hotkeys = Default::default();
                    self.mark_settings_dirty();
                    self.reregister_hotkeys();
                }
                ui.label(
                    RichText::new("Emergency stop always stops everything and releases every key and mouse button.")
                        .size(12.0)
                        .color(p.text_faint),
                );
            });
        });
        ui.add_space(12.0);

        let mut changed = false;
        widgets::card(ui, &p, |ui| {
            widgets::card_title(ui, &p, "Recording", None);
            let r = &mut self.settings.recording;
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut r.record_mouse_moves,
                "Record mouse movement",
                "Turn off to record only clicks, scrolls and keys.",
            );
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut r.record_raw_deltas,
                "Record raw mouse motion",
                "Needed to replay camera movement in 3D games that lock the cursor.",
            );
            changed |= widgets::toggle_row(ui, &p, &mut r.record_keyboard, "Record keyboard", "");
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut r.capture_target_window,
                "Remember the target window",
                "Stores which program you recorded in, for the “only play into the target window” option.",
            );
            widgets::form_row(ui, &p, "Mouse sample rate", |ui| {
                changed |= widgets::segmented(
                    ui,
                    &p,
                    &mut r.mouse_sample_hz,
                    &[(125, "125 Hz"), (250, "250 Hz"), (500, "500 Hz"), (1000, "1000 Hz")],
                );
            });
            widgets::form_row(ui, &p, "Countdown", |ui| {
                changed |= ui
                    .add(egui::DragValue::new(&mut r.countdown_secs).range(0..=30).suffix(" s"))
                    .changed();
                ui.label(RichText::new("before recording starts").size(12.0).color(p.text_faint));
            });
        });
        ui.add_space(12.0);

        widgets::card(ui, &p, |ui| {
            widgets::card_title(ui, &p, "Playback & safety", None);
            let s = &mut self.settings.safety;
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut s.prevent_sleep,
                "Keep the PC awake while playing",
                "Stops Windows from sleeping or turning off the screen during long runs.",
            );
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut s.stop_on_user_mouse,
                "Stop when I move or click the mouse",
                "Take back control instantly by using the mouse.",
            );
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut s.stop_on_user_key,
                "Stop when I press a key",
                "Hotkeys don't count.",
            );
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut s.warn_elevation,
                "Warn about administrator windows",
                "Explain when Windows would silently block input to an elevated program.",
            );
            widgets::form_row(ui, &p, "Countdown", |ui| {
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut s.play_countdown_secs)
                            .range(0..=30)
                            .suffix(" s"),
                    )
                    .changed();
                ui.label(RichText::new("before playback starts").size(12.0).color(p.text_faint));
            });
            if widgets::toggle_row(
                ui,
                &p,
                &mut s.virtual_key_injection,
                "Compatibility keyboard mode",
                "Send virtual keys instead of hardware scan codes. Leave off for games; try it if keys don't arrive over remote desktop.",
            ) {
                platform::set_virtual_key_injection(s.virtual_key_injection);
                changed = true;
            }
        });
        ui.add_space(12.0);

        let mut theme_changed = false;
        widgets::card(ui, &p, |ui| {
            widgets::card_title(ui, &p, "Appearance", None);
            let u = &mut self.settings.ui;
            widgets::form_row(ui, &p, "Theme", |ui| {
                theme_changed |=
                    widgets::segmented(ui, &p, &mut u.theme, &[(Theme::Dark, "Dark"), (Theme::Light, "Light")]);
            });
            widgets::form_row(ui, &p, "Status overlay", |ui| {
                changed |= widgets::segmented(
                    ui,
                    &p,
                    &mut u.overlay,
                    &[
                        (OverlayCorner::TopLeft, "Top left"),
                        (OverlayCorner::TopRight, "Top right"),
                        (OverlayCorner::BottomLeft, "Bottom left"),
                        (OverlayCorner::BottomRight, "Bottom right"),
                        (OverlayCorner::Hidden, "Off"),
                    ],
                );
            });
            widgets::form_row(ui, &p, "Zoom", |ui| {
                theme_changed |= ui
                    .add(egui::Slider::new(&mut u.zoom, 0.8..=1.6).step_by(0.05).suffix("×"))
                    .drag_stopped();
            });
            changed |= widgets::toggle_row(
                ui,
                &p,
                &mut u.minimize_while_running,
                "Minimize while recording or playing",
                "",
            );
            let mut top = u.always_on_top;
            if widgets::toggle_row(ui, &p, &mut top, "Always on top", "") {
                u.always_on_top = top;
                ui.ctx().send_viewport_cmd(egui::ViewportCommand::WindowLevel(if top {
                    egui::WindowLevel::AlwaysOnTop
                } else {
                    egui::WindowLevel::Normal
                }));
                changed = true;
            }
        });
        ui.add_space(12.0);

        self.updates_card(ui, &mut changed);
        ui.add_space(12.0);

        let mut pick_dir = false;
        let mut reset_dir = false;
        widgets::card(ui, &p, |ui| {
            widgets::card_title(ui, &p, "Storage", None);
            widgets::form_row(ui, &p, "Library folder", |ui| {
                ui.label(RichText::new(self.library.dir.display().to_string()).color(p.text_dim));
            });
            ui.horizontal(|ui| {
                ui.add_space(138.0);
                pick_dir = ui.add_enabled(idle, egui::Button::new("Change…")).clicked();
                reset_dir = ui
                    .add_enabled(
                        idle && !self.settings.library_dir.is_empty(),
                        egui::Button::new("Use default"),
                    )
                    .clicked();
            });
            widgets::form_row(ui, &p, "Mode", |ui| {
                ui.label(
                    RichText::new(if self.paths.portable {
                        "Portable — settings are stored next to SentinelTask.exe"
                    } else {
                        "Installed — create a file named “portable” next to the .exe to go portable"
                    })
                    .color(p.text_dim),
                );
            });
        });

        if pick_dir {
            if let Some(d) = platform::pick_folder_dialog("Choose library folder", Some(&self.library.dir)) {
                self.settings.library_dir = d.to_string_lossy().into_owned();
                self.library.set_dir(d);
                changed = true;
            }
        }
        if reset_dir {
            self.settings.library_dir.clear();
            let d = self.paths.library_dir("");
            let _ = std::fs::create_dir_all(&d);
            self.library.set_dir(d);
            changed = true;
        }
        if changed || theme_changed {
            self.mark_settings_dirty();
        }
    }

    fn updates_card(&mut self, ui: &mut egui::Ui, changed: &mut bool) {
        use crate::update::{InstallKind, Phase};
        let p = self.palette;
        let ctx = ui.ctx().clone();
        widgets::card(ui, &p, |ui| {
            let portable = self.updater.kind == InstallKind::Portable;
            widgets::card_title(
                ui,
                &p,
                "Updates",
                Some(if portable {
                    "Portable copy — you're asked before SentinelTask.exe is replaced."
                } else {
                    "New versions come from SentinelTask's GitHub releases page."
                }),
            );
            if !self.updater.enabled {
                ui.label(
                    RichText::new(if self.player_mode {
                        "Standalone players don't update themselves."
                    } else {
                        "Updates are turned off for this session."
                    })
                    .color(p.text_dim),
                );
                return;
            }
            let u = &mut self.settings.updates;
            *changed |= widgets::toggle_row(
                ui,
                &p,
                &mut u.check_automatically,
                "Check for updates automatically",
                "When SentinelTask starts and every few hours. Only the version is fetched from GitHub; nothing \
                 about you is sent.",
            );
            if !portable {
                *changed |= widgets::toggle_row(
                    ui,
                    &p,
                    &mut u.install_automatically,
                    "Install updates automatically",
                    "Downloads new versions in the background and installs them when you close SentinelTask. \
                     A running macro is never interrupted.",
                );
            }
            if !u.skipped_version.is_empty() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("Skipping version {}", u.skipped_version)).color(p.text_dim));
                    if ui.small_button("Don't skip").clicked() {
                        u.skipped_version.clear();
                        *changed = true;
                    }
                });
            }
            ui.add_space(4.0);
            let phase = self.updater.phase();
            let pending = self.pending_install.as_ref().map(|p| p.version);
            ui.horizontal(|ui| {
                let working = self.updater.is_working();
                match (&phase, pending) {
                    (_, Some(_)) | (Phase::Ready(_), _) => {
                        if widgets::primary_button(ui, &p, "Restart and update").clicked() {
                            self.update_now(&ctx);
                        }
                    }
                    (Phase::Available(_), _) => {
                        let label = if portable {
                            "Replace and restart"
                        } else {
                            "Update and restart"
                        };
                        if widgets::primary_button(ui, &p, label).clicked() {
                            self.update_now(&ctx);
                        }
                    }
                    (Phase::Downloading { .. }, _) => {
                        if ui.button("Cancel").clicked() {
                            self.updater.cancel();
                        }
                    }
                    _ => {
                        if ui.add_enabled(!working, egui::Button::new("Check now")).clicked() {
                            self.updater.check(true, self.settings.updates.install_automatically);
                        }
                    }
                }
                let status = match pending {
                    Some(v) => format!("Version {v} is ready — it installs when SentinelTask closes"),
                    None => crate::update::describe(&phase),
                };
                ui.label(RichText::new(status).color(p.text_dim));
                if let Phase::Downloading {
                    done, total: Some(t), ..
                } = phase
                {
                    if t > 0 {
                        ui.add(egui::ProgressBar::new(done as f32 / t as f32).desired_width(120.0));
                    }
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                } else if working {
                    ctx.request_repaint_after(std::time::Duration::from_millis(200));
                }
            });
        });
    }

    // -----------------------------------------------------------------
    // About
    // -----------------------------------------------------------------

    pub fn page_about(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        widgets::page_header(
            ui,
            &p,
            "About SentinelTask",
            "A modern, reliable successor to TinyTask.",
        );
        widgets::card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                widgets::logo(ui, &p, 56.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new("SentinelTask").size(22.0).strong());
                    ui.label(
                        RichText::new(format!(
                            "Version {}  ·  MIT licence  ·  64-bit",
                            sentinel_core::APP_VERSION
                        ))
                        .color(p.text_dim),
                    );
                    ui.horizontal(|ui| {
                        if ui.link("Release page").clicked() {
                            platform::open_url(crate::update::RELEASES_PAGE);
                        }
                        if self.updater.enabled && ui.link("Check for updates").clicked() {
                            self.page = Page::Settings;
                            self.updater.check(true, self.settings.updates.install_automatically);
                        }
                    });
                });
            });
            ui.add_space(8.0);
            ui.label(RichText::new("What it fixes compared to TinyTask").strong());
            for line in [
                "Exact clicks on scaled (125 %, 150 %…) and multi-monitor setups — fully DPI aware.",
                "Works in games: hardware scan codes for keys, raw relative motion for 3D cameras.",
                "No stuck keys: everything held is released on stop, pause, crash or exit.",
                "Drift-free timing for long loops, with a high-resolution timer.",
                "Your hotkeys never end up inside the macro; clicks on SentinelTask are ignored.",
                "Keeps the PC awake, can pause when the game loses focus, and warns about admin windows.",
                "Edit macros, rebind every hotkey, import TinyTask .rec files, auto-clicker built in.",
            ] {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(10.0, 18.0), egui::Sense::hover());
                    ui.painter().circle_filled(r.center(), 3.0, p.accent);
                    ui.label(RichText::new(line).color(p.text_dim));
                });
            }
        });
        ui.add_space(12.0);
        widgets::card(ui, &p, |ui| {
            widgets::card_title(ui, &p, "Diagnostics", Some("Useful when reporting a problem."));
            let s = self.screen;
            let rows = [
                (
                    "Running as",
                    if self.elevated {
                        "Administrator".to_string()
                    } else {
                        "Standard user".to_string()
                    },
                ),
                (
                    "Screen",
                    format!(
                        "{}×{} virtual desktop at {}, primary {}×{}, {} DPI",
                        s.desktop.w,
                        s.desktop.h,
                        format_args!("({}, {})", s.desktop.x, s.desktop.y),
                        s.primary.w,
                        s.primary.h,
                        s.dpi
                    ),
                ),
                ("Timer", self.sleeper_kind.to_string()),
                (
                    "Input capture",
                    if self.service.is_functional() {
                        "Low-level hooks + Raw Input".into()
                    } else {
                        "Unavailable on this OS".to_string()
                    },
                ),
                ("Settings", self.paths.settings_file().display().to_string()),
                (
                    "Log",
                    crate::logging::path()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default(),
                ),
            ];
            egui::Grid::new("diag")
                .num_columns(2)
                .spacing(vec2(18.0, 6.0))
                .show(ui, |ui| {
                    for (k, v) in rows {
                        ui.label(RichText::new(k).color(p.text_faint));
                        ui.label(RichText::new(v).color(p.text_dim));
                        ui.end_row();
                    }
                });
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if let Some(log) = crate::logging::path() {
                    if ui.button("Open log folder").clicked() {
                        if let Some(d) = log.parent() {
                            platform::open_folder(d);
                        }
                    }
                }
                if !self.elevated && ui.button("Restart as administrator").clicked() {
                    self.restart_elevated();
                }
            });
        });
        ui.add_space(12.0);
        widgets::banner(
            ui,
            &p,
            BannerKind::Info,
            "Use responsibly",
            "SentinelTask automates your own mouse and keyboard. Many online games forbid automation in their rules — check before you use it there.",
            &[],
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_core::keys::{vk, Hotkey};

    fn key_event(key: egui::Key, modifiers: egui::Modifiers, repeat: bool) -> egui::Event {
        egui::Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat,
            modifiers,
        }
    }

    #[test]
    fn egui_keys_map_to_virtual_keys() {
        assert_eq!(egui_key_to_vk(egui::Key::R), Some(0x52));
        assert_eq!(egui_key_to_vk(egui::Key::Num7), Some(0x37));
        assert_eq!(egui_key_to_vk(egui::Key::F8), Some(vk::F1 + 7));
        assert_eq!(egui_key_to_vk(egui::Key::F24), Some(vk::F24));
        assert_eq!(egui_key_to_vk(egui::Key::F25), None);
        assert_eq!(egui_key_to_vk(egui::Key::Pause), Some(vk::PAUSE));
        assert_eq!(egui_key_to_vk(egui::Key::ArrowUp), Some(vk::UP));
    }

    #[test]
    fn in_window_capture_reads_chords_and_escape() {
        let m = egui::Modifiers {
            ctrl: true,
            shift: true,
            ..Default::default()
        };
        assert_eq!(
            captured_from_events(&[key_event(egui::Key::R, m, false)]),
            Some(Some(Hotkey::with(true, false, true, 0x52)))
        );
        assert_eq!(
            captured_from_events(&[key_event(egui::Key::Escape, egui::Modifiers::NONE, false)]),
            Some(None)
        );
        // Auto-repeat and unmapped keys are ignored.
        assert_eq!(captured_from_events(&[key_event(egui::Key::R, m, true)]), None);
        assert_eq!(captured_from_events(&[key_event(egui::Key::F30, m, false)]), None);
        assert_eq!(captured_from_events(&[]), None);
    }
}
