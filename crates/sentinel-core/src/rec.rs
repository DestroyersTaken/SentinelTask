//! Import of TinyTask `.rec` files (RESEARCH §3).
//!
//! A `.rec` file is an optional short header followed by 20-byte
//! little-endian `EVENTMSG` records: `{message, paramL, paramH, time, hwnd}`.
//! The header length varies between versions, so every plausible length is
//! tried and the one producing the most sensible event stream wins.

use crate::model::{Event, EventKind, Key, Macro, MouseButton};

pub const RECORD_SIZE: usize = 20;
const HEADER_CANDIDATES: [usize; 5] = [0, 4, 8, 12, 16];

const WM_KEYDOWN: u32 = 0x0100;
const WM_KEYUP: u32 = 0x0101;
const WM_SYSKEYDOWN: u32 = 0x0104;
const WM_SYSKEYUP: u32 = 0x0105;
const WM_MOUSEMOVE: u32 = 0x0200;
const WM_LBUTTONDOWN: u32 = 0x0201;
const WM_LBUTTONUP: u32 = 0x0202;
const WM_RBUTTONDOWN: u32 = 0x0204;
const WM_RBUTTONUP: u32 = 0x0205;
const WM_MBUTTONDOWN: u32 = 0x0207;
const WM_MBUTTONUP: u32 = 0x0208;
const WM_MOUSEWHEEL: u32 = 0x020A;
const WM_XBUTTONDOWN: u32 = 0x020B;
const WM_XBUTTONUP: u32 = 0x020C;
const WM_MOUSEHWHEEL: u32 = 0x020E;

