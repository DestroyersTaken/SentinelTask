//! The macro editor (RESEARCH §1 #11, #16): timeline, event table,
//! per-event editing, clean-up tools and undo/redo.

use std::collections::BTreeSet;

use eframe::egui::{self, pos2, vec2, Color32, CornerRadius, RichText, Sense, Stroke, StrokeKind};
use egui_extras::{Column, TableBuilder};

use sentinel_core::edit::{self, KindFilter};
use sentinel_core::keys::vk_name;
use sentinel_core::util::format_duration_us;
use sentinel_core::{Event, EventKind};

use super::app::{SentinelApp, ToastKind};
use super::theme::Palette;
use super::widgets::{self, Icon};

const MAX_UNDO: usize = 50;

pub struct EditorState {
    pub selected: BTreeSet<usize>,
    anchor: Option<usize>,
    undo: Vec<Vec<Event>>,
    redo: Vec<Vec<Event>>,
    trim_keep_ms: u64,
    cap_secs: f64,
    speed_factor: f64,
    offset: (i32, i32),
    insert_ms: u64,
    scroll_to: Option<usize>,
    hide_motion: bool,
}

impl Default for EditorState {
    fn default() -> Self {
        Self {
            selected: BTreeSet::new(),
            anchor: None,
            undo: Vec::new(),
            redo: Vec::new(),
            trim_keep_ms: 300,
            cap_secs: 2.0,
            speed_factor: 2.0,
            offset: (0, 0),
            insert_ms: 500,
            scroll_to: None,
            hide_motion: false,
        }
    }
}

fn kind_label(k: &EventKind) -> &'static str {
    match k {
        EventKind::MouseMove { .. } => "Mouse move",
        EventKind::MouseDelta { .. } => "Raw motion",
        EventKind::MouseButton { down: true, .. } => "Mouse down",
        EventKind::MouseButton { down: false, .. } => "Mouse up",
        EventKind::Wheel { .. } => "Wheel",
        EventKind::Key { down: true, .. } => "Key down",
        EventKind::Key { down: false, .. } => "Key up",
    }
}

fn kind_color(p: &Palette, k: &EventKind) -> Color32 {
    match k {
        EventKind::MouseMove { .. } | EventKind::MouseDelta { .. } => p.text_faint,
        EventKind::MouseButton { .. } => p.accent,
        EventKind::Wheel { .. } => p.pause,
        EventKind::Key { .. } => Color32::from_rgb(0xB0, 0x7C, 0xFF),
    }
}

fn details(k: &EventKind) -> String {
    match *k {
        EventKind::MouseMove { x, y } => format!("({x}, {y})"),
        EventKind::MouseDelta { dx, dy } => format!("Δ {dx:+}, {dy:+}"),
        EventKind::MouseButton { button, x, y, .. } => format!("{} at ({x}, {y})", button.label()),
        EventKind::Wheel { delta, horizontal, .. } => {
            let notches = delta as f32 / 120.0;
            format!(
                "{} {notches:+.1} notch{}",
                if horizontal { "Horizontal" } else { "Vertical" },
                if notches.abs() == 1.0 { "" } else { "es" }
            )
        }
        EventKind::Key { key, .. } => {
            let mut s = vk_name(key.vk);
            if key.extended && matches!(key.vk, 0x0D) {
                s = "Num Enter".into();
            }
            s
        }
    }
}

impl SentinelApp {
    fn snapshot(&mut self) {
        if let Some(d) = self.doc.as_ref() {
            self.editor.undo.push(d.mac.events.clone());
            if self.editor.undo.len() > MAX_UNDO {
                self.editor.undo.remove(0);
            }
            self.editor.redo.clear();
        }
    }

    fn commit(&mut self) {
        if let Some(d) = self.doc.as_mut() {
            d.dirty = true;
            let n = d.mac.events.len();
            self.editor.selected.retain(|&i| i < n);
        }
    }

