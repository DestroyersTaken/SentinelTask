//! Clean-up passes applied to raw recordings and to loaded/edited macros.

use std::collections::BTreeMap;

use crate::keys::Hotkey;
use crate::model::{Event, EventKind, Key, MouseButton};

/// Stable sort by time (events with equal times keep their order).
pub fn sort_events(events: &mut [Event]) {
    events.sort_by_key(|e| e.t_us);
}

/// Make every press have a release and every release have a press
/// (RESEARCH §1 #5, #18).
///
/// * Releases with no matching earlier press are dropped (for example the
///   key-ups of the hotkey that *started* the recording).
/// * Presses still held at the end get a release at the time of the last
///   event, so a macro can never leave a key stuck down.
pub fn balance(events: &mut Vec<Event>) {
    let mut held_keys: BTreeMap<(u16, bool), Key> = BTreeMap::new();
    let mut held_buttons: Vec<(MouseButton, i32, i32)> = Vec::new();
    events.retain(|e| match e.kind {
        EventKind::Key { key, down: true } => {
            held_keys.insert(key.identity(), key);
            true
        }
        EventKind::Key { key, down: false } => held_keys.remove(&key.identity()).is_some(),
        EventKind::MouseButton {
            button,
            down: true,
            x,
            y,
        } => {
            held_buttons.retain(|b| b.0 != button);
            held_buttons.push((button, x, y));
            true
        }
        EventKind::MouseButton {
            button, down: false, ..
        } => {
            let before = held_buttons.len();
            held_buttons.retain(|b| b.0 != button);
            held_buttons.len() != before
        }
        _ => true,
    });
    let end = events.last().map(|e| e.t_us).unwrap_or(0);
    // Last known cursor position for releasing buttons.
    let last_pos = events.iter().rev().find_map(|e| e.kind.position());
    for (button, x, y) in held_buttons.into_iter().rev() {
        let (x, y) = last_pos.unwrap_or((x, y));
        events.push(Event::new(
            end,
            EventKind::MouseButton {
                button,
                down: false,
                x,
                y,
            },
        ));
    }
    let keys: Vec<Key> = held_keys.into_values().collect();
    let (mods, normal): (Vec<Key>, Vec<Key>) = keys.into_iter().partition(|k| crate::keys::is_modifier_vk(k.vk));
    for key in normal.into_iter().chain(mods) {
        events.push(Event::new(end, EventKind::Key { key, down: false }));
    }
}

/// Remove the keystrokes of the hotkey that *stopped* a recording, and
/// nothing else (TinyTask-Linux fix #3: neighbouring events were deleted).
///
/// Only the trailing run of events is considered (mouse motion is skipped
/// over and kept). Within it, presses of keys that belong to the chord are
/// removed together with their releases; a complete earlier `Ctrl+C` that
/// happens to share `Ctrl` with the hotkey is untouched.
pub fn strip_trailing_hotkey(events: &mut Vec<Event>, hotkey: &Hotkey) {
    // Find the start of the trailing segment.
    let mut seg_start = events.len();
    for (i, e) in events.iter().enumerate().rev() {
        match e.kind {
            k if k.is_mouse_motion() => {}
            EventKind::Key { key, .. } if hotkey.involves_vk(key.vk) => {}
            _ => break,
        }
        seg_start = i;
    }
    if seg_start == events.len() {
        return;
    }
    // Chord keys pressed inside the trailing segment belong to the stop
    // chord: remove the press and its release. Releases of keys pressed
    // *before* the segment (a real Ctrl+C sharing Ctrl) are kept.
    let mut started_in_segment: Vec<(u16, bool)> = Vec::new();
    let mut idx = 0usize;
    events.retain(|e| {
        let i = idx;
        idx += 1;
        if i < seg_start {
            return true;
        }
        match e.kind {
            EventKind::Key { key, down: true } if hotkey.involves_vk(key.vk) => {
                if !started_in_segment.contains(&key.identity()) {
                    started_in_segment.push(key.identity());
                }
                false
            }
            EventKind::Key { key, down: false } if hotkey.involves_vk(key.vk) => {
                !started_in_segment.contains(&key.identity())
            }
            _ => true,
        }
    });
}

