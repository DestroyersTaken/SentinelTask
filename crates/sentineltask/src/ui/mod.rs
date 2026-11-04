//! The egui user interface.

pub mod app;
pub mod editor;
pub mod pages;
pub mod recorder;
pub mod theme;
pub mod widgets;

/// Rasterise the app icon (accent rounded square with a white shield).
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let accent = theme::DARK.accent;
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let radius = s * 0.24;
    // Shield outline (unit coordinates, centred).
    let shield = [
        (0.0, -0.36),
        (0.30, -0.24),
        (0.28, 0.06),
        (0.0, 0.38),
        (-0.28, 0.06),
        (-0.30, -0.24),
    ];
    let inside_poly = |x: f32, y: f32| {
        let mut inside = false;
        let n = shield.len();
        for i in 0..n {
            let (xi, yi) = shield[i];
            let (xj, yj) = shield[(i + n - 1) % n];
            if (yi > y) != (yj > y) && x < (xj - xi) * (y - yi) / (yj - yi) + xi {
                inside = !inside;
            }
        }
        inside
    };
    for py in 0..size {
        for px in 0..size {
            // 4x supersampling for smooth edges.
            let mut cover_bg = 0.0;
            let mut cover_fg = 0.0;
            let mut cover_dot = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let x = px as f32 + (sx as f32 + 0.5) / 4.0;
                    let y = py as f32 + (sy as f32 + 0.5) / 4.0;
                    let cx = x.clamp(radius, s - radius);
                    let cy = y.clamp(radius, s - radius);
                    let in_bg = (x - cx).powi(2) + (y - cy).powi(2) <= radius * radius;
                    if in_bg {
                        cover_bg += 1.0 / 16.0;
                        let ux = x / s - 0.5;
                        let uy = y / s - 0.5;
                        if inside_poly(ux, uy) {
                            cover_fg += 1.0 / 16.0;
                            if ux * ux + (uy + 0.02).powi(2) <= 0.10 * 0.10 {
                                cover_dot += 1.0 / 16.0;
                            }
                        }
                    }
                }
            }
            let fg = cover_fg - cover_dot;
            let r = accent.r() as f32 * (cover_bg - fg) + 255.0 * fg;
            let g = accent.g() as f32 * (cover_bg - fg) + 255.0 * fg;
            let b = accent.b() as f32 * (cover_bg - fg) + 255.0 * fg;
            let a = cover_bg;
            let i = ((py * size + px) * 4) as usize;
            if a > 0.0 {
                out[i] = (r / a).round().clamp(0.0, 255.0) as u8;
                out[i + 1] = (g / a).round().clamp(0.0, 255.0) as u8;
                out[i + 2] = (b / a).round().clamp(0.0, 255.0) as u8;
                out[i + 3] = (a * 255.0).round() as u8;
            }
        }
    }
    out
}
