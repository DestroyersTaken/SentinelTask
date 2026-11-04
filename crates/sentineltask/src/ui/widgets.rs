//! Reusable, custom-painted widgets for a polished look.

use eframe::egui::{
    self, pos2, vec2, Align2, Color32, CornerRadius, FontId, Margin, Rect, Response, RichText, Sense, Shape, Stroke,
    StrokeKind, Ui, Vec2,
};

use super::theme::{Palette, RADIUS, RADIUS_SMALL};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    Record,
    Play,
    Stop,
    Pause,
    Library,
    Edit,
    Clicker,
    Settings,
    Info,
    Folder,
    Save,
    Plus,
    Trash,
    Shield,
    Compact,
    Expand,
    Import,
    Undo,
    Redo,
}

/// Paint a simple vector icon into `rect`.
pub fn paint_icon(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let c = rect.center();
    let s = rect.width().min(rect.height());
    let stroke = Stroke::new((s * 0.11).max(1.4), color);
    match icon {
        Icon::Record => {
            painter.circle_filled(c, s * 0.36, color);
        }
        Icon::Play => {
            let h = s * 0.40;
            let pts = vec![
                pos2(c.x - h * 0.75, c.y - h),
                pos2(c.x + h * 1.0, c.y),
                pos2(c.x - h * 0.75, c.y + h),
            ];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Stop => {
            let r = Rect::from_center_size(c, Vec2::splat(s * 0.62));
            painter.rect_filled(r, CornerRadius::same((s * 0.1) as u8), color);
        }
        Icon::Pause => {
            let w = s * 0.18;
            let h = s * 0.66;
            for dx in [-s * 0.15, s * 0.15] {
                let r = Rect::from_center_size(pos2(c.x + dx, c.y), vec2(w, h));
                painter.rect_filled(r, CornerRadius::same(2), color);
            }
        }
        Icon::Library => {
            for (i, dy) in [-0.28f32, 0.0, 0.28].iter().enumerate() {
                let y = c.y + dy * s;
                painter.circle_filled(pos2(c.x - s * 0.32, y), s * 0.06, color);
                let len = if i == 2 { 0.4 } else { 0.55 };
                painter.line_segment([pos2(c.x - s * 0.18, y), pos2(c.x - s * 0.18 + s * len, y)], stroke);
            }
        }
        Icon::Edit => {
            let a = pos2(c.x - s * 0.30, c.y + s * 0.30);
            let b = pos2(c.x + s * 0.24, c.y - s * 0.24);
            painter.line_segment([a, b], Stroke::new(stroke.width * 1.6, color));
            painter.line_segment(
                [
                    pos2(c.x - s * 0.36, c.y + s * 0.36),
                    pos2(c.x + s * 0.0, c.y + s * 0.36),
                ],
                stroke,
            );
        }
        Icon::Clicker => {
            let r = Rect::from_center_size(c, vec2(s * 0.48, s * 0.70));
            painter.rect_stroke(r, CornerRadius::same((s * 0.24) as u8), stroke, StrokeKind::Middle);
            painter.line_segment([pos2(c.x, r.top()), pos2(c.x, r.top() + s * 0.24)], stroke);
            painter.line_segment(
                [pos2(r.left(), r.top() + s * 0.24), pos2(r.right(), r.top() + s * 0.24)],
                stroke,
            );
        }
        Icon::Settings => {
            painter.circle_stroke(c, s * 0.18, stroke);
            for i in 0..8 {
                let a = i as f32 * std::f32::consts::TAU / 8.0;
                let d = vec2(a.cos(), a.sin());
                painter.line_segment(
                    [c + d * s * 0.27, c + d * s * 0.38],
                    Stroke::new(stroke.width * 1.3, color),
                );
            }
        }
        Icon::Info => {
            painter.circle_stroke(c, s * 0.36, stroke);
            painter.circle_filled(pos2(c.x, c.y - s * 0.16), s * 0.05, color);
            painter.line_segment([pos2(c.x, c.y - s * 0.04), pos2(c.x, c.y + s * 0.20)], stroke);
        }
        Icon::Folder => {
            let r = Rect::from_center_size(pos2(c.x, c.y + s * 0.04), vec2(s * 0.74, s * 0.52));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.line_segment(
                [pos2(r.left(), r.top()), pos2(r.left() + s * 0.26, r.top() - s * 0.1)],
                stroke,
            );
            painter.line_segment(
                [
                    pos2(r.left() + s * 0.26, r.top() - s * 0.1),
                    pos2(r.left() + s * 0.34, r.top()),
                ],
                stroke,
            );
        }
        Icon::Save => {
            let r = Rect::from_center_size(c, Vec2::splat(s * 0.66));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Middle);
            let top = Rect::from_min_size(pos2(r.left() + s * 0.14, r.top()), vec2(s * 0.36, s * 0.2));
            painter.rect_stroke(top, CornerRadius::ZERO, stroke, StrokeKind::Middle);
            painter.circle_stroke(pos2(c.x, c.y + s * 0.12), s * 0.09, stroke);
        }
        Icon::Plus => {
            painter.line_segment([pos2(c.x - s * 0.3, c.y), pos2(c.x + s * 0.3, c.y)], stroke);
            painter.line_segment([pos2(c.x, c.y - s * 0.3), pos2(c.x, c.y + s * 0.3)], stroke);
        }
        Icon::Trash => {
            let r = Rect::from_min_max(
                pos2(c.x - s * 0.24, c.y - s * 0.18),
                pos2(c.x + s * 0.24, c.y + s * 0.36),
            );
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.line_segment(
                [
                    pos2(c.x - s * 0.34, c.y - s * 0.26),
                    pos2(c.x + s * 0.34, c.y - s * 0.26),
                ],
                stroke,
            );
            painter.line_segment(
                [pos2(c.x - s * 0.1, c.y - s * 0.36), pos2(c.x + s * 0.1, c.y - s * 0.36)],
                stroke,
            );
        }
        Icon::Shield => {
            let w = s * 0.36;
            let pts = vec![
                pos2(c.x, c.y - s * 0.42),
                pos2(c.x + w, c.y - s * 0.28),
                pos2(c.x + w * 0.92, c.y + s * 0.08),
                pos2(c.x, c.y + s * 0.44),
                pos2(c.x - w * 0.92, c.y + s * 0.08),
                pos2(c.x - w, c.y - s * 0.28),
            ];
            painter.add(Shape::convex_polygon(pts, color, Stroke::NONE));
        }
        Icon::Compact => {
            let r = Rect::from_center_size(c, vec2(s * 0.7, s * 0.3));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Middle);
        }
        Icon::Expand => {
            let r = Rect::from_center_size(c, vec2(s * 0.66, s * 0.56));
            painter.rect_stroke(r, CornerRadius::same(2), stroke, StrokeKind::Middle);
            painter.line_segment(
                [pos2(r.left(), r.top() + s * 0.16), pos2(r.right(), r.top() + s * 0.16)],
                stroke,
            );
        }
        Icon::Import => {
            painter.line_segment([pos2(c.x, c.y - s * 0.34), pos2(c.x, c.y + s * 0.1)], stroke);
            painter.line_segment([pos2(c.x - s * 0.16, c.y - s * 0.06), pos2(c.x, c.y + s * 0.1)], stroke);
            painter.line_segment([pos2(c.x + s * 0.16, c.y - s * 0.06), pos2(c.x, c.y + s * 0.1)], stroke);
            painter.line_segment(
                [
                    pos2(c.x - s * 0.32, c.y + s * 0.32),
                    pos2(c.x + s * 0.32, c.y + s * 0.32),
                ],
                stroke,
            );
        }
        Icon::Undo | Icon::Redo => {
            let dir = if icon == Icon::Undo { -1.0 } else { 1.0 };
            let pts: Vec<_> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * (0.15 + 0.85 * i as f32 / 12.0);
                    pos2(c.x - dir * a.cos() * s * 0.28, c.y + s * 0.06 - a.sin() * s * 0.28)
                })
                .collect();
            let tip = pts[0];
            painter.add(Shape::line(pts, stroke));
            painter.line_segment([tip, tip + vec2(dir * s * 0.02, -s * 0.18)], stroke);
            painter.line_segment([tip, tip + vec2(-dir * s * 0.16, -s * 0.06)], stroke);
        }
    }
}