    pub fn editor_undo(&mut self) {
        if let (Some(prev), Some(d)) = (self.editor.undo.pop(), self.doc.as_mut()) {
            self.editor.redo.push(std::mem::replace(&mut d.mac.events, prev));
            d.dirty = true;
            self.editor.selected.clear();
        }
    }

    pub fn editor_redo(&mut self) {
        if let (Some(next), Some(d)) = (self.editor.redo.pop(), self.doc.as_mut()) {
            self.editor.undo.push(std::mem::replace(&mut d.mac.events, next));
            d.dirty = true;
            self.editor.selected.clear();
        }
    }

    fn apply_edit(&mut self, f: impl FnOnce(&mut Vec<Event>) -> String) {
        self.snapshot();
        let msg = match self.doc.as_mut() {
            Some(d) => f(&mut d.mac.events),
            None => return,
        };
        self.commit();
        if !msg.is_empty() {
            self.toast(ToastKind::Ok, msg);
        }
    }

    pub fn page_editor(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        widgets::page_header(
            ui,
            &p,
            "Editor",
            "Fine-tune a macro: trim idle time, fix delays, remove or adjust individual events.",
        );
        if self.doc.is_none() {
            widgets::card(ui, &p, |ui| {
                widgets::card_title(
                    ui,
                    &p,
                    "Nothing to edit",
                    Some("Record a macro or open one from the Library first."),
                );
            });
            return;
        }
        if !self.mode.is_idle() {
            widgets::banner(
                ui,
                &p,
                widgets::BannerKind::Info,
                "Busy",
                "Editing is disabled while recording or playing.",
                &[],
            );
            return;
        }

        self.editor_toolbar(ui);
        ui.add_space(10.0);
        self.editor_tools(ui);
        ui.add_space(10.0);
        self.editor_timeline(ui);
        ui.add_space(8.0);
        self.editor_selected_panel(ui);
        self.editor_table(ui);
    }

    fn editor_toolbar(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let n_sel = self.editor.selected.len();
        let (undo, redo) = (!self.editor.undo.is_empty(), !self.editor.redo.is_empty());
        let mut do_undo = false;
        let mut do_redo = false;
        let mut do_delete = false;
        let mut do_insert = false;
        let mut select_all = false;
        let mut clear = false;
        ui.horizontal_wrapped(|ui| {
            do_undo = widgets::tool_button(ui, &p, Icon::Undo, "Undo", undo)
                .on_hover_text("Ctrl+Z")
                .clicked();
            do_redo = widgets::tool_button(ui, &p, Icon::Redo, "Redo", redo)
                .on_hover_text("Ctrl+Y")
                .clicked();
            ui.separator();
            do_delete = widgets::tool_button(ui, &p, Icon::Trash, &format!("Delete ({n_sel})"), n_sel > 0)
                .on_hover_text("Delete the selected events (Del). Later events move earlier to close the gap.")
                .clicked();
            do_insert = widgets::tool_button(ui, &p, Icon::Plus, "Insert pause", n_sel > 0)
                .on_hover_text("Insert a pause before the first selected event")
                .clicked();
            ui.add(
                egui::DragValue::new(&mut self.editor.insert_ms)
                    .range(1..=3_600_000)
                    .speed(10.0)
                    .suffix(" ms"),
            );
            ui.separator();
            select_all = ui.button("Select all").clicked();
            clear = ui.add_enabled(n_sel > 0, egui::Button::new("Select none")).clicked();
            ui.checkbox(&mut self.editor.hide_motion, "Hide movement");
        });
        let del_key = ui.input(|i| i.key_pressed(egui::Key::Delete)) && !ui.ctx().egui_wants_keyboard_input();
        if do_undo {
            self.editor_undo();
        }
        if do_redo {
            self.editor_redo();
        }
        if (do_delete || del_key) && n_sel > 0 {
            let sel: Vec<usize> = self.editor.selected.iter().copied().collect();
            self.apply_edit(|e| {
                edit::delete_events(e, &sel, false);
                format!("Deleted {} event{}", sel.len(), if sel.len() == 1 { "" } else { "s" })
            });
            self.editor.selected.clear();
        }
        if do_insert {
            if let Some(&first) = self.editor.selected.iter().next() {
                let us = self.editor.insert_ms * 1000;
                self.apply_edit(|e| {
                    edit::insert_wait(e, first, us);
                    format!("Inserted a {} pause", format_duration_us(us))
                });
            }
        }
        if select_all {
            let n = self.doc.as_ref().map(|d| d.mac.events.len()).unwrap_or(0);
            self.editor.selected = (0..n).collect();
        }
        if clear {
            self.editor.selected.clear();
        }
    }

