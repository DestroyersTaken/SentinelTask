//! Virtual-key names, the extended-key table and hotkey definitions.

use serde::{Deserialize, Serialize};
use std::fmt;

pub mod vk {
    pub const CANCEL: u16 = 0x03;
    pub const BACK: u16 = 0x08;
    pub const TAB: u16 = 0x09;
    pub const RETURN: u16 = 0x0D;
    pub const SHIFT: u16 = 0x10;
    pub const CONTROL: u16 = 0x11;
    pub const MENU: u16 = 0x12;
    pub const PAUSE: u16 = 0x13;
    pub const CAPITAL: u16 = 0x14;
    pub const ESCAPE: u16 = 0x1B;
    pub const SPACE: u16 = 0x20;
    pub const PRIOR: u16 = 0x21;
    pub const NEXT: u16 = 0x22;
    pub const END: u16 = 0x23;
    pub const HOME: u16 = 0x24;
    pub const LEFT: u16 = 0x25;
    pub const UP: u16 = 0x26;
    pub const RIGHT: u16 = 0x27;
    pub const DOWN: u16 = 0x28;
    pub const SNAPSHOT: u16 = 0x2C;
    pub const INSERT: u16 = 0x2D;
    pub const DELETE: u16 = 0x2E;
    pub const LWIN: u16 = 0x5B;
    pub const RWIN: u16 = 0x5C;
    pub const APPS: u16 = 0x5D;
    pub const NUMPAD0: u16 = 0x60;
    pub const MULTIPLY: u16 = 0x6A;
    pub const ADD: u16 = 0x6B;
    pub const SUBTRACT: u16 = 0x6D;
    pub const DECIMAL: u16 = 0x6E;
    pub const DIVIDE: u16 = 0x6F;
    pub const F1: u16 = 0x70;
    pub const F24: u16 = 0x87;
    pub const NUMLOCK: u16 = 0x90;
    pub const SCROLL: u16 = 0x91;
    pub const LSHIFT: u16 = 0xA0;
    pub const RSHIFT: u16 = 0xA1;
    pub const LCONTROL: u16 = 0xA2;
    pub const RCONTROL: u16 = 0xA3;
    pub const LMENU: u16 = 0xA4;
    pub const RMENU: u16 = 0xA5;
}

/// Keys that need `KEYEVENTF_EXTENDEDKEY` (an E0-prefixed scan code).
///
/// `MapVirtualKey` cannot be relied on to report this (RESEARCH §1 #3), so
/// we keep our own table. Recorded keys carry the real flag from the hook;
/// this table is only for keys created by hand or imported without one.
pub fn is_extended_vk(code: u16) -> bool {
    matches!(
        code,
        vk::CANCEL
            | vk::PRIOR
            | vk::NEXT
            | vk::END
            | vk::HOME
            | vk::LEFT
            | vk::UP
            | vk::RIGHT
            | vk::DOWN
            | vk::SNAPSHOT
            | vk::INSERT
            | vk::DELETE
            | vk::LWIN
            | vk::RWIN
            | vk::APPS
            | vk::DIVIDE
            | vk::NUMLOCK
            | vk::RCONTROL
            | vk::RMENU
            // Browser, volume and media keys.
            | 0xA6..=0xB7
    )
}

pub fn is_modifier_vk(code: u16) -> bool {
    matches!(
        code,
        vk::SHIFT
            | vk::CONTROL
            | vk::MENU
            | vk::LSHIFT
            | vk::RSHIFT
            | vk::LCONTROL
            | vk::RCONTROL
            | vk::LMENU
            | vk::RMENU
            | vk::LWIN
            | vk::RWIN
    )
}

/// Which modifier (if any) a virtual key represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Modifier {
    Ctrl,
    Alt,
    Shift,
    Win,
}

pub fn modifier_of(code: u16) -> Option<Modifier> {
    match code {
        vk::CONTROL | vk::LCONTROL | vk::RCONTROL => Some(Modifier::Ctrl),
        vk::MENU | vk::LMENU | vk::RMENU => Some(Modifier::Alt),
        vk::SHIFT | vk::LSHIFT | vk::RSHIFT => Some(Modifier::Shift),
        vk::LWIN | vk::RWIN => Some(Modifier::Win),
        _ => None,
    }
}