/// A rounded card container.
pub fn card<R>(ui: &mut Ui, p: &Palette, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::new()
        .fill(p.card)
        .stroke(Stroke::new(1.0, p.border))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::same(16))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui)
        })
        .inner
}

pub fn card_title(ui: &mut Ui, p: &Palette, title: &str, subtitle: Option<&str>) {
    ui.label(RichText::new(title).size(15.5).strong().color(p.text));
    if let Some(s) = subtitle {
        ui.add_space(-4.0);
        ui.label(RichText::new(s).size(12.5).color(p.text_faint));
    }
    ui.add_space(4.0);
}

pub fn page_header(ui: &mut Ui, p: &Palette, title: &str, subtitle: &str) {
    ui.label(RichText::new(title).size(24.0).strong().color(p.text));
    ui.add_space(-4.0);
    ui.label(RichText::new(subtitle).size(13.0).color(p.text_dim));
    ui.add_space(10.0);
}

/// A big square action button with a painted icon (Record / Play / Stop).
#[allow(clippy::too_many_arguments)]
pub fn big_action(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    label: &str,
    hint: &str,
    color: Color32,
    active: bool,
    enabled: bool,
) -> Response {
    let size = vec2(132.0, 112.0);
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let painter = ui.painter_at(rect.expand(4.0));
    let hovered = enabled && resp.hovered();
    let t = ui.ctx().animate_bool(resp.id.with("hover"), hovered);
    let fill = if active {
        color.gamma_multiply(if p.dark { 0.28 } else { 0.16 })
    } else {
        p.raised.lerp_to_gamma(color, 0.06 + 0.10 * t)
    };
    let stroke_col = if active {
        color
    } else {
        p.border.lerp_to_gamma(color, 0.6 * t)
    };
    painter.rect(
        rect,
        CornerRadius::same(14),
        fill,
        Stroke::new(if active { 1.5 } else { 1.0 }, stroke_col),
        StrokeKind::Inside,
    );
    let icol = if enabled {
        color
    } else {
        p.text_faint.gamma_multiply(0.6)
    };
    // Icon in a soft circle.
    let ic = pos2(rect.center().x, rect.top() + 40.0);
    painter.circle_filled(
        ic,
        22.0 + 2.0 * t,
        icol.gamma_multiply(if p.dark { 0.18 } else { 0.12 }),
    );
    paint_icon(&painter, Rect::from_center_size(ic, Vec2::splat(30.0)), icon, icol);
    let text_col = if enabled { p.text } else { p.text_faint };
    painter.text(
        pos2(rect.center().x, rect.top() + 76.0),
        Align2::CENTER_CENTER,
        label,
        FontId::proportional(15.0),
        text_col,
    );
    painter.text(
        pos2(rect.center().x, rect.top() + 95.0),
        Align2::CENTER_CENTER,
        hint,
        FontId::proportional(10.5),
        p.text_faint,
    );
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Small toolbar button with icon + optional text.
pub fn tool_button(ui: &mut Ui, p: &Palette, icon: Icon, text: &str, enabled: bool) -> Response {
    tool_button_tinted(ui, p, icon, text, enabled, None)
}

/// Toolbar button whose icon uses a custom colour (e.g. red for Record).
pub fn tool_button_tinted(
    ui: &mut Ui,
    p: &Palette,
    icon: Icon,
    text: &str,
    enabled: bool,
    tint: Option<Color32>,
) -> Response {
    let font = FontId::proportional(13.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), p.text);
    let icon_w = 16.0;
    let gap = if text.is_empty() { 0.0 } else { 6.0 };
    let w = 10.0 + icon_w + gap + galley.size().x + 12.0;
    let (rect, resp) = ui.allocate_exact_size(
        vec2(w.max(32.0), 30.0),
        if enabled { Sense::click() } else { Sense::hover() },
    );
    let hovered = enabled && resp.hovered();
    let fill = if hovered {
        p.raised.lerp_to_gamma(p.accent, 0.12)
    } else {
        p.raised
    };
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS_SMALL),
        fill,
        Stroke::new(
            1.0,
            if hovered {
                p.accent.gamma_multiply(0.6)
            } else {
                p.border
            },
        ),
        StrokeKind::Inside,
    );
    let col = if enabled { p.text } else { p.text_faint };
    let icon_rect = Rect::from_center_size(
        pos2(rect.left() + 10.0 + icon_w / 2.0, rect.center().y),
        Vec2::splat(icon_w),
    );
    let icon_col = match (enabled, tint) {
        (true, Some(t)) => t,
        (true, None) => p.text_dim,
        _ => p.text_faint,
    };
    paint_icon(ui.painter(), icon_rect, icon, icon_col);
    if !text.is_empty() {
        ui.painter().text(
            pos2(icon_rect.right() + gap, rect.center().y),
            Align2::LEFT_CENTER,
            text,
            font,
            col,
        );
    }
    if enabled {
        resp.on_hover_cursor(egui::CursorIcon::PointingHand)
    } else {
        resp
    }
}

