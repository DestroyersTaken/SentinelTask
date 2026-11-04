//! Visual design: palette, typography and spacing for dark and light themes.

use eframe::egui::{self, epaint::Shadow, Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, TextStyle, Vec2};

use sentinel_core::settings::Theme;

/// Semantic colours used throughout the UI.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub bg: Color32,
    pub sidebar: Color32,
    pub card: Color32,
    pub card_hover: Color32,
    pub raised: Color32,
    pub border: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub text_faint: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub on_accent: Color32,
    pub record: Color32,
    pub play: Color32,
    pub pause: Color32,
    pub danger: Color32,
    pub warn_bg: Color32,
    pub warn_fg: Color32,
    pub info_bg: Color32,
    pub ok_bg: Color32,
    pub err_bg: Color32,
    pub dark: bool,
}

pub const DARK: Palette = Palette {
    bg: Color32::from_rgb(0x0E, 0x11, 0x17),
    sidebar: Color32::from_rgb(0x12, 0x16, 0x1E),
    card: Color32::from_rgb(0x17, 0x1C, 0x26),
    card_hover: Color32::from_rgb(0x1D, 0x23, 0x2F),
    raised: Color32::from_rgb(0x22, 0x29, 0x36),
    border: Color32::from_rgb(0x2A, 0x32, 0x40),
    text: Color32::from_rgb(0xE8, 0xEC, 0xF3),
    text_dim: Color32::from_rgb(0xA3, 0xAD, 0xBF),
    text_faint: Color32::from_rgb(0x6E, 0x79, 0x8C),
    accent: Color32::from_rgb(0x5B, 0x8D, 0xFF),
    accent_soft: Color32::from_rgb(0x1E, 0x2B, 0x4D),
    on_accent: Color32::WHITE,
    record: Color32::from_rgb(0xF2, 0x4E, 0x5E),
    play: Color32::from_rgb(0x2E, 0xCC, 0x8B),
    pause: Color32::from_rgb(0xF5, 0xB0, 0x3B),
    danger: Color32::from_rgb(0xF2, 0x4E, 0x5E),
    warn_bg: Color32::from_rgb(0x3A, 0x2E, 0x12),
    warn_fg: Color32::from_rgb(0xF8, 0xC9, 0x6B),
    info_bg: Color32::from_rgb(0x15, 0x24, 0x40),
    ok_bg: Color32::from_rgb(0x11, 0x33, 0x27),
    err_bg: Color32::from_rgb(0x40, 0x17, 0x1D),
    dark: true,
};

pub const LIGHT: Palette = Palette {
    bg: Color32::from_rgb(0xF4, 0xF6, 0xFA),
    sidebar: Color32::from_rgb(0xEB, 0xEF, 0xF5),
    card: Color32::WHITE,
    card_hover: Color32::from_rgb(0xF7, 0xF9, 0xFC),
    raised: Color32::from_rgb(0xEE, 0xF1, 0xF6),
    border: Color32::from_rgb(0xD9, 0xDF, 0xE8),
    text: Color32::from_rgb(0x15, 0x1A, 0x23),
    text_dim: Color32::from_rgb(0x4B, 0x55, 0x66),
    text_faint: Color32::from_rgb(0x85, 0x8F, 0x9F),
    accent: Color32::from_rgb(0x2F, 0x6B, 0xF2),
    accent_soft: Color32::from_rgb(0xE2, 0xEB, 0xFE),
    on_accent: Color32::WHITE,
    record: Color32::from_rgb(0xDC, 0x2F, 0x45),
    play: Color32::from_rgb(0x12, 0x9E, 0x67),
    pause: Color32::from_rgb(0xD9, 0x8A, 0x0B),
    danger: Color32::from_rgb(0xDC, 0x2F, 0x45),
    warn_bg: Color32::from_rgb(0xFF, 0xF4, 0xDB),
    warn_fg: Color32::from_rgb(0x8A, 0x5A, 0x00),
    info_bg: Color32::from_rgb(0xE6, 0xEE, 0xFF),
    ok_bg: Color32::from_rgb(0xDF, 0xF6, 0xEC),
    err_bg: Color32::from_rgb(0xFD, 0xE4, 0xE7),
    dark: false,
};

pub fn palette(theme: Theme) -> Palette {
    match theme {
        Theme::Dark => DARK,
        Theme::Light => LIGHT,
    }
}

pub const RADIUS: u8 = 10;
pub const RADIUS_SMALL: u8 = 7;

/// Apply the palette to egui's global style.
pub fn apply(ctx: &egui::Context, p: &Palette, zoom: f32) {
    let mut style = (*ctx.global_style()).clone();
    style.text_styles = [
        (TextStyle::Heading, FontId::new(22.0, FontFamily::Proportional)),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Monospace, FontId::new(13.0, FontFamily::Monospace)),
        (TextStyle::Button, FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small, FontId::new(11.5, FontFamily::Proportional)),
    ]
    .into();
    style.spacing.item_spacing = Vec2::new(8.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.interact_size = Vec2::new(36.0, 30.0);
    style.spacing.slider_width = 220.0;
    style.spacing.combo_width = 160.0;
    style.spacing.window_margin = Margin::same(16);
    style.spacing.menu_margin = Margin::same(8);

    let mut v = if p.dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.bg;
    v.window_fill = p.card;
    v.extreme_bg_color = if p.dark {
        Color32::from_rgb(0x0B, 0x0E, 0x13)
    } else {
        Color32::from_rgb(0xFA, 0xFB, 0xFD)
    };
    v.faint_bg_color = p.card_hover;
    v.code_bg_color = p.raised;
    v.hyperlink_color = p.accent;
    v.warn_fg_color = p.warn_fg;
    v.error_fg_color = p.danger;
    v.window_corner_radius = CornerRadius::same(12);
    v.menu_corner_radius = CornerRadius::same(RADIUS_SMALL);
    v.window_stroke = Stroke::new(1.0, p.border);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 110 } else { 40 }),
    };
    v.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 14,
        spread: 0,
        color: Color32::from_black_alpha(if p.dark { 90 } else { 30 }),
    };
    v.selection.bg_fill = p.accent.gamma_multiply(if p.dark { 0.45 } else { 0.25 });
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.slider_trailing_fill = true;

    let r = CornerRadius::same(RADIUS_SMALL);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.card;
    w.noninteractive.weak_bg_fill = p.card;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text_dim);
    w.noninteractive.corner_radius = r;

    w.inactive.bg_fill = p.raised;
    w.inactive.weak_bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::new(1.0, p.border);
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.corner_radius = r;

    w.hovered.bg_fill = p.card_hover.lerp_to_gamma(p.accent, 0.10);
    w.hovered.weak_bg_fill = p.raised.lerp_to_gamma(p.accent, 0.12);
    w.hovered.bg_stroke = Stroke::new(1.0, p.accent.gamma_multiply(0.7));
    w.hovered.fg_stroke = Stroke::new(1.5, p.text);
    w.hovered.corner_radius = r;
    w.hovered.expansion = 0.0;

    w.active.bg_fill = p.accent.gamma_multiply(0.35);
    w.active.weak_bg_fill = p.accent.gamma_multiply(0.30);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.5, p.text);
    w.active.corner_radius = r;
    w.active.expansion = 0.0;

    w.open.bg_fill = p.raised;
    w.open.weak_bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.accent);
    w.open.fg_stroke = Stroke::new(1.0, p.text);
    w.open.corner_radius = r;

    style.visuals = v;
    ctx.set_global_style(style);
    ctx.set_zoom_factor(zoom.clamp(0.75, 2.0));
}