fn is_known(message: u32) -> bool {
    matches!(
        message,
        WM_KEYDOWN
            | WM_KEYUP
            | WM_SYSKEYDOWN
            | WM_SYSKEYUP
            | WM_MOUSEMOVE
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_MBUTTONDOWN
            | WM_MBUTTONUP
            | WM_MOUSEWHEEL
            | WM_XBUTTONDOWN
            | WM_XBUTTONUP
            | WM_MOUSEHWHEEL
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EventMsg {
    pub message: u32,
    pub param_l: u32,
    pub param_h: u32,
    pub time: u32,
    pub hwnd: u32,
}

impl EventMsg {
    fn read(chunk: &[u8]) -> Self {
        let at = |i: usize| u32::from_le_bytes([chunk[i], chunk[i + 1], chunk[i + 2], chunk[i + 3]]);
        Self {
            message: at(0),
            param_l: at(4),
            param_h: at(8),
            time: at(12),
            hwnd: at(16),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecError {
    Empty,
    Unrecognised,
}

impl std::fmt::Display for RecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RecError::Empty => write!(f, "the file is empty"),
            RecError::Unrecognised => write!(f, "this doesn't look like a TinyTask .rec recording"),
        }
    }
}

impl std::error::Error for RecError {}

/// Positive time step between two tick values, tolerant of the 49.7-day
/// `GetTickCount` wrap and of slightly out-of-order records (TinyTask-Linux
/// fix #6), which are treated as simultaneous.
fn tick_delta(prev: u32, cur: u32) -> u32 {
    let d = cur.wrapping_sub(prev);
    if d > 0x8000_0000 {
        0
    } else {
        d
    }
}

fn decode(bytes: &[u8], header: usize) -> Option<Vec<EventMsg>> {
    let payload = bytes.get(header..)?;
    if payload.is_empty() || payload.len() % RECORD_SIZE != 0 {
        return None;
    }
    Some(payload.chunks_exact(RECORD_SIZE).map(EventMsg::read).collect())
}

/// Score a candidate decoding: known messages and monotonic, plausibly
/// spaced timestamps score positively.
fn score(msgs: &[EventMsg]) -> (f64, i64) {
    let mut known = 0usize;
    let mut s: i64 = 0;
    for (i, m) in msgs.iter().enumerate() {
        if is_known(m.message) {
            known += 1;
            s += 2;
        } else {
            s -= 6;
        }
        if i > 0 {
            let d = m.time.wrapping_sub(msgs[i - 1].time);
            // Within ten minutes forward is plausible.
            if d <= 600_000 {
                s += 1;
            } else {
                s -= 1;
            }
        }
    }
    (known as f64 / msgs.len().max(1) as f64, s)
}

/// Decode raw `EVENTMSG` records, auto-detecting the header length.
pub fn parse_records(bytes: &[u8]) -> Result<(usize, Vec<EventMsg>), RecError> {
    if bytes.is_empty() {
        return Err(RecError::Empty);
    }
    let mut best: Option<(i64, usize, Vec<EventMsg>)> = None;
    for h in HEADER_CANDIDATES {
        let Some(msgs) = decode(bytes, h) else { continue };
        let (ratio, s) = score(&msgs);
        if ratio < 0.9 {
            continue;
        }
        if best.as_ref().is_none_or(|b| s > b.0) {
            best = Some((s, h, msgs));
        }
    }
    best.map(|(_, h, m)| (h, m)).ok_or(RecError::Unrecognised)
}

/// Convert TinyTask records to SentinelTask events.
pub fn to_events(msgs: &[EventMsg]) -> Vec<Event> {
    let mut out = Vec::with_capacity(msgs.len());
    let mut t_ms: u64 = 0;
    let mut prev_time: Option<u32> = None;
    let mut pos = (0i32, 0i32);
    for m in msgs {
        if let Some(p) = prev_time {
            t_ms += tick_delta(p, m.time) as u64;
        }
        prev_time = Some(m.time);
        let t = t_ms * 1000;
        let xy = (m.param_l as i32, m.param_h as i32);
        let button = |b: MouseButton, down: bool| EventKind::MouseButton {
            button: b,
            down,
            x: xy.0,
            y: xy.1,
        };
        let kind = match m.message {
            WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
                let vk = (m.param_l & 0xFF) as u16;
                if vk == 0 {
                    continue;
                }
                let scan = ((m.param_l >> 8) & 0xFF) as u16;
                let extended = m.param_h & 0x8000 != 0 || (scan == 0 && crate::keys::is_extended_vk(vk));
                EventKind::Key {
                    key: Key::new(vk, scan, extended),
                    down: matches!(m.message, WM_KEYDOWN | WM_SYSKEYDOWN),
                }
            }
            WM_MOUSEMOVE => {
                pos = xy;
                EventKind::MouseMove { x: xy.0, y: xy.1 }
            }
            WM_LBUTTONDOWN => button(MouseButton::Left, true),
            WM_LBUTTONUP => button(MouseButton::Left, false),
            WM_RBUTTONDOWN => button(MouseButton::Right, true),
            WM_RBUTTONUP => button(MouseButton::Right, false),
            WM_MBUTTONDOWN => button(MouseButton::Middle, true),
            WM_MBUTTONUP => button(MouseButton::Middle, false),
            WM_XBUTTONDOWN => button(MouseButton::X1, true),
            WM_XBUTTONUP => button(MouseButton::X1, false),
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                let delta = m.param_h as i32;
                if delta == 0 {
                    continue;
                }
                EventKind::Wheel {
                    delta,
                    horizontal: m.message == WM_MOUSEHWHEEL,
                    x: pos.0,
                    y: pos.1,
                }
            }
            _ => continue,
        };
        if let EventKind::MouseButton { x, y, .. } = kind {
            pos = (x, y);
        }
        out.push(Event::new(t, kind));
    }
    crate::normalize::balance(&mut out);
    out
}