/// Filled accent ("primary") button.
pub fn primary_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).color(p.on_accent).strong())
        .fill(p.accent)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(RADIUS_SMALL))
        .min_size(vec2(0.0, 30.0));
    ui.add(b).on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn danger_button(ui: &mut Ui, p: &Palette, text: &str) -> Response {
    let b = egui::Button::new(RichText::new(text).color(Color32::WHITE).strong())
        .fill(p.danger)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(RADIUS_SMALL))
        .min_size(vec2(0.0, 30.0));
    ui.add(b)
}

/// iOS-style switch. Returns the response; flips `on` when clicked.
pub fn toggle(ui: &mut Ui, p: &Palette, on: &mut bool) -> Response {
    let size = vec2(38.0, 22.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let bg = p.raised.lerp_to_gamma(p.accent, t);
    ui.painter().rect(
        rect,
        CornerRadius::same(11),
        bg,
        Stroke::new(1.0, p.border.lerp_to_gamma(p.accent, t)),
        StrokeKind::Inside,
    );
    let x = egui::lerp((rect.left() + 11.0)..=(rect.right() - 11.0), t);
    ui.painter()
        .circle_filled(pos2(x, rect.center().y), 8.0, Color32::WHITE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A toggle row: title, description, switch on the right.
pub fn toggle_row(ui: &mut Ui, p: &Palette, on: &mut bool, title: &str, desc: &str) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_width((ui.available_width() - 56.0).max(100.0));
            ui.label(RichText::new(title).color(p.text));
            if !desc.is_empty() {
                ui.add_space(-6.0);
                ui.label(RichText::new(desc).size(12.0).color(p.text_faint));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            changed = toggle(ui, p, on).changed();
        });
    });
    changed
}

