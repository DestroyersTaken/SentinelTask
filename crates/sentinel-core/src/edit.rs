//! Editing operations used by the macro editor (RESEARCH §1 #11, #16).
//!
//! All operations keep events sorted and leave press/release pairs intact
//! where possible; callers run [`crate::normalize::balance`] before playback
//! anyway, so a deletion can never produce a stuck key.

use crate::coords::{rescale_point, Rect};
use crate::model::{Event, EventKind};
use crate::normalize::{balance, sort_events};

/// Gap before event `index` (time since the previous event, or since the
/// start for the first event).
pub fn gap_before(events: &[Event], index: usize) -> u64 {
    match index {
        0 => events.first().map(|e| e.t_us).unwrap_or(0),
        i => events[i].t_us.saturating_sub(events[i - 1].t_us),
    }
}

/// Change the gap before `index` to `new_gap_us`, shifting it and every
/// later event so all other gaps are unchanged.
pub fn set_gap_before(events: &mut [Event], index: usize, new_gap_us: u64) {
    if index >= events.len() {
        return;
    }
    let old = gap_before(events, index);
    if new_gap_us >= old {
        shift_from(events, index, (new_gap_us - old) as i64);
    } else {
        shift_from(events, index, -((old - new_gap_us) as i64));
    }
}

/// Insert a pause of `us` before event `index`.
pub fn insert_wait(events: &mut [Event], index: usize, us: u64) {
    shift_from(events, index, us as i64);
}

fn shift_from(events: &mut [Event], index: usize, delta: i64) {
    for e in events.iter_mut().skip(index) {
        e.t_us = (e.t_us as i64 + delta).max(0) as u64;
    }
    sort_events(events);
}

/// Delete the given event indices (any order, duplicates allowed). The gap
/// that preceded each deleted event is removed too, so later events move
/// earlier — unless `keep_timing` is set, in which case the remaining events
/// keep their absolute times.
pub fn delete_events(events: &mut Vec<Event>, indices: &[usize], keep_timing: bool) {
    let mut del = vec![false; events.len()];
    for &i in indices {
        if i < del.len() {
            del[i] = true;
        }
    }
    if !keep_timing {
        // Accumulate removed gaps and pull later events earlier.
        let mut removed: u64 = 0;
        let mut prev_t = 0u64;
        for (i, e) in events.iter_mut().enumerate() {
            let gap = e.t_us.saturating_sub(prev_t);
            prev_t = e.t_us;
            if del[i] {
                removed += gap;
            } else {
                e.t_us = e.t_us.saturating_sub(removed);
            }
        }
    }
    let mut i = 0;
    events.retain(|_| {
        let keep = !del[i];
        i += 1;
        keep
    });
    balance(events);
}

/// Shift everything so the first event happens `keep_us` after the start.
pub fn trim_leading_idle(events: &mut [Event], keep_us: u64) {
    let Some(first) = events.first().map(|e| e.t_us) else {
        return;
    };
    if first > keep_us {
        let d = first - keep_us;
        for e in events.iter_mut() {
            e.t_us -= d;
        }
    }
}

/// Shorten every pause longer than `max_gap_us` to exactly `max_gap_us`.
/// Returns the total time removed.
pub fn cap_pauses(events: &mut [Event], max_gap_us: u64) -> u64 {
    let mut removed = 0u64;
    let mut prev_orig = 0u64;
    for e in events.iter_mut() {
        let orig = e.t_us;
        let gap = orig.saturating_sub(prev_orig);
        if gap > max_gap_us {
            removed += gap - max_gap_us;
        }
        prev_orig = orig;
        e.t_us = orig - removed;
    }
    removed
}

/// Multiply all times by `factor` (0.5 = twice as fast, permanently).
pub fn scale_time(events: &mut [Event], factor: f64) {
    if !factor.is_finite() || factor <= 0.0 {
        return;
    }
    for e in events.iter_mut() {
        e.t_us = (e.t_us as f64 * factor).round() as u64;
    }
}

/// Which kinds of event a filter removes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KindFilter {
    pub moves: bool,
    pub deltas: bool,
    pub clicks: bool,
    pub wheel: bool,
    pub keys: bool,
}

impl KindFilter {
    pub fn matches(&self, k: &EventKind) -> bool {
        match k {
            EventKind::MouseMove { .. } => self.moves,
            EventKind::MouseDelta { .. } => self.deltas,
            EventKind::MouseButton { .. } => self.clicks,
            EventKind::Wheel { .. } => self.wheel,
            EventKind::Key { .. } => self.keys,
        }
    }
}

/// Remove all events of the selected kinds, keeping absolute timing.
pub fn remove_kinds(events: &mut Vec<Event>, filter: KindFilter) -> usize {
    let before = events.len();
    events.retain(|e| !filter.matches(&e.kind));
    balance(events);
    before.saturating_sub(events.len())
}