    fn editor_tools(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let recorded = self.doc.as_ref().and_then(|d| d.mac.meta.screen);
        let current = self.screen;
        let mut action: Option<u8> = None;
        egui::CollapsingHeader::new(RichText::new("Clean-up tools").strong().color(p.text))
            .default_open(true)
            .show(ui, |ui| {
                widgets::card(ui, &p, |ui| {
                    egui::Grid::new("tools")
                        .num_columns(3)
                        .spacing(vec2(14.0, 10.0))
                        .show(ui, |ui| {
                            ui.label(RichText::new("Trim start").color(p.text_dim));
                            ui.horizontal(|ui| {
                                ui.label("Keep");
                                ui.add(
                                    egui::DragValue::new(&mut self.editor.trim_keep_ms)
                                        .range(0..=60_000)
                                        .speed(10.0)
                                        .suffix(" ms"),
                                );
                                ui.label("before the first event");
                            });
                            if ui.button("Apply").clicked() {
                                action = Some(1);
                            }
                            ui.end_row();

                            ui.label(RichText::new("Shorten long pauses").color(p.text_dim));
                            ui.horizontal(|ui| {
                                ui.label("Cap every pause at");
                                ui.add(
                                    egui::DragValue::new(&mut self.editor.cap_secs)
                                        .range(0.05..=3600.0)
                                        .speed(0.05)
                                        .suffix(" s"),
                                );
                            });
                            if ui.button("Apply").clicked() {
                                action = Some(2);
                            }
                            ui.end_row();

                            ui.label(RichText::new("Change speed").color(p.text_dim));
                            ui.horizontal(|ui| {
                                ui.label("Make it");
                                ui.add(
                                    egui::DragValue::new(&mut self.editor.speed_factor)
                                        .range(0.1..=100.0)
                                        .speed(0.05)
                                        .suffix("× faster"),
                                );
                            });
                            if ui.button("Apply").clicked() {
                                action = Some(3);
                            }
                            ui.end_row();

                            ui.label(RichText::new("Shift positions").color(p.text_dim));
                            ui.horizontal(|ui| {
                                ui.label("Move");
                                ui.add(
                                    egui::DragValue::new(&mut self.editor.offset.0)
                                        .range(-20_000..=20_000)
                                        .prefix("x "),
                                );
                                ui.add(
                                    egui::DragValue::new(&mut self.editor.offset.1)
                                        .range(-20_000..=20_000)
                                        .prefix("y "),
                                );
                                ui.label(if self.editor.selected.is_empty() {
                                    "(all)"
                                } else {
                                    "(selected)"
                                });
                            });
                            if ui.button("Apply").clicked() {
                                action = Some(4);
                            }
                            ui.end_row();

                            ui.label(RichText::new("Remove").color(p.text_dim));
                            ui.horizontal(|ui| {
                                if ui
                                    .button("Mouse movement")
                                    .on_hover_text("Keep clicks; drop all cursor moves")
                                    .clicked()
                                {
                                    action = Some(5);
                                }
                                if ui.button("Raw motion").on_hover_text("Drop 3D-camera motion").clicked() {
                                    action = Some(6);
                                }
                                if ui.button("Keyboard").clicked() {
                                    action = Some(7);
                                }
                                if ui.button("Wheel").clicked() {
                                    action = Some(8);
                                }
                            });
                            ui.label("");
                            ui.end_row();

                            if let Some(r) = recorded {
                                if r.desktop != current.desktop {
                                    ui.label(RichText::new("Rescale").color(p.text_dim));
                                    ui.label(format!(
                                        "from {}×{} to {}×{} (this screen)",
                                        r.desktop.w, r.desktop.h, current.desktop.w, current.desktop.h
                                    ));
                                    if ui.button("Apply").clicked() {
                                        action = Some(9);
                                    }
                                    ui.end_row();
                                }
                            }
                        });
                });
            });
        let Some(a) = action else { return };
        let keep = self.editor.trim_keep_ms * 1000;
        let cap = (self.editor.cap_secs * 1_000_000.0) as u64;
        let factor = self.editor.speed_factor;
        let (dx, dy) = self.editor.offset;
        let sel: Vec<usize> = self.editor.selected.iter().copied().collect();
        match a {
            1 => self.apply_edit(|e| {
                edit::trim_leading_idle(e, keep);
                "Trimmed the start".into()
            }),
            2 => self.apply_edit(|e| {
                let removed = edit::cap_pauses(e, cap);
                format!("Removed {} of waiting", format_duration_us(removed))
            }),
            3 => self.apply_edit(|e| {
                edit::scale_time(e, 1.0 / factor);
                format!("Macro is now {factor}× faster")
            }),
            4 => self.apply_edit(|e| {
                edit::offset_positions(e, dx, dy, if sel.is_empty() { None } else { Some(&sel) });
                format!("Moved positions by ({dx}, {dy})")
            }),
            5..=8 => {
                let filter = KindFilter {
                    moves: a == 5,
                    deltas: a == 6,
                    keys: a == 7,
                    wheel: a == 8,
                    clicks: false,
                };
                self.apply_edit(|e| format!("Removed {} events", edit::remove_kinds(e, filter)));
                self.editor.selected.clear();
            }
            9 => {
                if let Some(r) = recorded {
                    self.apply_edit(|e| {
                        edit::rescale_positions(e, r.desktop, current.desktop);
                        "Positions rescaled to this screen".into()
                    });
                    if let Some(d) = self.doc.as_mut() {
                        d.mac.meta.screen = Some(current);
                        d.mac.playback.scale_to_screen = false;
                    }
                }
            }
            _ => {}
        }
    }