/// Segmented control. Returns true when the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, p: &Palette, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    let font = FontId::proportional(13.0);
    let widths: Vec<f32> = options
        .iter()
        .map(|(_, l)| {
            ui.painter()
                .layout_no_wrap(l.to_string(), font.clone(), p.text)
                .size()
                .x
                + 24.0
        })
        .collect();
    let total: f32 = widths.iter().sum::<f32>() + 4.0;
    let (rect, _) = ui.allocate_exact_size(vec2(total, 30.0), Sense::hover());
    ui.painter().rect(
        rect,
        CornerRadius::same(RADIUS_SMALL),
        p.raised,
        Stroke::new(1.0, p.border),
        StrokeKind::Inside,
    );
    let mut x = rect.left() + 2.0;
    for ((v, label), w) in options.iter().zip(widths) {
        let r = Rect::from_min_size(pos2(x, rect.top() + 2.0), vec2(w, rect.height() - 4.0));
        let id = ui.scope_id().with(("seg", label));
        let resp = ui.interact(r, id, Sense::click());
        let selected = *value == *v;
        if resp.clicked() && !selected {
            *value = *v;
            changed = true;
        }
        if selected {
            ui.painter()
                .rect_filled(r, CornerRadius::same(RADIUS_SMALL - 2), p.accent);
        } else if resp.hovered() {
            ui.painter()
                .rect_filled(r, CornerRadius::same(RADIUS_SMALL - 2), p.card_hover);
        }
        ui.painter().text(
            r.center(),
            Align2::CENTER_CENTER,
            *label,
            font.clone(),
            if selected { p.on_accent } else { p.text_dim },
        );
        resp.on_hover_cursor(egui::CursorIcon::PointingHand);
        x += w;
    }
    changed
}

