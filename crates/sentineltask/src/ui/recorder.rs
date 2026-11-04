//! The Recorder page (home) and the standalone-player page.

use eframe::egui::{self, vec2, RichText};

use sentinel_core::playback::{MouseMode, PlaybackOptions, Repeat, MAX_SPEED, MIN_SPEED};
use sentinel_core::util::{format_clock_us, format_duration_us};
use sentinel_core::{ScreenInfo, TargetWindow};

use crate::engine::player::{PauseReason, RunState};
use crate::platform::{HotkeyAction, WindowInfo};

use super::app::{Mode, Page, SentinelApp};
use super::theme::Palette;
use super::widgets::{self, BannerKind, Icon};

#[derive(Default)]
struct DocActions {
    save: bool,
    save_as: bool,
    edit: bool,
    export: bool,
    set_defaults: bool,
}

impl SentinelApp {
    fn warnings(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if !self.service.is_functional() {
            widgets::banner(
                ui,
                &p,
                BannerKind::Info,
                "Preview build",
                "Recording and playback need Windows. This build only shows the interface.",
                &[],
            );
            ui.add_space(8.0);
        }
        if let Some(proc_name) = self.elevation_problem() {
            let text = format!(
                "{proc_name} runs as administrator. Windows blocks normal programs from controlling it, \
                 so clicks and keys would silently do nothing. Restart SentinelTask as administrator to fix this."
            );
            if widgets::banner(
                ui,
                &p,
                BannerKind::Warn,
                "Target window is elevated",
                &text,
                &["Restart as admin"],
            )
            .is_some()
            {
                self.restart_elevated();
            }
            ui.add_space(8.0);
        }
        if let Some(v) = self.pending_install.as_ref().map(|p| p.version) {
            if widgets::banner(
                ui,
                &p,
                BannerKind::Info,
                &format!("SentinelTask {v} is ready"),
                "It installs automatically when you close SentinelTask — or restart now to update.",
                &["Restart now"],
            )
            .is_some()
            {
                let ctx = ui.ctx().clone();
                self.update_now(&ctx);
            }
            ui.add_space(8.0);
        }
        let failed: Vec<String> = self
            .hotkey_status
            .iter()
            .filter(|r| r.is_broken())
            .filter_map(|r| {
                r.error
                    .as_ref()
                    .map(|e| format!("{} ({}): {e}", r.action.label(), r.hotkey))
            })
            .collect();
        if !failed.is_empty() {
            if widgets::banner(
                ui,
                &p,
                BannerKind::Warn,
                "Some hotkeys aren't available",
                &failed.join("\n"),
                &["Change hotkeys"],
            )
            .is_some()
            {
                self.page = Page::Settings;
            }
            ui.add_space(8.0);
        }
    }

    pub fn page_recorder(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        widgets::page_header(
            ui,
            &p,
            "Recorder",
            "Record your mouse and keyboard once, then replay it exactly — as often as you like.",
        );
        self.warnings(ui);
        self.controls_card(ui);
        ui.add_space(12.0);
        self.current_macro_card(ui);
    }

    pub fn page_player(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let name = self.doc.as_ref().map(|d| d.mac.meta.name.clone()).unwrap_or_default();
        widgets::page_header(ui, &p, &name, "Standalone SentinelTask player");
        self.warnings(ui);
        self.controls_card(ui);
        ui.add_space(12.0);
        if let Some(doc) = self.doc.as_mut() {
            let idle = self.mode.is_idle();
            let screen = self.screen;
            let last = self.last_foreign_window.clone();
            widgets::card(ui, &p, |ui| {
                widgets::card_title(ui, &p, "Playback", None);
                ui.add_enabled_ui(idle, |ui| {
                    playback_options_ui(
                        ui,
                        &p,
                        &mut doc.mac.playback,
                        &mut doc.mac.meta.target,
                        doc.mac.meta.screen,
                        screen,
                        last.as_ref(),
                    );
                });
            });
        }
    }

