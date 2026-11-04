//! Screen-coordinate math (RESEARCH §1 #1, #2, §3 MOUSEINPUT).
//!
//! All positions are physical pixels on the *virtual desktop*: the bounding
//! box of every monitor. Monitors left of or above the primary one have
//! negative coordinates, which TinyTask-era tools commonly mishandle.

use serde::{Deserialize, Serialize};

/// An axis-aligned rectangle in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> i32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> i32 {
        self.y + self.h
    }

    pub fn is_valid(&self) -> bool {
        self.w > 0 && self.h > 0
    }

    pub fn contains(&self, px: i32, py: i32) -> bool {
        px >= self.x && py >= self.y && px < self.right() && py < self.bottom()
    }

    /// Clamp a point so it lies on a pixel inside the rectangle.
    pub fn clamp_point(&self, px: i32, py: i32) -> (i32, i32) {
        if !self.is_valid() {
            return (px, py);
        }
        (px.clamp(self.x, self.right() - 1), py.clamp(self.y, self.bottom() - 1))
    }
}

/// Maximum value of the normalised absolute coordinate space used by
/// `SendInput` with `MOUSEEVENTF_ABSOLUTE`.
pub const ABS_MAX: i32 = 65535;

fn normalise_axis(p: i32, origin: i32, size: i32) -> i32 {
    if size <= 0 {
        return 0;
    }
    let offset = (p as i64 - origin as i64).clamp(0, size as i64 - 1);
    // Aim at the *centre* of the target pixel: floor((offset + 0.5) * 65536 / size).
    // Windows converts back with a multiply-and-shift, so aiming at the pixel
    // centre lands on the right pixel regardless of whether it floors or
    // rounds. 64-bit intermediates avoid the i32 overflow of 65535 * width.
    let v = ((2 * offset + 1) * 65536) / (2 * size as i64);
    v.clamp(0, ABS_MAX as i64) as i32
}

/// Convert a physical pixel position to `MOUSEEVENTF_ABSOLUTE |
/// MOUSEEVENTF_VIRTUALDESK` coordinates relative to `desktop`.
pub fn to_absolute(px: i32, py: i32, desktop: Rect) -> (i32, i32) {
    (
        normalise_axis(px, desktop.x, desktop.w),
        normalise_axis(py, desktop.y, desktop.h),
    )
}

/// The pixel Windows will put the cursor on for a given normalised
/// coordinate (floor model). Used by tests to prove the round trip.
pub fn from_absolute(ax: i32, ay: i32, desktop: Rect) -> (i32, i32) {
    let fx = (ax as i64 * desktop.w as i64) >> 16;
    let fy = (ay as i64 * desktop.h as i64) >> 16;
    (desktop.x + fx as i32, desktop.y + fy as i32)
}

/// Map a point recorded on `from` onto `to`, preserving its relative
/// position, clamped into `to`. The mapping is linear between the first and
/// last pixel on each axis, so edges and corners always map to edges and
/// corners (TinyTask-Linux fix #5).
pub fn rescale_point(px: i32, py: i32, from: Rect, to: Rect) -> (i32, i32) {
    if !from.is_valid() || !to.is_valid() || from == to {
        return (px, py);
    }
    let map = |p: i32, fo: i32, fs: i32, to_o: i32, ts: i32| -> i32 {
        if fs <= 1 {
            return to_o;
        }
        let off = (p as i64 - fo as i64).clamp(0, fs as i64 - 1);
        let num = off * (ts as i64 - 1);
        let den = fs as i64 - 1;
        // Integer round-half-up of num / den (num >= 0).
        let v = (2 * num + den) / (2 * den);
        (to_o as i64 + v) as i32
    };
    let nx = map(px, from.x, from.w, to.x, to.w);
    let ny = map(py, from.y, from.h, to.y, to.h);
    to.clamp_point(nx, ny)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roundtrip_all(desk: Rect) {
        for px in desk.x..desk.right() {
            let (ax, _) = to_absolute(px, desk.y, desk);
            let (bx, _) = from_absolute(ax, 0, desk);
            assert_eq!(bx, px, "x pixel {px} on {desk:?}");
        }
        for py in desk.y..desk.bottom() {
            let (_, ay) = to_absolute(desk.x, py, desk);
            let (_, by) = from_absolute(0, ay, desk);
            assert_eq!(by, py, "y pixel {py} on {desk:?}");
        }
    }

    #[test]
    fn every_pixel_roundtrips_on_common_layouts() {
        roundtrip_all(Rect::new(0, 0, 1920, 1080));
        roundtrip_all(Rect::new(0, 0, 3840, 2160));
        roundtrip_all(Rect::new(0, 0, 2560, 1440));
        roundtrip_all(Rect::new(0, 0, 1366, 768));
        // Second monitor to the left of and above the primary.
        roundtrip_all(Rect::new(-1920, -300, 3840 + 1920, 2160 + 300));
        // Ultra-wide triple monitor.
        roundtrip_all(Rect::new(-2560, 0, 2560 * 3, 1440));
    }

    #[test]
    fn far_corner_is_reachable() {
        // TinyTask-Linux fix #5: the far corner was missed after rescaling.
        let d = Rect::new(0, 0, 1920, 1080);
        let (ax, ay) = to_absolute(1919, 1079, d);
        assert_eq!(from_absolute(ax, ay, d), (1919, 1079));
        assert!(ax <= ABS_MAX && ay <= ABS_MAX);
    }

    #[test]
    fn out_of_range_points_are_clamped() {
        let d = Rect::new(0, 0, 100, 100);
        assert_eq!(
            from_absolute(to_absolute(-50, 500, d).0, to_absolute(-50, 500, d).1, d),
            (0, 99)
        );
    }

    #[test]
    fn rescale_identity_is_noop() {
        let d = Rect::new(0, 0, 1920, 1080);
        assert_eq!(rescale_point(123, 456, d, d), (123, 456));
    }

    #[test]
    fn rescale_maps_corners_and_centre() {
        let from = Rect::new(0, 0, 1920, 1080);
        let to = Rect::new(0, 0, 3840, 2160);
        assert_eq!(rescale_point(0, 0, from, to), (0, 0));
        assert_eq!(rescale_point(1919, 1079, from, to), (3839, 2159));
        let (cx, cy) = rescale_point(960, 540, from, to);
        assert!((cx - 1920).abs() <= 1 && (cy - 1080).abs() <= 1);
        // Down-scaling keeps the far corner on screen.
        assert_eq!(rescale_point(3839, 2159, to, from), (1919, 1079));
    }

    #[test]
    fn rescale_handles_negative_origins() {
        let from = Rect::new(-1920, 0, 3840, 1080);
        let to = Rect::new(0, 0, 1920, 1080);
        assert_eq!(rescale_point(-1920, 0, from, to), (0, 0));
        assert_eq!(rescale_point(1919, 1079, from, to), (1919, 1079));
    }

    #[test]
    fn contains_and_clamp() {
        let r = Rect::new(-10, -10, 20, 20);
        assert!(r.contains(-10, -10));
        assert!(r.contains(9, 9));
        assert!(!r.contains(10, 0));
        assert_eq!(r.clamp_point(100, -100), (9, -10));
    }
}