/// Human-readable name for a virtual-key code.
pub fn vk_name(code: u16) -> String {
    let s: &str = match code {
        0x01 => "Mouse Left",
        0x02 => "Mouse Right",
        vk::CANCEL => "Break",
        0x04 => "Mouse Middle",
        0x05 => "Mouse X1",
        0x06 => "Mouse X2",
        vk::BACK => "Backspace",
        vk::TAB => "Tab",
        0x0C => "Clear",
        vk::RETURN => "Enter",
        vk::SHIFT => "Shift",
        vk::CONTROL => "Ctrl",
        vk::MENU => "Alt",
        vk::PAUSE => "Pause",
        vk::CAPITAL => "Caps Lock",
        vk::ESCAPE => "Esc",
        vk::SPACE => "Space",
        vk::PRIOR => "Page Up",
        vk::NEXT => "Page Down",
        vk::END => "End",
        vk::HOME => "Home",
        vk::LEFT => "Left",
        vk::UP => "Up",
        vk::RIGHT => "Right",
        vk::DOWN => "Down",
        vk::SNAPSHOT => "Print Screen",
        vk::INSERT => "Insert",
        vk::DELETE => "Delete",
        vk::LWIN => "Left Win",
        vk::RWIN => "Right Win",
        vk::APPS => "Menu",
        vk::MULTIPLY => "Num *",
        vk::ADD => "Num +",
        0x6C => "Num Separator",
        vk::SUBTRACT => "Num -",
        vk::DECIMAL => "Num .",
        vk::DIVIDE => "Num /",
        vk::NUMLOCK => "Num Lock",
        vk::SCROLL => "Scroll Lock",
        vk::LSHIFT => "Left Shift",
        vk::RSHIFT => "Right Shift",
        vk::LCONTROL => "Left Ctrl",
        vk::RCONTROL => "Right Ctrl",
        vk::LMENU => "Left Alt",
        vk::RMENU => "Right Alt",
        0xA6 => "Browser Back",
        0xA7 => "Browser Forward",
        0xA8 => "Browser Refresh",
        0xAD => "Volume Mute",
        0xAE => "Volume Down",
        0xAF => "Volume Up",
        0xB0 => "Next Track",
        0xB1 => "Previous Track",
        0xB2 => "Stop Media",
        0xB3 => "Play/Pause",
        0xBA => ";",
        0xBB => "=",
        0xBC => ",",
        0xBD => "-",
        0xBE => ".",
        0xBF => "/",
        0xC0 => "`",
        0xDB => "[",
        0xDC => "\\",
        0xDD => "]",
        0xDE => "'",
        0xE2 => "\\ (102)",
        _ => "",
    };
    if !s.is_empty() {
        return s.to_string();
    }
    match code {
        0x30..=0x39 | 0x41..=0x5A => char::from(code as u8).to_string(),
        vk::NUMPAD0..=0x69 => format!("Num {}", code - vk::NUMPAD0),
        vk::F1..=vk::F24 => format!("F{}", code - vk::F1 + 1),
        _ => format!("Key 0x{code:02X}"),
    }
}

/// Parse a key name produced by [`vk_name`] (case-insensitive) back to a code.
pub fn vk_from_name(name: &str) -> Option<u16> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    let lower = name.to_ascii_lowercase();
    // Common aliases.
    let alias = match lower.as_str() {
        "escape" => Some(vk::ESCAPE),
        "return" => Some(vk::RETURN),
        "pgup" | "pageup" => Some(vk::PRIOR),
        "pgdn" | "pagedown" => Some(vk::NEXT),
        "del" => Some(vk::DELETE),
        "ins" => Some(vk::INSERT),
        "control" => Some(vk::CONTROL),
        "scrolllock" => Some(vk::SCROLL),
        "printscreen" | "prtsc" => Some(vk::SNAPSHOT),
        _ => None,
    };
    if alias.is_some() {
        return alias;
    }
    if let Some(hex) = lower.strip_prefix("key 0x") {
        return u16::from_str_radix(hex, 16).ok();
    }
    (1u16..=0xFE).find(|&c| vk_name(c).eq_ignore_ascii_case(name))
}

/// A global hotkey: optional modifiers plus one main key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Hotkey {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub win: bool,
    pub vk: u16,
}

impl Hotkey {
    pub const fn key(vk: u16) -> Self {
        Self {
            ctrl: false,
            alt: false,
            shift: false,
            win: false,
            vk,
        }
    }

    pub const fn with(ctrl: bool, alt: bool, shift: bool, vk: u16) -> Self {
        Self {
            ctrl,
            alt,
            shift,
            win: false,
            vk,
        }
    }

    pub fn has_modifier(&self, m: Modifier) -> bool {
        match m {
            Modifier::Ctrl => self.ctrl,
            Modifier::Alt => self.alt,
            Modifier::Shift => self.shift,
            Modifier::Win => self.win,
        }
    }