/// Import a whole `.rec` file as a macro.
pub fn import(bytes: &[u8], name: &str) -> Result<Macro, RecError> {
    let (header, msgs) = parse_records(bytes)?;
    let mut m = Macro::new(name);
    m.events = to_events(&msgs);
    m.meta.source = format!(
        "Imported from TinyTask .rec ({} records, {header}-byte header). Positions are as TinyTask saw them; if clicks are off on a scaled display, use the Editor's Rescale tool.",
        msgs.len()
    );
    Ok(m)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(msgs: &[(u32, u32, u32, u32)], header: &[u8]) -> Vec<u8> {
        let mut v = header.to_vec();
        for &(m, l, h, t) in msgs {
            for x in [m, l, h, t, 0] {
                v.extend_from_slice(&x.to_le_bytes());
            }
        }
        v
    }

    fn sample_msgs(t0: u32) -> Vec<(u32, u32, u32, u32)> {
        vec![
            (WM_MOUSEMOVE, 100, 200, t0),
            (WM_LBUTTONDOWN, 100, 200, t0.wrapping_add(50)),
            (WM_LBUTTONUP, 100, 200, t0.wrapping_add(120)),
            (WM_KEYDOWN, 0x41 | (0x1E << 8), 1, t0.wrapping_add(500)),
            (WM_KEYUP, 0x41 | (0x1E << 8), 1, t0.wrapping_add(560)),
            (WM_KEYDOWN, 0x26 | (0x48 << 8), 0x8001, t0.wrapping_add(600)),
            (WM_KEYUP, 0x26 | (0x48 << 8), 0x8001, t0.wrapping_add(650)),
            (WM_MOUSEWHEEL, 0, (-120i32) as u32, t0.wrapping_add(700)),
        ]
    }

    #[test]
    fn detects_header_lengths() {
        for header in [vec![], vec![8, 0, 0, 0], vec![8, 0, 0, 0, 1, 0, 0, 0]] {
            let bytes = rec(&sample_msgs(1_000_000), &header);
            let (h, msgs) = parse_records(&bytes).unwrap();
            assert_eq!(h, header.len());
            assert_eq!(msgs.len(), 8);
        }
    }

    #[test]
    fn converts_events_with_relative_times() {
        let bytes = rec(&sample_msgs(5_000), &[]);
        let m = import(&bytes, "x").unwrap();
        let e = &m.events;
        assert_eq!(e.len(), 8);
        assert_eq!(e[0].t_us, 0);
        assert_eq!(e[1].t_us, 50_000);
        assert_eq!(
            e[3].kind,
            EventKind::Key {
                key: Key::new(0x41, 0x1E, false),
                down: true
            }
        );
        assert_eq!(
            e[5].kind,
            EventKind::Key {
                key: Key::new(0x26, 0x48, true),
                down: true
            }
        );
        assert_eq!(
            e[7].kind,
            EventKind::Wheel {
                delta: -120,
                horizontal: false,
                x: 100,
                y: 200
            }
        );
    }

    #[test]
    fn survives_tick_wraparound_and_disorder() {
        let bytes = rec(&sample_msgs(u32::MAX - 100), &[]);
        let m = import(&bytes, "x").unwrap();
        assert_eq!(m.events[2].t_us, 120_000);
        assert_eq!(m.events.last().unwrap().t_us, 700_000);
        assert_eq!(tick_delta(1000, 990), 0);
        assert_eq!(tick_delta(u32::MAX, 4), 5);
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_records(&[]), Err(RecError::Empty));
        assert_eq!(parse_records(&[0xAB; 77]), Err(RecError::Unrecognised));
        let junk: Vec<u8> = (0..400u32).map(|i| (i * 37 % 251) as u8).collect();
        assert!(parse_records(&junk).is_err());
    }

    #[test]
    fn unbalanced_recording_is_fixed_up() {
        let bytes = rec(&[(WM_KEYUP, 0x10, 0, 0), (WM_KEYDOWN, 0x41, 0, 10)], &[]);
        let m = import(&bytes, "x").unwrap();
        // Orphan Shift-up dropped; 'A' gets a release.
        assert_eq!(m.events.len(), 2);
        assert!(matches!(m.events[1].kind, EventKind::Key { down: false, .. }));
    }
}