/// Move every positioned event by `(dx, dy)` pixels (e.g. the target window
/// moved). Only events in `indices` if given.
pub fn offset_positions(events: &mut [Event], dx: i32, dy: i32, indices: Option<&[usize]>) {
    let apply = |e: &mut Event| {
        if let Some((x, y)) = e.kind.position() {
            e.kind.set_position(x.saturating_add(dx), y.saturating_add(dy));
        }
    };
    match indices {
        Some(ix) => {
            for &i in ix {
                if let Some(e) = events.get_mut(i) {
                    apply(e);
                }
            }
        }
        None => events.iter_mut().for_each(apply),
    }
}

/// Rescale every positioned event from one screen layout to another.
pub fn rescale_positions(events: &mut [Event], from: Rect, to: Rect) {
    for e in events.iter_mut() {
        if let Some((x, y)) = e.kind.position() {
            let (nx, ny) = rescale_point(x, y, from, to);
            e.kind.set_position(nx, ny);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Key, MouseButton};

    fn mv(t: u64) -> Event {
        Event::new(t, EventKind::MouseMove { x: 0, y: 0 })
    }
    fn times(e: &[Event]) -> Vec<u64> {
        e.iter().map(|e| e.t_us).collect()
    }

    #[test]
    fn gap_editing_shifts_following_events() {
        let mut e = vec![mv(100), mv(200), mv(1000)];
        assert_eq!(gap_before(&e, 0), 100);
        assert_eq!(gap_before(&e, 2), 800);
        set_gap_before(&mut e, 1, 50);
        assert_eq!(times(&e), vec![100, 150, 950]);
        set_gap_before(&mut e, 2, 1000);
        assert_eq!(times(&e), vec![100, 150, 1150]);
        insert_wait(&mut e, 0, 10);
        assert_eq!(times(&e), vec![110, 160, 1160]);
    }

    #[test]
    fn delete_pulls_later_events_earlier() {
        let mut e = vec![mv(100), mv(500), mv(600)];
        delete_events(&mut e, &[1], false);
        assert_eq!(times(&e), vec![100, 200]);
        let mut e = vec![mv(100), mv(500), mv(600)];
        delete_events(&mut e, &[1, 1, 99], true);
        assert_eq!(times(&e), vec![100, 600]);
    }

    #[test]
    fn deleting_a_key_up_never_leaves_key_stuck() {
        let k = Key::from_vk(0x41);
        let mut e = vec![
            Event::new(0, EventKind::Key { key: k, down: true }),
            Event::new(10, EventKind::Key { key: k, down: false }),
            mv(20),
        ];
        delete_events(&mut e, &[1], true);
        assert_eq!(e.last().unwrap().kind, EventKind::Key { key: k, down: false });
    }

    #[test]
    fn trim_and_cap() {
        let mut e = vec![mv(5_000_000), mv(5_100_000), mv(65_100_000)];
        trim_leading_idle(&mut e, 500_000);
        assert_eq!(times(&e), vec![500_000, 600_000, 60_600_000]);
        let removed = cap_pauses(&mut e, 1_000_000);
        assert_eq!(removed, 59_000_000);
        assert_eq!(times(&e), vec![500_000, 600_000, 1_600_000]);
    }

    #[test]
    fn remove_kinds_and_scale() {
        let mut e = vec![
            mv(0),
            Event::new(
                10,
                EventKind::MouseButton {
                    button: MouseButton::Left,
                    down: true,
                    x: 3,
                    y: 4,
                },
            ),
            mv(20),
            Event::new(
                30,
                EventKind::MouseButton {
                    button: MouseButton::Left,
                    down: false,
                    x: 3,
                    y: 4,
                },
            ),
        ];
        let n = remove_kinds(
            &mut e,
            KindFilter {
                moves: true,
                ..Default::default()
            },
        );
        assert_eq!(n, 2);
        scale_time(&mut e, 0.5);
        assert_eq!(times(&e), vec![5, 15]);
        scale_time(&mut e, f64::NAN);
        assert_eq!(times(&e), vec![5, 15]);
    }

    #[test]
    fn offsets_and_rescale() {
        let mut e = vec![
            Event::new(0, EventKind::MouseMove { x: 10, y: 10 }),
            Event::new(1, EventKind::MouseDelta { dx: 1, dy: 1 }),
        ];
        offset_positions(&mut e, 5, -5, None);
        assert_eq!(e[0].kind, EventKind::MouseMove { x: 15, y: 5 });
        assert_eq!(e[1].kind, EventKind::MouseDelta { dx: 1, dy: 1 });
        rescale_positions(&mut e, Rect::new(0, 0, 100, 100), Rect::new(0, 0, 200, 200));
        assert_eq!(e[0].kind, EventKind::MouseMove { x: 30, y: 10 });
    }
}