    /// Whether `code` is one of the keys making up this chord.
    pub fn involves_vk(&self, code: u16) -> bool {
        if code == self.vk {
            return true;
        }
        modifier_of(code).is_some_and(|m| self.has_modifier(m))
    }

    /// A hotkey must have a non-modifier main key, and that key must not be
    /// a mouse button (TinyTask-Linux fix #9: a mouse button as hotkey key
    /// made the hotkey impossible to press).
    pub fn validate(&self) -> Result<(), String> {
        if self.vk == 0 {
            return Err("No key chosen".into());
        }
        if is_modifier_vk(self.vk) {
            return Err("A hotkey needs a non-modifier key (e.g. F8 or Ctrl+Shift+R)".into());
        }
        if matches!(self.vk, 0x01 | 0x02 | 0x04 | 0x05 | 0x06) {
            return Err("Mouse buttons can't be used as hotkeys".into());
        }
        Ok(())
    }

    /// Parse strings like `Ctrl+Shift+F8` or `Pause`.
    pub fn parse(s: &str) -> Option<Self> {
        let mut hk = Hotkey::key(0);
        let parts: Vec<&str> = s.split('+').map(str::trim).collect();
        // Allow a literal "+" key written as "Num +" by splitting carefully:
        // only the last part is the key; all others must be modifiers.
        let (last, mods) = parts.split_last()?;
        for m in mods {
            match m.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => hk.ctrl = true,
                "alt" => hk.alt = true,
                "shift" => hk.shift = true,
                "win" | "windows" => hk.win = true,
                // "Num +" splits into ["Num ", ""]; handle below.
                "num" if last.is_empty() => {}
                _ => return None,
            }
        }
        hk.vk = if last.is_empty() && s.trim_end().ends_with("Num +") {
            vk::ADD
        } else {
            vk_from_name(last)?
        };
        Some(hk)
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.ctrl {
            write!(f, "Ctrl+")?;
        }
        if self.alt {
            write!(f, "Alt+")?;
        }
        if self.shift {
            write!(f, "Shift+")?;
        }
        if self.win {
            write!(f, "Win+")?;
        }
        write!(f, "{}", vk_name(self.vk))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arrows_and_navigation_are_extended() {
        for code in [
            vk::LEFT,
            vk::UP,
            vk::RIGHT,
            vk::DOWN,
            vk::HOME,
            vk::END,
            vk::INSERT,
            vk::DELETE,
            vk::PRIOR,
            vk::NEXT,
        ] {
            assert!(is_extended_vk(code), "{}", vk_name(code));
        }
        assert!(is_extended_vk(vk::RCONTROL));
        assert!(!is_extended_vk(vk::LCONTROL));
        assert!(!is_extended_vk(0x41));
        assert!(!is_extended_vk(vk::NUMPAD0));
    }

    #[test]
    fn names_roundtrip_for_every_named_key() {
        for code in 1u16..=0xFE {
            let name = vk_name(code);
            let back = vk_from_name(&name).unwrap_or_else(|| panic!("{name}"));
            assert_eq!(vk_name(back), name, "code {code:#x}");
        }
    }

    #[test]
    fn hotkey_display_and_parse() {
        let hk = Hotkey::with(true, false, true, vk::F1 + 7);
        assert_eq!(hk.to_string(), "Ctrl+Shift+F8");
        assert_eq!(Hotkey::parse("Ctrl+Shift+F8"), Some(hk));
        assert_eq!(Hotkey::parse("ctrl + shift + f8"), Some(hk));
        assert_eq!(Hotkey::parse("Pause"), Some(Hotkey::key(vk::PAUSE)));
        assert_eq!(
            Hotkey::parse("Ctrl+Num +"),
            Some(Hotkey::with(true, false, false, vk::ADD))
        );
        assert_eq!(Hotkey::parse("Hyper+X"), None);
        assert_eq!(Hotkey::parse(""), None);
    }

    #[test]
    fn hotkey_validation() {
        assert!(Hotkey::key(vk::F1).validate().is_ok());
        assert!(Hotkey::key(vk::LSHIFT).validate().is_err());
        assert!(Hotkey::key(0x01).validate().is_err());
        assert!(Hotkey::key(0).validate().is_err());
    }

    #[test]
    fn involves_vk_matches_left_right_modifiers() {
        let hk = Hotkey::with(true, true, false, 0x52);
        assert!(hk.involves_vk(vk::LCONTROL));
        assert!(hk.involves_vk(vk::RMENU));
        assert!(hk.involves_vk(0x52));
        assert!(!hk.involves_vk(vk::LSHIFT));
        assert!(!hk.involves_vk(0x41));
    }
}
