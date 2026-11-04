//! The `.stmacro` file format (RESEARCH §1 #15, #16).
//!
//! Plain, versioned JSON so macros can be inspected, diffed and repaired by
//! hand. Events are stored as short arrays to keep long recordings small:
//!
//! ```text
//! [t_us, "M", x, y]                         mouse move (absolute pixels)
//! [t_us, "D", dx, dy]                       raw mouse delta
//! [t_us, "B", button, down, x, y]           button 0=L 1=R 2=M 3=X1 4=X2
//! [t_us, "W", delta, horizontal, x, y]      wheel
//! [t_us, "K", vk, scan, extended, down]     key
//! ```

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::model::{Event, EventKind, Key, Macro, MacroMeta, MouseButton};
use crate::playback::PlaybackOptions;

pub const FORMAT_ID: &str = "sentineltask-macro";
pub const FORMAT_VERSION: u32 = 1;
pub const FILE_EXTENSION: &str = "stmacro";

#[derive(Debug)]
pub enum FormatError {
    Json(serde_json::Error),
    NotAMacro,
    TooNew(u32),
    BadEvent { index: usize, reason: String },
}

impl std::fmt::Display for FormatError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FormatError::Json(e) => write!(f, "invalid JSON: {e}"),
            FormatError::NotAMacro => write!(f, "this file is not a SentinelTask macro"),
            FormatError::TooNew(v) => write!(
                f,
                "this macro was saved by a newer SentinelTask (format v{v}); please update"
            ),
            FormatError::BadEvent { index, reason } => write!(f, "event #{index} is invalid: {reason}"),
        }
    }
}

impl std::error::Error for FormatError {}

impl From<serde_json::Error> for FormatError {
    fn from(e: serde_json::Error) -> Self {
        FormatError::Json(e)
    }
}

#[derive(Serialize, Deserialize)]
struct FileHeader {
    format: String,
    version: u32,
    meta: MacroMeta,
    #[serde(default)]
    playback: PlaybackOptions,
    events: Vec<Value>,
}

fn b(v: bool) -> u8 {
    v as u8
}

pub fn event_to_value(e: &Event) -> Value {
    let t = e.t_us;
    match e.kind {
        EventKind::MouseMove { x, y } => json!([t, "M", x, y]),
        EventKind::MouseDelta { dx, dy } => json!([t, "D", dx, dy]),
        EventKind::MouseButton { button, down, x, y } => json!([t, "B", button.code(), b(down), x, y]),
        EventKind::Wheel {
            delta,
            horizontal,
            x,
            y,
        } => json!([t, "W", delta, b(horizontal), x, y]),
        EventKind::Key { key, down } => json!([t, "K", key.vk, key.scan, b(key.extended), b(down)]),
    }
}

pub fn event_from_value(v: &Value) -> Result<Event, String> {
    let arr = v.as_array().ok_or("not an array")?;
    let int = |i: usize| -> Result<i64, String> {
        arr.get(i)
            .and_then(Value::as_i64)
            .ok_or_else(|| format!("field {i} missing or not an integer"))
    };
    let i32_at = |i: usize| -> Result<i32, String> {
        let v = int(i)?;
        i32::try_from(v).map_err(|_| format!("field {i} out of range"))
    };
    let flag = |i: usize| -> Result<bool, String> {
        match arr.get(i) {
            Some(Value::Bool(b)) => Ok(*b),
            Some(v) => v
                .as_i64()
                .map(|n| n != 0)
                .ok_or_else(|| format!("field {i} not a flag")),
            None => Err(format!("field {i} missing")),
        }
    };
    let t = arr.first().and_then(Value::as_u64).ok_or("time missing or negative")?;
    let tag = arr.get(1).and_then(Value::as_str).ok_or("type tag missing")?;
    let kind = match tag {
        "M" => EventKind::MouseMove {
            x: i32_at(2)?,
            y: i32_at(3)?,
        },
        "D" => EventKind::MouseDelta {
            dx: i32_at(2)?,
            dy: i32_at(3)?,
        },
        "B" => EventKind::MouseButton {
            button: MouseButton::from_code(int(2)? as u8).ok_or("unknown mouse button")?,
            down: flag(3)?,
            x: i32_at(4)?,
            y: i32_at(5)?,
        },
        "W" => EventKind::Wheel {
            delta: i32_at(2)?,
            horizontal: flag(3)?,
            x: i32_at(4)?,
            y: i32_at(5)?,
        },
        "K" => {
            let vk = u16::try_from(int(2)?).map_err(|_| "vk out of range")?;
            let scan = u16::try_from(int(3)?).map_err(|_| "scan out of range")?;
            EventKind::Key {
                key: Key::new(vk, scan, flag(4)?),
                down: flag(5)?,
            }
        }
        other => return Err(format!("unknown event type {other:?}")),
    };
    Ok(Event::new(t, kind))
}

/// Serialise a macro to pretty-ish JSON (one event per line).
pub fn to_string(m: &Macro) -> String {
    let header = FileHeader {
        format: FORMAT_ID.into(),
        version: FORMAT_VERSION,
        meta: m.meta.clone(),
        playback: m.playback,
        events: Vec::new(),
    };
    // Pretty-print the header, then write events compactly one per line so
    // files stay readable without being enormous.
    let mut head = serde_json::to_string_pretty(&header).expect("header serialises");
    let marker = "\"events\": []";
    let pos = head.rfind(marker).expect("events field present");
    let mut events = String::from("\"events\": [");
    for (i, e) in m.events.iter().enumerate() {
        events.push_str(if i == 0 { "\n    " } else { ",\n    " });
        events.push_str(&event_to_value(e).to_string());
    }
    if !m.events.is_empty() {
        events.push_str("\n  ");
    }
    events.push(']');
    head.replace_range(pos..pos + marker.len(), &events);
    head.push('\n');
    head
}

