//! Tracking of keys and buttons held down by playback, so that they can
//! always be released (RESEARCH §1 #5: stuck keys).

use std::collections::BTreeMap;

use crate::model::{EventKind, Key, MouseButton};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PressedState {
    keys: BTreeMap<(u16, bool), Key>,
    buttons: Vec<MouseButton>,
}

impl PressedState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Record the effect of an event that was just injected.
    pub fn apply(&mut self, kind: &EventKind) {
        match *kind {
            EventKind::Key { key, down: true } => {
                self.keys.insert(key.identity(), key);
            }
            EventKind::Key { key, down: false } => {
                self.keys.remove(&key.identity());
            }
            EventKind::MouseButton { button, down: true, .. } => {
                if !self.buttons.contains(&button) {
                    self.buttons.push(button);
                }
            }
            EventKind::MouseButton {
                button, down: false, ..
            } => self.buttons.retain(|b| *b != button),
            _ => {}
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty()
    }

    pub fn held_keys(&self) -> impl Iterator<Item = &Key> {
        self.keys.values()
    }

    pub fn held_buttons(&self) -> &[MouseButton] {
        &self.buttons
    }

    /// Release events for everything that is still held, and clear the
    /// state. Mouse buttons are released first (ending drags at the current
    /// position), then non-modifier keys, then modifiers last so that e.g.
    /// `Ctrl` is not released before the `C` of a held `Ctrl+C`.
    /// Button releases carry position (0, 0) and must be sent without a move.
    pub fn drain_releases(&mut self) -> Vec<EventKind> {
        let mut out = Vec::new();
        for b in self.buttons.drain(..).rev() {
            out.push(EventKind::MouseButton {
                button: b,
                down: false,
                x: 0,
                y: 0,
            });
        }
        let keys: Vec<Key> = std::mem::take(&mut self.keys).into_values().collect();
        let (mods, normal): (Vec<Key>, Vec<Key>) = keys.into_iter().partition(|k| crate::keys::is_modifier_vk(k.vk));
        for key in normal.into_iter().chain(mods) {
            out.push(EventKind::Key { key, down: false });
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::vk;

    #[test]
    fn tracks_and_releases_in_safe_order() {
        let mut s = PressedState::new();
        let ctrl = Key::new(vk::LCONTROL, 0x1D, false);
        let c = Key::new(0x43, 0x2E, false);
        s.apply(&EventKind::Key { key: ctrl, down: true });
        s.apply(&EventKind::Key { key: c, down: true });
        s.apply(&EventKind::MouseButton {
            button: MouseButton::Left,
            down: true,
            x: 5,
            y: 5,
        });
        let rel = s.drain_releases();
        assert!(s.is_empty());
        assert_eq!(rel.len(), 3);
        assert!(matches!(rel[0], EventKind::MouseButton { down: false, .. }));
        assert_eq!(rel[1], EventKind::Key { key: c, down: false });
        assert_eq!(rel[2], EventKind::Key { key: ctrl, down: false });
    }

    #[test]
    fn released_keys_are_forgotten() {
        let mut s = PressedState::new();
        let a = Key::from_vk(0x41);
        s.apply(&EventKind::Key { key: a, down: true });
        // Auto-repeat downs don't duplicate.
        s.apply(&EventKind::Key { key: a, down: true });
        s.apply(&EventKind::Key { key: a, down: false });
        assert!(s.is_empty());
        assert!(s.drain_releases().is_empty());
    }

    #[test]
    fn left_and_right_shift_are_separate() {
        let mut s = PressedState::new();
        let l = Key::new(vk::LSHIFT, 0x2A, false);
        let r = Key::new(vk::RSHIFT, 0x36, false);
        s.apply(&EventKind::Key { key: l, down: true });
        s.apply(&EventKind::Key { key: r, down: true });
        s.apply(&EventKind::Key { key: l, down: false });
        assert_eq!(s.held_keys().copied().collect::<Vec<_>>(), vec![r]);
    }
}