    fn editor_timeline(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(doc) = self.doc.as_ref() else { return };
        let events = &doc.mac.events;
        let dur = doc.mac.duration_us().max(1) as f32;
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
        let painter = ui.painter_at(rect);
        painter.rect(
            rect,
            CornerRadius::same(8),
            p.card,
            Stroke::new(1.0, p.border),
            StrokeKind::Inside,
        );
        let inner = rect.shrink2(vec2(10.0, 8.0));
        // Bucket events into columns so long macros stay fast to draw.
        let cols = inner.width().max(1.0) as usize;
        let mut motion = vec![0u32; cols];
        let mut marks: Vec<(usize, Color32, bool)> = Vec::new();
        for (i, e) in events.iter().enumerate() {
            let c = ((e.t_us as f32 / dur) * (cols - 1) as f32) as usize;
            if e.kind.is_mouse_motion() {
                motion[c.min(cols - 1)] += 1;
            } else if matches!(
                e.kind,
                EventKind::MouseButton { down: true, .. } | EventKind::Key { down: true, .. } | EventKind::Wheel { .. }
            ) {
                marks.push((
                    c.min(cols - 1),
                    kind_color(&p, &e.kind),
                    self.editor.selected.contains(&i),
                ));
            }
        }
        let max_m = motion.iter().copied().max().unwrap_or(1).max(1) as f32;
        for (c, &m) in motion.iter().enumerate() {
            if m > 0 {
                let h = 4.0 + 10.0 * (m as f32 / max_m);
                let x = inner.left() + c as f32;
                painter.line_segment(
                    [pos2(x, inner.bottom()), pos2(x, inner.bottom() - h)],
                    Stroke::new(1.0, p.text_faint.gamma_multiply(0.5)),
                );
            }
        }
        for (c, col, sel) in marks {
            let x = inner.left() + c as f32;
            let top = if sel { inner.top() } else { inner.top() + 8.0 };
            painter.line_segment(
                [pos2(x, top), pos2(x, inner.bottom() - 16.0)],
                Stroke::new(if sel { 2.5 } else { 1.5 }, col),
            );
        }
        painter.text(
            pos2(inner.right(), inner.top()),
            egui::Align2::RIGHT_TOP,
            format_duration_us(doc.mac.duration_us()),
            egui::FontId::proportional(11.0),
            p.text_faint,
        );
        if resp.clicked() {
            if let Some(pos) = resp.interact_pointer_pos() {
                let t = ((pos.x - inner.left()) / inner.width()).clamp(0.0, 1.0) * dur;
                let idx = events
                    .partition_point(|e| (e.t_us as f32) < t)
                    .min(events.len().saturating_sub(1));
                self.editor.selected = [idx].into_iter().collect();
                self.editor.anchor = Some(idx);
                self.editor.scroll_to = Some(idx);
            }
        }
        resp.on_hover_text("Click to jump to that moment");
    }