/// A small coloured pill / badge.
pub fn pill(ui: &mut Ui, text: &str, fg: Color32, bg: Color32) -> Response {
    let font = FontId::proportional(11.5);
    let galley = ui.painter().layout_no_wrap(text.to_string(), font.clone(), fg);
    let size = vec2(galley.size().x + 16.0, 20.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    ui.painter().rect_filled(rect, CornerRadius::same(10), bg);
    ui.painter().text(rect.center(), Align2::CENTER_CENTER, text, font, fg);
    resp
}

/// Stat block: big value with a caption.
pub fn stat(ui: &mut Ui, p: &Palette, value: &str, caption: &str) {
    ui.vertical(|ui| {
        ui.label(RichText::new(value).size(19.0).strong().color(p.text));
        ui.add_space(-6.0);
        ui.label(RichText::new(caption).size(11.5).color(p.text_faint));
    });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerKind {
    Info,
    Warn,
}

/// A full-width notice. Returns the index of the clicked action, if any.
pub fn banner(ui: &mut Ui, p: &Palette, kind: BannerKind, title: &str, text: &str, actions: &[&str]) -> Option<usize> {
    let (bg, fg) = match kind {
        BannerKind::Info => (p.info_bg, p.accent),
        BannerKind::Warn => (p.warn_bg, p.warn_fg),
    };
    let mut clicked = None;
    egui::Frame::new()
        .fill(bg)
        .stroke(Stroke::new(1.0, fg.gamma_multiply(0.35)))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(4.0, 34.0), Sense::hover());
                ui.painter().rect_filled(r, CornerRadius::same(2), fg);
                ui.vertical(|ui| {
                    ui.set_width((ui.available_width() - 30.0 - 110.0 * actions.len() as f32).max(160.0));
                    ui.label(RichText::new(title).strong().color(fg));
                    ui.add_space(-6.0);
                    ui.label(RichText::new(text).size(12.5).color(p.text_dim));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (i, a) in actions.iter().enumerate().rev() {
                        if ui.button(*a).clicked() {
                            clicked = Some(i);
                        }
                    }
                });
            });
        });
    clicked
}

/// Labelled form row with a fixed-width label column.
pub fn form_row(ui: &mut Ui, p: &Palette, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(130.0, 28.0), Sense::hover());
        ui.painter().text(
            pos2(r.left(), r.center().y),
            Align2::LEFT_CENTER,
            label,
            FontId::proportional(13.5),
            p.text_dim,
        );
        add(ui);
    });
}

/// Draw the SentinelTask logo mark.
pub fn logo(ui: &mut Ui, p: &Palette, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, CornerRadius::same((size * 0.28) as u8), p.accent);
    paint_icon(painter, rect.shrink(size * 0.12), Icon::Shield, Color32::WHITE);
    painter.circle_filled(rect.center() + vec2(0.0, -size * 0.02), size * 0.11, p.accent);
    resp
}