/// Parse a macro. Events are sorted and balanced on load so a hand-edited
/// file can never leave keys stuck.
pub fn from_str(s: &str) -> Result<Macro, FormatError> {
    let header: FileHeader = serde_json::from_str(s)?;
    if header.format != FORMAT_ID {
        return Err(FormatError::NotAMacro);
    }
    if header.version > FORMAT_VERSION {
        return Err(FormatError::TooNew(header.version));
    }
    let mut events = Vec::with_capacity(header.events.len());
    for (index, v) in header.events.iter().enumerate() {
        events.push(event_from_value(v).map_err(|reason| FormatError::BadEvent { index, reason })?);
    }
    crate::normalize::sort_events(&mut events);
    crate::normalize::balance(&mut events);
    Ok(Macro {
        meta: header.meta,
        playback: header.playback,
        events,
    })
}

/// Quick check used when deciding how to open a file.
pub fn looks_like_macro(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(512)];
    let s = String::from_utf8_lossy(head);
    s.trim_start().starts_with('{') && s.contains(FORMAT_ID)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coords::Rect;
    use crate::model::{ScreenInfo, TargetWindow};
    use crate::playback::{MouseMode, Repeat};

    fn sample() -> Macro {
        let mut m = Macro::new("Farm loop");
        m.meta.description = "Collects \"stuff\"\nline two".into();
        m.meta.screen = Some(ScreenInfo {
            desktop: Rect::new(-1920, 0, 3840, 1080),
            primary: Rect::new(0, 0, 1920, 1080),
            dpi: 144,
        });
        m.meta.target = Some(TargetWindow {
            process: "game.exe".into(),
            title: "Game".into(),
        });
        m.playback = PlaybackOptions {
            speed: 1.5,
            repeat: Repeat::Duration { secs: 3600 },
            loop_delay_ms: 250,
            mouse_mode: MouseMode::Relative,
            scale_to_screen: true,
            focus_guard: true,
            clicks_only: false,
        };
        let k = Key::new(0x26, 0x48, true);
        m.events = vec![
            Event::new(0, EventKind::MouseMove { x: -5, y: 7 }),
            Event::new(10, EventKind::MouseDelta { dx: -3, dy: 4 }),
            Event::new(
                20,
                EventKind::MouseButton {
                    button: MouseButton::X2,
                    down: true,
                    x: -5,
                    y: 7,
                },
            ),
            Event::new(
                30,
                EventKind::MouseButton {
                    button: MouseButton::X2,
                    down: false,
                    x: -5,
                    y: 7,
                },
            ),
            Event::new(
                40,
                EventKind::Wheel {
                    delta: -240,
                    horizontal: true,
                    x: 1,
                    y: 2,
                },
            ),
            Event::new(50, EventKind::Key { key: k, down: true }),
            Event::new(60, EventKind::Key { key: k, down: false }),
        ];
        m
    }

    #[test]
    fn roundtrip_is_lossless() {
        let m = sample();
        let s = to_string(&m);
        let back = from_str(&s).unwrap();
        assert_eq!(back, m);
        assert!(looks_like_macro(s.as_bytes()));
        // One event per line.
        assert_eq!(s.lines().filter(|l| l.trim_start().starts_with('[')).count(), 7);
    }

    #[test]
    fn empty_macro_roundtrips() {
        let m = Macro::new("empty");
        assert_eq!(from_str(&to_string(&m)).unwrap(), m);
    }

    #[test]
    fn rejects_foreign_and_future_files() {
        assert!(matches!(
            from_str("{\"format\":\"x\",\"version\":1,\"meta\":{\"name\":\"a\"},\"events\":[]}"),
            Err(FormatError::NotAMacro)
        ));
        let future = format!("{{\"format\":\"{FORMAT_ID}\",\"version\":99,\"meta\":{{\"name\":\"a\"}},\"events\":[]}}");
        assert!(matches!(from_str(&future), Err(FormatError::TooNew(99))));
        assert!(matches!(from_str("not json"), Err(FormatError::Json(_))));
    }

    #[test]
    fn bad_events_report_their_index() {
        let s = format!("{{\"format\":\"{FORMAT_ID}\",\"version\":1,\"meta\":{{\"name\":\"a\"}},\"events\":[[0,\"M\",1,2],[5,\"Q\"]]}}");
        match from_str(&s) {
            Err(FormatError::BadEvent { index, .. }) => assert_eq!(index, 1),
            other => panic!("{other:?}"),
        }
        let s = format!(
            "{{\"format\":\"{FORMAT_ID}\",\"version\":1,\"meta\":{{\"name\":\"a\"}},\"events\":[[-1,\"M\",1,2]]}}"
        );
        assert!(from_str(&s).is_err());
    }

    #[test]
    fn hand_edited_files_are_sorted_and_balanced() {
        let s = format!(
            "{{\"format\":\"{FORMAT_ID}\",\"version\":1,\"meta\":{{\"name\":\"a\"}},\"events\":[[50,\"K\",65,30,false,true],[10,\"M\",1,1]]}}"
        );
        let m = from_str(&s).unwrap();
        assert_eq!(m.events.len(), 3);
        assert_eq!(m.events[0].t_us, 10);
        assert!(matches!(m.events[2].kind, EventKind::Key { down: false, .. }));
    }

    #[test]
    fn missing_playback_uses_defaults() {
        let s = format!("{{\"format\":\"{FORMAT_ID}\",\"version\":1,\"meta\":{{\"name\":\"a\"}},\"events\":[]}}");
        assert_eq!(from_str(&s).unwrap().playback, PlaybackOptions::default());
    }
}