/// Reduce mouse-motion density to at most one sample per `interval_us`
/// (RESEARCH: high-rate mice produce 1000 events/s). Absolute moves keep the
/// latest position of each group; raw deltas are *summed* so no motion is
/// lost. Groups never span a click, key or wheel event, so a click always
/// happens after the motion that led to it.
pub fn coalesce_motion(events: &mut Vec<Event>, interval_us: u64) {
    if interval_us == 0 || events.len() < 2 {
        return;
    }
    let mut out: Vec<Event> = Vec::with_capacity(events.len());
    // (index in out, group start time)
    let mut move_group: Option<(usize, u64)> = None;
    let mut delta_group: Option<(usize, u64)> = None;
    for e in events.drain(..) {
        match e.kind {
            EventKind::MouseMove { x, y } => {
                if let Some((i, start)) = move_group {
                    if e.t_us.saturating_sub(start) < interval_us {
                        out[i].kind = EventKind::MouseMove { x, y };
                        out[i].t_us = e.t_us;
                        continue;
                    }
                }
                move_group = Some((out.len(), e.t_us));
                out.push(e);
            }
            EventKind::MouseDelta { dx, dy } => {
                if let Some((i, start)) = delta_group {
                    if e.t_us.saturating_sub(start) < interval_us {
                        if let EventKind::MouseDelta { dx: ax, dy: ay } = out[i].kind {
                            out[i].kind = EventKind::MouseDelta {
                                dx: ax.saturating_add(dx),
                                dy: ay.saturating_add(dy),
                            };
                            out[i].t_us = e.t_us;
                            continue;
                        }
                    }
                }
                delta_group = Some((out.len(), e.t_us));
                out.push(e);
            }
            _ => {
                move_group = None;
                delta_group = None;
                out.push(e);
            }
        }
    }
    // Updating a group's time to its latest member can't reorder events:
    // every member came after everything already in `out` up to that group,
    // except members of the *other* motion stream. Re-sort to be safe.
    sort_events(&mut out);
    *events = out;
}

/// Drop zero-length raw deltas.
pub fn drop_empty_deltas(events: &mut Vec<Event>) {
    events.retain(|e| !matches!(e.kind, EventKind::MouseDelta { dx: 0, dy: 0 }));
}

