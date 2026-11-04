//! Application settings, persisted as JSON in the user's config folder.
//! Every field has a default so older settings files keep loading
//! (atbswp #107: old config files crashed newer versions).

use serde::{Deserialize, Serialize};

use crate::clicker::ClickerConfig;
use crate::keys::{vk, Hotkey};
use crate::playback::PlaybackOptions;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OverlayCorner {
    TopLeft,
    #[default]
    TopRight,
    BottomLeft,
    BottomRight,
    Hidden,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    /// Start / stop recording.
    pub record: Hotkey,
    /// Start / stop playback.
    pub play: Hotkey,
    /// Pause / resume playback.
    pub pause: Hotkey,
    /// Always stops everything (playback, recording, auto-clicker).
    pub emergency_stop: Hotkey,
    /// Start / stop the auto-clicker.
    pub clicker: Hotkey,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            // TinyTask's defaults, so muscle memory carries over.
            record: Hotkey::with(true, true, true, 0x52), // Ctrl+Alt+Shift+R
            play: Hotkey::with(true, true, true, 0x50),   // Ctrl+Alt+Shift+P
            pause: Hotkey::key(vk::F1 + 8),               // F9
            emergency_stop: Hotkey::key(vk::PAUSE),       // Pause/Break
            clicker: Hotkey::key(vk::F1 + 5),             // F6
        }
    }
}

impl Hotkeys {
    pub fn all(&self) -> [(&'static str, Hotkey); 5] {
        [
            ("Record", self.record),
            ("Play", self.play),
            ("Pause", self.pause),
            ("Emergency stop", self.emergency_stop),
            ("Auto-clicker", self.clicker),
        ]
    }

    /// Names of hotkeys that share the same chord.
    pub fn duplicates(&self) -> Vec<(&'static str, &'static str)> {
        let all = self.all();
        let mut out = Vec::new();
        for i in 0..all.len() {
            for j in i + 1..all.len() {
                if all[i].1 == all[j].1 {
                    out.push((all[i].0, all[j].0));
                }
            }
        }
        out
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RecordingSettings {
    pub record_mouse_moves: bool,
    /// Also capture raw relative motion (needed for 3D game cameras).
    pub record_raw_deltas: bool,
    pub record_keyboard: bool,
    /// Mouse-motion sample rate after coalescing, in Hz.
    pub mouse_sample_hz: u32,
    /// Countdown before recording starts (seconds, 0 = none).
    pub countdown_secs: u32,
    /// Remember the foreground window as the macro's target.
    pub capture_target_window: bool,
}

impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            record_mouse_moves: true,
            record_raw_deltas: true,
            record_keyboard: true,
            mouse_sample_hz: 250,
            countdown_secs: 0,
            capture_target_window: true,
        }
    }
}

impl RecordingSettings {
    pub fn motion_interval_us(&self) -> u64 {
        let hz = self.mouse_sample_hz.clamp(30, 1000) as u64;
        1_000_000 / hz
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SafetySettings {
    /// Stop playback when the physical mouse is moved more than a few px.
    pub stop_on_user_mouse: bool,
    /// Stop playback when a physical key is pressed.
    pub stop_on_user_key: bool,
    /// Keep the PC and display awake while playing.
    pub prevent_sleep: bool,
    /// Countdown before playback starts (seconds).
    pub play_countdown_secs: u32,
    /// Warn before playing into a window running as administrator.
    pub warn_elevation: bool,
    /// Send virtual-key codes instead of hardware scan codes. Scan codes are
    /// what games read (the default); virtual keys can help with some remote
    /// desktop tools and compatibility layers.
    pub virtual_key_injection: bool,
}

impl Default for SafetySettings {
    fn default() -> Self {
        Self {
            stop_on_user_mouse: false,
            stop_on_user_key: false,
            prevent_sleep: true,
            play_countdown_secs: 0,
            warn_elevation: true,
            virtual_key_injection: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiSettings {
    pub theme: Theme,
    pub overlay: OverlayCorner,
    /// Minimise the main window while recording / playing.
    pub minimize_while_running: bool,
    /// Compact, always-on-top toolbar mode (like TinyTask).
    pub compact: bool,
    pub always_on_top: bool,
    /// UI zoom factor.
    pub zoom: f32,
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme: Theme::Dark,
            overlay: OverlayCorner::TopRight,
            minimize_while_running: false,
            compact: false,
            always_on_top: false,
            zoom: 1.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub hotkeys: Hotkeys,
    pub recording: RecordingSettings,
    pub safety: SafetySettings,
    pub ui: UiSettings,
    /// Defaults applied to newly recorded macros.
    pub playback_defaults: PlaybackOptions,
    pub clicker: ClickerConfig,
    /// Folder holding the macro library; empty = default location.
    pub library_dir: String,
    pub recent_files: Vec<String>,
}

impl Settings {
    /// Parse settings, falling back to defaults for anything missing or
    /// malformed instead of refusing to start.
    pub fn from_json(s: &str) -> Self {
        serde_json::from_str(s).unwrap_or_default()
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("settings serialise")
    }

    pub fn push_recent(&mut self, path: &str) {
        self.recent_files.retain(|p| !p.eq_ignore_ascii_case(path));
        self.recent_files.insert(0, path.to_string());
        self.recent_files.truncate(10);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_have_no_duplicate_hotkeys_and_are_valid() {
        let h = Hotkeys::default();
        assert!(h.duplicates().is_empty());
        for (_, hk) in h.all() {
            hk.validate().unwrap();
        }
        assert_eq!(h.record.to_string(), "Ctrl+Alt+Shift+R");
    }

    #[test]
    fn roundtrip_and_partial_files() {
        let mut s = Settings::default();
        s.ui.theme = Theme::Light;
        s.push_recent("C:\\a.stmacro");
        assert_eq!(Settings::from_json(&s.to_json()), s);
        // Old/partial file: only one field present.
        let partial = Settings::from_json("{\"ui\":{\"theme\":\"light\"}}");
        assert_eq!(partial.ui.theme, Theme::Light);
        assert_eq!(partial.hotkeys, Hotkeys::default());
        // Garbage falls back to defaults.
        assert_eq!(Settings::from_json("{{{"), Settings::default());
    }

    #[test]
    fn recent_files_dedupe_and_cap() {
        let mut s = Settings::default();
        for i in 0..15 {
            s.push_recent(&format!("f{i}"));
        }
        s.push_recent("F3");
        assert_eq!(s.recent_files.len(), 10);
        assert_eq!(s.recent_files[0], "F3");
        assert_eq!(
            s.recent_files.iter().filter(|p| p.eq_ignore_ascii_case("f3")).count(),
            1
        );
    }

    #[test]
    fn sample_rate_is_clamped() {
        let mut r = RecordingSettings::default();
        assert_eq!(r.motion_interval_us(), 4000);
        r.mouse_sample_hz = 0;
        assert_eq!(r.motion_interval_us(), 1_000_000 / 30);
    }
}