    fn editor_selected_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        if self.editor.selected.len() != 1 {
            return;
        }
        let idx = *self.editor.selected.iter().next().unwrap();
        let Some(doc) = self.doc.as_ref() else { return };
        if idx >= doc.mac.events.len() {
            return;
        }
        let mut ev = doc.mac.events[idx];
        let mut gap_ms = edit::gap_before(&doc.mac.events, idx) as f64 / 1000.0;
        let orig_gap = gap_ms;
        let mut changed = false;
        widgets::card(ui, &p, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(format!("Event #{} · {}", idx + 1, kind_label(&ev.kind)))
                        .strong()
                        .color(kind_color(&p, &ev.kind)),
                );
                ui.separator();
                ui.label("Delay before:");
                ui.add(
                    egui::DragValue::new(&mut gap_ms)
                        .range(0.0..=3_600_000.0)
                        .speed(1.0)
                        .suffix(" ms")
                        .max_decimals(1),
                );
                match &mut ev.kind {
                    EventKind::MouseMove { x, y }
                    | EventKind::MouseButton { x, y, .. }
                    | EventKind::Wheel { x, y, .. } => {
                        ui.separator();
                        ui.label("Position:");
                        changed |= ui.add(egui::DragValue::new(x).prefix("x ")).changed();
                        changed |= ui.add(egui::DragValue::new(y).prefix("y ")).changed();
                        if let EventKind::Wheel { delta, .. } = &mut ev.kind {
                            changed |= ui.add(egui::DragValue::new(delta).speed(10).prefix("Δ ")).changed();
                        }
                    }
                    EventKind::MouseDelta { dx, dy } => {
                        ui.separator();
                        changed |= ui.add(egui::DragValue::new(dx).prefix("dx ")).changed();
                        changed |= ui.add(egui::DragValue::new(dy).prefix("dy ")).changed();
                    }
                    EventKind::Key { key, .. } => {
                        ui.separator();
                        ui.label(
                            RichText::new(format!(
                                "{} (scan 0x{:02X}{})",
                                vk_name(key.vk),
                                key.scan,
                                if key.extended { ", extended" } else { "" }
                            ))
                            .color(p.text_dim),
                        );
                    }
                }
            });
        });
        ui.add_space(8.0);
        let gap_changed = (gap_ms - orig_gap).abs() > 1e-9;
        if changed || gap_changed {
            // Coalesce rapid drags into one undo step.
            let id = egui::Id::unique(("ed-drag", idx));
            let dragging = ui.ctx().memory(|m| m.data.get_temp::<bool>(id)).unwrap_or(false);
            if !dragging {
                self.snapshot();
                ui.ctx().memory_mut(|m| m.data.insert_temp(id, true));
            }
            if let Some(d) = self.doc.as_mut() {
                if changed {
                    d.mac.events[idx].kind = ev.kind;
                }
                if gap_changed {
                    edit::set_gap_before(&mut d.mac.events, idx, (gap_ms * 1000.0).round() as u64);
                }
                d.dirty = true;
            }
        } else if !ui.input(|i| i.pointer.any_down()) {
            ui.ctx()
                .memory_mut(|m| m.data.remove::<bool>(egui::Id::unique(("ed-drag", idx))));
        }
    }

    fn editor_table(&mut self, ui: &mut egui::Ui) {
        let p = self.palette;
        let Some(doc) = self.doc.as_ref() else { return };
        let events = &doc.mac.events;
        let rows: Vec<usize> = if self.editor.hide_motion {
            (0..events.len())
                .filter(|&i| !events[i].kind.is_mouse_motion())
                .collect()
        } else {
            (0..events.len()).collect()
        };
        let mut clicked: Option<(usize, egui::Modifiers)> = None;
        let scroll_row = self
            .editor
            .scroll_to
            .take()
            .and_then(|i| rows.iter().position(|&r| r == i));
        widgets::card(ui, &p, |ui| {
            let mut table = TableBuilder::new(ui)
                .id_salt("events")
                .striped(true)
                .sense(Sense::click())
                .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
                .column(Column::exact(64.0))
                .column(Column::exact(96.0))
                .column(Column::exact(90.0))
                .column(Column::exact(110.0))
                .column(Column::remainder())
                .min_scrolled_height(260.0)
                .max_scroll_height(460.0);
            if let Some(r) = scroll_row {
                table = table.scroll_to_row(r, Some(egui::Align::Center));
            }
            table
                .header(24.0, |mut h| {
                    for t in ["#", "Time", "Delay", "Type", "Details"] {
                        h.col(|ui| {
                            ui.label(RichText::new(t).size(12.0).strong().color(p.text_faint));
                        });
                    }
                })
                .body(|body| {
                    body.rows(24.0, rows.len(), |mut row| {
                        let i = rows[row.index()];
                        let e = &events[i];
                        row.set_selected(self.editor.selected.contains(&i));
                        row.col(|ui| {
                            ui.label(RichText::new((i + 1).to_string()).size(12.0).color(p.text_faint));
                        });
                        row.col(|ui| {
                            ui.label(
                                RichText::new(format!("{:.3}s", e.t_us as f64 / 1e6))
                                    .monospace()
                                    .size(12.0),
                            );
                        });
                        row.col(|ui| {
                            let gap = edit::gap_before(events, i);
                            ui.label(
                                RichText::new(format!("+{}", format_duration_us(gap)))
                                    .size(12.0)
                                    .color(p.text_dim),
                            );
                        });
                        row.col(|ui| {
                            let c = kind_color(&p, &e.kind);
                            let (r, _) = ui.allocate_exact_size(vec2(8.0, 8.0), Sense::hover());
                            ui.painter().circle_filled(r.center(), 3.5, c);
                            ui.label(RichText::new(kind_label(&e.kind)).size(12.5));
                        });
                        row.col(|ui| {
                            ui.label(RichText::new(details(&e.kind)).size(12.5).color(p.text_dim));
                        });
                        if row.response().clicked() {
                            clicked = Some((i, ui_mods(&row.response())));
                        }
                    });
                });
        });
        if let Some((i, mods)) = clicked {
            let sel = &mut self.editor.selected;
            if mods.shift {
                let a = self.editor.anchor.unwrap_or(i);
                let (lo, hi) = (a.min(i), a.max(i));
                if !mods.command {
                    sel.clear();
                }
                sel.extend(rows.iter().copied().filter(|&r| r >= lo && r <= hi));
            } else if mods.command {
                if !sel.remove(&i) {
                    sel.insert(i);
                }
                self.editor.anchor = Some(i);
            } else {
                sel.clear();
                sel.insert(i);
                self.editor.anchor = Some(i);
            }
        }
    }
}

fn ui_mods(r: &egui::Response) -> egui::Modifiers {
    r.ctx.input(|i| i.modifiers)
}