    fn controls_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let ctx = ui.ctx().clone();
        let recording = matches!(self.mode, Mode::Recording { .. } | Mode::RecordCountdown { .. });
        let playing = matches!(self.mode, Mode::Playing { .. });
        let idle = self.mode.is_idle();
        let has_doc = self.doc.as_ref().is_some_and(|d| !d.mac.events.is_empty());
        widgets::card(ui, &p, |ui| {
            // Wraps the status text below the buttons when the window is
            // narrow or the UI is zoomed in.
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                if !self.player_mode {
                    let r = widgets::big_action(
                        ui,
                        &p,
                        if recording { Icon::Stop } else { Icon::Record },
                        if recording { "Stop" } else { "Record" },
                        &self.hotkey_text(HotkeyAction::Record),
                        p.record,
                        recording,
                        idle || recording,
                    );
                    if r.clicked() {
                        if recording {
                            self.stop_recording(&ctx, None);
                        } else {
                            self.start_recording(&ctx, false);
                        }
                    }
                }
                let r = widgets::big_action(
                    ui,
                    &p,
                    if playing { Icon::Stop } else { Icon::Play },
                    if playing { "Stop" } else { "Play" },
                    &self.hotkey_text(HotkeyAction::Play),
                    p.play,
                    playing,
                    (idle && has_doc) || playing,
                );
                if r.clicked() {
                    if playing {
                        self.stop_playback();
                    } else {
                        self.start_playback(&ctx, false);
                    }
                }
                let paused = matches!(&self.mode, Mode::Playing { handle } if handle.is_user_paused());
                let r = widgets::big_action(
                    ui,
                    &p,
                    if paused { Icon::Play } else { Icon::Pause },
                    if paused { "Resume" } else { "Pause" },
                    &self.hotkey_text(HotkeyAction::Pause),
                    p.pause,
                    paused,
                    playing,
                );
                if r.clicked() {
                    if let Mode::Playing { handle } = &self.mode {
                        handle.toggle_pause();
                    }
                }
                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.set_min_height(112.0);
                    ui.set_min_width(240.0);
                    ui.set_max_width(ui.available_width().max(240.0));
                    self.live_status(ui);
                });
            });
        });
    }

    fn live_status(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        match &self.mode {
            Mode::Idle => {
                if let Some(last) = &self.last_run {
                    ui.label(RichText::new("Last run").size(12.0).color(p.text_faint));
                    ui.label(RichText::new(last).color(p.text));
                    ui.add_space(4.0);
                }
                match self.doc.as_ref() {
                    Some(d) => {
                        ui.label(RichText::new("Ready to play").size(17.0).strong().color(p.text));
                        let total = d
                            .mac
                            .playback
                            .estimated_total_us(d.mac.duration_us())
                            .map(format_duration_us)
                            .unwrap_or_else(|| "until stopped".into());
                        ui.label(
                            RichText::new(format!(
                                "{} · {} at {}× · total {}",
                                d.mac.meta.name,
                                d.mac.playback.repeat.describe(),
                                trim_speed(d.mac.playback.speed),
                                total
                            ))
                            .color(p.text_dim),
                        );
                    }
                    None => {
                        ui.label(RichText::new("Nothing recorded yet").size(17.0).strong().color(p.text));
                        ui.label(
                            RichText::new(format!(
                                "Press Record (or {}) and do what you want automated. Press it again to stop.",
                                self.hotkey_text(HotkeyAction::Record)
                            ))
                            .color(p.text_dim),
                        );
                    }
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new(format!(
                        "Emergency stop: {} — always works, releases every key.",
                        self.hotkey_text(HotkeyAction::EmergencyStop)
                    ))
                    .size(12.0)
                    .color(p.text_faint),
                );
            }
            Mode::RecordCountdown { start_at, .. } => {
                let s = start_at.saturating_duration_since(std::time::Instant::now()).as_secs() + 1;
                ui.label(
                    RichText::new(format!("Recording in {s}…"))
                        .size(26.0)
                        .strong()
                        .color(p.record),
                );
                ui.label(RichText::new("Switch to the window you want to automate.").color(p.text_dim));
            }
            Mode::Recording { session, .. } => {
                let t = crate::platform::now_us().saturating_sub(session.start_us());
                ui.horizontal(|ui| {
                    let pulse = (ui.input(|i| i.time) * 3.0).sin() as f32 * 0.5 + 0.5;
                    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), egui::Sense::hover());
                    ui.painter()
                        .circle_filled(r.center(), 6.0, p.record.gamma_multiply(0.5 + 0.5 * pulse));
                    ui.label(
                        RichText::new(format!("Recording  {}", format_clock_us(t)))
                            .size(22.0)
                            .strong()
                            .color(p.text),
                    );
                });
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 26.0;
                    widgets::stat(ui, &p, &session.clicks.to_string(), "clicks");
                    widgets::stat(ui, &p, &session.keys.to_string(), "key presses");
                    widgets::stat(ui, &p, &session.moves.to_string(), "mouse samples");
                });
                ui.label(
                    RichText::new("Clicks on SentinelTask itself are not recorded.")
                        .size(12.0)
                        .color(p.text_faint),
                );
            }
            Mode::Playing { handle } => {
                let s = handle.status();
                let (title, col) = match &s.state {
                    RunState::Paused(PauseReason::User) => ("Paused".to_string(), p.pause),
                    RunState::Paused(PauseReason::WaitingForWindow(w)) => (format!("Waiting for {w}"), p.pause),
                    RunState::Countdown { remaining_ms } => {
                        (format!("Starting in {}…", remaining_ms / 1000 + 1), p.play)
                    }
                    RunState::WaitingForModifiers => ("Release Ctrl / Alt / Shift…".to_string(), p.play),
                    _ => ("Playing".to_string(), p.play),
                };
                ui.label(RichText::new(title).size(22.0).strong().color(col));
                ui.add_space(4.0);
                let bar = egui::ProgressBar::new(s.pass_progress)
                    .desired_width(ui.available_width().min(380.0))
                    .fill(col);
                ui.add(bar);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 26.0;
                    let total = s
                        .total_passes
                        .map(|t| format!(" / {t}"))
                        .unwrap_or_else(|| " / forever".into());
                    widgets::stat(ui, &p, &format!("{}{total}", s.pass), "pass");
                    widgets::stat(ui, &p, &format_clock_us(s.elapsed_us), "elapsed");
                    if let Some(est) = s.estimated_total_us {
                        widgets::stat(ui, &p, &format_clock_us(est.saturating_sub(s.elapsed_us)), "remaining");
                    }
                });
                if s.total_passes != Some(s.pass)
                    && ui
                        .small_button("Stop after this loop")
                        .on_hover_text("Let the current pass finish, then stop")
                        .clicked()
                {
                    handle.finish_after_pass();
                }
                if let RunState::Paused(PauseReason::WaitingForWindow(w)) = &s.state {
                    ui.label(
                        RichText::new(format!("Focus guard: playback resumes when {w} is the active window."))
                            .size(12.0)
                            .color(p.text_faint),
                    );
                }
                if s.shifted_us > 0 {
                    ui.label(
                        RichText::new(format!(
                            "PC stalled for {} — timing re-synced.",
                            format_duration_us(s.shifted_us)
                        ))
                        .size(12.0)
                        .color(p.text_faint),
                    );
                }
            }
            Mode::Clicking { handle } => {
                ui.label(RichText::new("Auto-clicker running").size(20.0).strong().color(p.play));
                ui.label(
                    RichText::new(format!(
                        "{} clicks · {}",
                        handle.clicks(),
                        format_clock_us(handle.elapsed_us())
                    ))
                    .color(p.text_dim),
                );
            }
        }
    }

    fn current_macro_card(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let idle = self.mode.is_idle();
        let screen = self.screen;
        let last = self.last_foreign_window.clone();
        let mut act = DocActions::default();
        if self.doc.is_none() {
            let (mut open, mut lib) = (false, false);
            widgets::card(ui, &p, |ui| {
                widgets::card_title(
                    ui,
                    &p,
                    "No macro loaded",
                    Some("Record something, or open a saved macro or a TinyTask .rec file."),
                );
                ui.horizontal(|ui| {
                    open = widgets::tool_button(ui, &p, Icon::Folder, "Open…", idle).clicked();
                    lib = widgets::tool_button(ui, &p, Icon::Library, "Library", true).clicked();
                });
            });
            if open {
                self.open_dialog();
            }
            if lib {
                self.page = Page::Library;
            }
            return;
        }
        let doc = self.doc.as_mut().expect("checked above");

        widgets::card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                let r = ui.add_enabled(
                    idle,
                    egui::TextEdit::singleline(&mut doc.mac.meta.name)
                        .font(egui::FontId::proportional(17.0))
                        .desired_width((ui.available_width() - 110.0).clamp(160.0, 420.0))
                        .hint_text("Macro name"),
                );
                if r.changed() {
                    doc.dirty = true;
                }
                if doc.dirty {
                    widgets::pill(ui, "Unsaved", p.warn_fg, p.warn_bg);
                } else if doc.path.is_some() {
                    widgets::pill(ui, "Saved", p.play, p.ok_bg);
                }
            });
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                if widgets::tool_button(ui, &p, Icon::Save, "Save", idle && doc.dirty).on_hover_text("Ctrl+S").clicked() {
                    act.save = true;
                }
                if widgets::tool_button(ui, &p, Icon::Save, "Save as…", idle).clicked() {
                    act.save_as = true;
                }
                if widgets::tool_button(ui, &p, Icon::Edit, "Edit events", idle).clicked() {
                    act.edit = true;
                }
                if widgets::tool_button(ui, &p, Icon::Import, "Export .exe", idle)
                    .on_hover_text("Create a standalone player that runs this macro without installing anything (like TinyTask's Compile EXE)")
                    .clicked()
                {
                    act.export = true;
                }
            });
            ui.add_space(6.0);
            let st = doc.mac.stats();
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 30.0;
                widgets::stat(ui, &p, &format_duration_us(st.duration_us), "length");
                widgets::stat(ui, &p, &st.clicks.to_string(), "clicks");
                widgets::stat(ui, &p, &st.key_presses.to_string(), "key presses");
                widgets::stat(ui, &p, &(st.moves + st.deltas).to_string(), "mouse samples");
                widgets::stat(ui, &p, &st.events.to_string(), "events");
            });
            let mut info = Vec::new();
            if let Some(s) = doc.mac.meta.screen {
                info.push(format!(
                    "Recorded on {}×{} at {}% scale",
                    s.desktop.w,
                    s.desktop.h,
                    s.dpi * 100 / 96
                ));
            }
            if let Some(t) = &doc.mac.meta.target {
                info.push(format!("in {}", t.process));
            }
            if let Some(path) = &doc.path {
                info.push(path.display().to_string());
            } else if !doc.mac.meta.source.is_empty() {
                info.push(doc.mac.meta.source.clone());
            }
            if !info.is_empty() {
                ui.label(RichText::new(info.join("  ·  ")).size(12.0).color(p.text_faint));
            }
        });
        ui.add_space(12.0);

        widgets::card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                widgets::card_title(ui, &p, "Playback options", Some("Saved with this macro."));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add_enabled(idle, egui::Button::new("Use for new recordings"))
                        .on_hover_text("Make these the defaults for future recordings")
                        .clicked()
                    {
                        act.set_defaults = true;
                    }
                });
            });
            ui.add_enabled_ui(idle, |ui| {
                if playback_options_ui(
                    ui,
                    &p,
                    &mut doc.mac.playback,
                    &mut doc.mac.meta.target,
                    doc.mac.meta.screen,
                    screen,
                    last.as_ref(),
                ) {
                    doc.dirty = true;
                }
            });
        });

        let defaults = doc.mac.playback;
        if act.save {
            self.save();
        }
        if act.save_as {
            self.save_as();
        }
        if act.edit {
            self.page = Page::Editor;
        }
        if act.export {
            self.export_player();
        }
        if act.set_defaults {
            self.settings.playback_defaults = PlaybackOptions {
                focus_guard: false,
                scale_to_screen: false,
                ..defaults
            };
            self.mark_settings_dirty();
            self.toast(super::app::ToastKind::Ok, "New recordings will use these options.");
        }
    }
}