/// The standard pipeline for a fresh recording.
pub fn finish_recording(events: &mut Vec<Event>, stop_hotkey: Option<&Hotkey>, motion_interval_us: u64) {
    sort_events(events);
    if let Some(hk) = stop_hotkey {
        strip_trailing_hotkey(events, hk);
    }
    drop_empty_deltas(events);
    coalesce_motion(events, motion_interval_us);
    balance(events);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::vk;

    fn key(t: u64, code: u16, down: bool) -> Event {
        Event::new(
            t,
            EventKind::Key {
                key: Key::from_vk(code),
                down,
            },
        )
    }
    fn mv(t: u64, x: i32, y: i32) -> Event {
        Event::new(t, EventKind::MouseMove { x, y })
    }
    fn btn(t: u64, down: bool) -> Event {
        Event::new(
            t,
            EventKind::MouseButton {
                button: MouseButton::Left,
                down,
                x: 1,
                y: 2,
            },
        )
    }

    #[test]
    fn orphan_releases_are_dropped() {
        // Ctrl+Shift+R started the recording; only their ups were captured.
        let mut e = vec![
            key(1, vk::LCONTROL, false),
            key(2, vk::LSHIFT, false),
            key(3, 0x52, false),
            key(10, 0x41, true),
            key(20, 0x41, false),
            btn(30, false),
        ];
        balance(&mut e);
        assert_eq!(e, vec![key(10, 0x41, true), key(20, 0x41, false)]);
    }

    #[test]
    fn held_presses_get_released_at_end() {
        let mut e = vec![
            key(10, vk::LSHIFT, true),
            key(11, 0x41, true),
            btn(12, true),
            mv(50, 7, 8),
        ];
        balance(&mut e);
        let tail: Vec<_> = e[4..].iter().map(|e| (e.t_us, e.kind)).collect();
        assert_eq!(
            tail,
            vec![
                (
                    50,
                    EventKind::MouseButton {
                        button: MouseButton::Left,
                        down: false,
                        x: 7,
                        y: 8
                    }
                ),
                (
                    50,
                    EventKind::Key {
                        key: Key::from_vk(0x41),
                        down: false
                    }
                ),
                (
                    50,
                    EventKind::Key {
                        key: Key::from_vk(vk::LSHIFT),
                        down: false
                    }
                ),
            ]
        );
    }

    #[test]
    fn strips_only_stop_hotkey_presses() {
        let hk = Hotkey::with(true, false, true, 0x52); // Ctrl+Shift+R
        let mut e = vec![
            // A real Ctrl+C the user wants to keep.
            key(10, vk::LCONTROL, true),
            key(11, 0x43, true),
            key(12, 0x43, false),
            key(13, vk::LCONTROL, false),
            mv(14, 1, 1),
            // The stop chord (key-ups never recorded).
            key(20, vk::LCONTROL, true),
            mv(21, 2, 2),
            key(22, vk::LSHIFT, true),
            key(23, 0x52, true),
        ];
        strip_trailing_hotkey(&mut e, &hk);
        assert_eq!(
            e,
            vec![
                key(10, vk::LCONTROL, true),
                key(11, 0x43, true),
                key(12, 0x43, false),
                key(13, vk::LCONTROL, false),
                mv(14, 1, 1),
                mv(21, 2, 2),
            ]
        );
    }

    #[test]
    fn strip_handles_swallowed_main_key_and_single_key_hotkeys() {
        let hk = Hotkey::key(vk::F1 + 7); // F8
        let mut e = vec![key(1, 0x41, true), key(2, 0x41, false), key(3, vk::F1 + 7, true)];
        strip_trailing_hotkey(&mut e, &hk);
        assert_eq!(e.len(), 2);

        // Main key swallowed by RegisterHotKey: only the modifier remains.
        let hk = Hotkey::with(true, false, false, 0x51);
        let mut e = vec![btn(1, true), btn(2, false), key(3, vk::LCONTROL, true)];
        strip_trailing_hotkey(&mut e, &hk);
        assert_eq!(e, vec![btn(1, true), btn(2, false)]);
    }

    #[test]
    fn strip_removes_a_fully_released_stop_chord() {
        // The user released Ctrl/Alt/Shift before capture stopped.
        let hk = Hotkey::with(true, true, true, 0x52);
        let mut e = vec![
            key(1, 0x85, true),
            key(2, 0x85, false),
            key(3, vk::LCONTROL, true),
            key(4, vk::LMENU, true),
            key(5, vk::LSHIFT, true),
            key(6, vk::LSHIFT, false),
            key(7, vk::LMENU, false),
            key(8, vk::LCONTROL, false),
        ];
        strip_trailing_hotkey(&mut e, &hk);
        assert_eq!(e, vec![key(1, 0x85, true), key(2, 0x85, false)]);
    }

    #[test]
    fn strip_leaves_unrelated_recordings_alone() {
        let hk = Hotkey::key(vk::PAUSE);
        let orig = vec![key(1, 0x41, true), key(2, 0x41, false), mv(3, 1, 1)];
        let mut e = orig.clone();
        strip_trailing_hotkey(&mut e, &hk);
        assert_eq!(e, orig);
    }

    #[test]
    fn coalesce_keeps_latest_move_and_sums_deltas() {
        let mut e = vec![
            mv(0, 0, 0),
            mv(1000, 1, 1),
            mv(2000, 2, 2),
            Event::new(2500, EventKind::MouseDelta { dx: 1, dy: 0 }),
            Event::new(3000, EventKind::MouseDelta { dx: 2, dy: -1 }),
            mv(5000, 5, 5),
            btn(5100, true),
            mv(5200, 6, 6),
            btn(5300, false),
        ];
        coalesce_motion(&mut e, 4000);
        assert_eq!(
            e,
            vec![
                mv(2000, 2, 2),
                Event::new(3000, EventKind::MouseDelta { dx: 3, dy: -1 }),
                mv(5000, 5, 5),
                btn(5100, true),
                mv(5200, 6, 6),
                btn(5300, false),
            ]
        );
    }

    #[test]
    fn coalesce_preserves_total_delta() {
        let mut e: Vec<Event> = (0..1000)
            .map(|i| Event::new(i * 1000, EventKind::MouseDelta { dx: 1, dy: -2 }))
            .collect();
        coalesce_motion(&mut e, 4000);
        let (sx, sy) = e.iter().fold((0, 0), |acc, e| match e.kind {
            EventKind::MouseDelta { dx, dy } => (acc.0 + dx, acc.1 + dy),
            _ => acc,
        });
        assert_eq!((sx, sy), (1000, -2000));
        assert!(e.len() <= 250);
    }

    #[test]
    fn finish_recording_pipeline_is_idempotent() {
        let hk = Hotkey::key(vk::PAUSE);
        let mut e = vec![key(5, 0x41, true), mv(6, 1, 1), key(7, vk::PAUSE, true)];
        finish_recording(&mut e, Some(&hk), 4000);
        let once = e.clone();
        finish_recording(&mut e, Some(&hk), 4000);
        assert_eq!(e, once);
        // 'A' got its release; Pause is gone.
        assert_eq!(once.len(), 3);
    }
}