fn trim_speed(s: f64) -> String {
    let t = format!("{s:.2}");
    t.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The playback options form. Returns true if anything changed.
#[allow(clippy::too_many_arguments)]
pub fn playback_options_ui(
    ui: &mut egui::Ui,
    p: &Palette,
    o: &mut PlaybackOptions,
    target: &mut Option<TargetWindow>,
    recorded: Option<ScreenInfo>,
    current: ScreenInfo,
    last_window: Option<&WindowInfo>,
) -> bool {
    let mut changed = false;

    widgets::form_row(ui, p, "Speed", |ui| {
        // Logarithmic slider so 0.5× and 50× are both easy to hit.
        let mut s = o.speed.clamp(MIN_SPEED, MAX_SPEED);
        let r = ui.add(
            egui::Slider::new(&mut s, MIN_SPEED..=MAX_SPEED)
                .logarithmic(true)
                .suffix("×")
                .max_decimals(2)
                .custom_formatter(|v, _| trim_speed(v)),
        );
        if r.changed() {
            o.speed = s;
            changed = true;
        }
        ui.add_space(6.0);
        for preset in [0.5, 1.0, 2.0, 5.0, 10.0] {
            let sel = (o.speed - preset).abs() < 1e-6;
            if ui.selectable_label(sel, format!("{}×", trim_speed(preset))).clicked() {
                o.speed = preset;
                changed = true;
            }
        }
    });

    #[derive(Clone, Copy, PartialEq)]
    enum R {
        Once,
        Times,
        Forever,
        Duration,
    }
    let mut kind = match o.repeat {
        Repeat::Times { count: 1 } => R::Once,
        Repeat::Times { .. } => R::Times,
        Repeat::Forever => R::Forever,
        Repeat::Duration { .. } => R::Duration,
    };
    widgets::form_row(ui, p, "Repeat", |ui| {
        if widgets::segmented(
            ui,
            p,
            &mut kind,
            &[
                (R::Once, "Once"),
                (R::Times, "Times"),
                (R::Forever, "Forever"),
                (R::Duration, "For a time"),
            ],
        ) {
            o.repeat = match kind {
                R::Once => Repeat::Times { count: 1 },
                R::Times => Repeat::Times { count: 10 },
                R::Forever => Repeat::Forever,
                R::Duration => Repeat::Duration { secs: 3600 },
            };
            changed = true;
        }
        match &mut o.repeat {
            Repeat::Times { count } if kind == R::Times => {
                changed |= ui
                    .add(egui::DragValue::new(count).range(2..=10_000_000).suffix(" times"))
                    .changed();
            }
            Repeat::Duration { secs } => {
                let mut h = *secs / 3600;
                let mut m = (*secs % 3600) / 60;
                let c1 = ui
                    .add(egui::DragValue::new(&mut h).range(0..=999).suffix(" h"))
                    .changed();
                let c2 = ui
                    .add(egui::DragValue::new(&mut m).range(0..=59).suffix(" min"))
                    .changed();
                if c1 || c2 {
                    *secs = (h * 3600 + m * 60).max(60);
                    changed = true;
                }
            }
            _ => {}
        }
    });
    if matches!(o.repeat, Repeat::Duration { .. }) {
        hint(
            ui,
            p,
            "The loop that is running when time is up gets to finish, so nothing stops half-way.",
        );
    }

    if !matches!(o.repeat, Repeat::Times { count: 1 }) {
        widgets::form_row(ui, p, "Pause between", |ui| {
            changed |= ui
                .add(
                    egui::DragValue::new(&mut o.loop_delay_ms)
                        .range(0..=3_600_000)
                        .speed(10.0)
                        .suffix(" ms"),
                )
                .changed();
            ui.label(RichText::new("between loops").color(p.text_faint));
        });
    }

    widgets::form_row(ui, p, "Mouse", |ui| {
        changed |= widgets::segmented(
            ui,
            p,
            &mut o.mouse_mode,
            &[
                (MouseMode::Absolute, "Exact positions"),
                (MouseMode::Relative, "Relative (3D games)"),
            ],
        );
    });
    hint(
        ui,
        p,
        match o.mouse_mode {
            MouseMode::Absolute => {
                "Moves the cursor to the exact recorded spots. Best for apps, browsers and 2D games."
            }
            MouseMode::Relative => "Replays raw mouse movement. Use for games that lock the cursor to turn the camera.",
        },
    );
    ui.add_space(4.0);

    let mismatch = recorded.is_some_and(|r| r.desktop != current.desktop);
    if mismatch || o.scale_to_screen {
        let r = recorded.unwrap_or(current);
        changed |= widgets::toggle_row(
            ui,
            p,
            &mut o.scale_to_screen,
            "Fit to current screen",
            &format!(
                "Recorded on {}×{}, this screen is {}×{}. Scales click positions to match.",
                r.desktop.w, r.desktop.h, current.desktop.w, current.desktop.h
            ),
        );
    }

    let target_name = target.as_ref().map(|t| t.process.clone()).unwrap_or_default();
    let desc = if target_name.is_empty() {
        "Pause automatically whenever the target window isn't in front, and resume when it is.".to_string()
    } else {
        format!("Pause automatically while {target_name} isn't the active window, then resume.")
    };
    changed |= widgets::toggle_row(ui, p, &mut o.focus_guard, "Only play into the target window", &desc);
    if o.focus_guard {
        ui.horizontal(|ui| {
            ui.add_space(4.0);
            ui.label(RichText::new("Target program:").color(p.text_dim));
            let mut name = target_name.clone();
            if ui
                .add(
                    egui::TextEdit::singleline(&mut name)
                        .desired_width(200.0)
                        .hint_text("e.g. notepad.exe"),
                )
                .changed()
            {
                *target = if name.trim().is_empty() {
                    None
                } else {
                    Some(TargetWindow {
                        process: name.trim().to_string(),
                        title: String::new(),
                    })
                };
                changed = true;
            }
            if let Some(w) = last_window {
                if !w.process.is_empty()
                    && !w.matches_process(&target_name)
                    && ui.button(format!("Use {}", w.process)).clicked()
                {
                    *target = Some(TargetWindow {
                        process: w.process.clone(),
                        title: w.title.clone(),
                    });
                    changed = true;
                }
            }
        });
    }
    changed |= widgets::toggle_row(
        ui,
        p,
        &mut o.clicks_only,
        "Clicks only",
        "Skip recorded mouse movement and just click at each spot. Faster and immune to motion drift.",
    );
    changed
}

fn hint(ui: &mut egui::Ui, p: &Palette, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(138.0);
        ui.label(RichText::new(text).size(12.0).color(p.text_faint));
    });
}
