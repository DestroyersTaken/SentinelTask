//! GUI self-test, run in CI on a real Windows machine:
//! `SENTINEL_SELFTEST=<result file> SentinelTask.exe`.
//!
//! Drives the real app the way a user would: minimizes the window, presses
//! the Play hotkey with real (simulated) keystrokes, checks playback runs
//! while minimized, presses the emergency stop and checks it stopped. Writes
//! PASS/FAIL to the result file and exits with 0/1.

use std::time::Instant;

use eframe::egui;
use sentinel_core::keys::{vk, Hotkey};
use sentinel_core::{Event, EventKind, Macro, PlaybackOptions, Repeat};

use super::app::{Document, Mode, SentinelApp};
use crate::platform;

pub struct SelfTest {
    out: String,
    start: Instant,
    stage: u8,
    fg_at_keys: Option<platform::WindowInfo>,
}

impl SelfTest {
    pub fn from_env() -> Option<Self> {
        let out = std::env::var("SENTINEL_SELFTEST").ok()?;
        Some(Self {
            out,
            start: Instant::now(),
            stage: 0,
            fg_at_keys: None,
        })
    }
}

fn chord(h: &Hotkey) -> Vec<(u16, bool)> {
    let mut mods = Vec::new();
    if h.ctrl {
        mods.push(vk::LCONTROL);
    }
    if h.alt {
        mods.push(vk::LMENU);
    }
    if h.shift {
        mods.push(vk::LSHIFT);
    }
    let mut v: Vec<(u16, bool)> = mods.iter().map(|&m| (m, true)).collect();
    v.push((h.vk, true));
    v.push((h.vk, false));
    v.extend(mods.iter().rev().map(|&m| (m, false)));
    v
}

/// A harmless macro: slow mouse moves in the middle of the primary screen.
pub fn demo_macro() -> Macro {
    let s = platform::screen_info().primary;
    let (cx, cy) = (s.x + s.w / 2, s.y + s.h / 2);
    let mut m = Macro::new("Self-test");
    for i in 0..40u64 {
        let a = i as f64 * 0.3;
        m.events.push(Event::new(
            i * 100_000,
            EventKind::MouseMove {
                x: cx + (a.cos() * 80.0) as i32,
                y: cy + (a.sin() * 80.0) as i32,
            },
        ));
    }
    m.playback = PlaybackOptions {
        repeat: Repeat::Forever,
        ..Default::default()
    };
    m
}

fn finish(out: &str, result: Result<(), String>) -> ! {
    let text = match &result {
        Ok(()) => "PASS".to_string(),
        Err(e) => format!("FAIL: {e}"),
    };
    let _ = std::fs::write(out, &text);
    crate::logging::log("selftest", &text);
    platform::emergency_release_all();
    std::process::exit(if result.is_ok() { 0 } else { 1 });
}

impl SentinelApp {
    pub fn drive_selftest(&mut self, ctx: &egui::Context) {
        let Some(t) = self.selftest.as_mut() else { return };
        let secs = t.start.elapsed().as_secs_f32();
        let out = t.out.clone();
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        if secs > 45.0 {
            finish(&out, Err(format!("timed out in stage {}", t.stage)));
        }
        match t.stage {
            0 if secs >= 3.0 => {
                if let Some(r) = self.hotkey_status.iter().find(|r| r.is_broken()) {
                    finish(
                        &out,
                        Err(format!("hotkey {:?} not registered: {:?}", r.action, r.error)),
                    );
                }
                self.doc = Some(Document {
                    mac: demo_macro(),
                    path: None,
                    dirty: false,
                });
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                t.stage = 1;
            }
            1 if secs >= 5.0 => {
                platform::simulate_user_keys(&chord(&self.settings.hotkeys.play));
                t.stage = 2;
            }
            2 if secs >= 8.0 => {
                if !matches!(self.mode, Mode::Playing { .. }) {
                    finish(&out, Err("Play hotkey while minimized did not start playback".into()));
                }
                platform::simulate_user_keys(&chord(&self.settings.hotkeys.emergency_stop));
                t.stage = 3;
            }
            3 if secs >= 10.5 => {
                if !self.mode.is_idle() {
                    finish(&out, Err("emergency stop while minimized did not stop playback".into()));
                }
                match &self.last_run {
                    Some(r) if r.starts_with("Stopped") => {}
                    other => finish(&out, Err(format!("unexpected run result {other:?}"))),
                }
                // Still minimized (the usual way to record): record hotkey.
                // Keys typed into SentinelTask's own window are deliberately
                // never recorded, so the window must not be in front here.
                t.stage = 4;
            }
            4 if secs >= 12.0 => {
                platform::simulate_user_keys(&chord(&self.settings.hotkeys.record));
                t.stage = 5;
            }
            5 if secs >= 14.0 => {
                if !matches!(self.mode, Mode::Recording { .. }) {
                    finish(&out, Err("Record hotkey did not start recording".into()));
                }
                t.fg_at_keys = platform::foreground_window();
                platform::simulate_user_keys(&[(vk::F1 + 21, true), (vk::F1 + 21, false)]); // F22
                platform::simulate_user_keys(&chord(&self.settings.hotkeys.record));
                t.stage = 6;
            }
            6 if secs >= 16.0 => {
                if !self.mode.is_idle() {
                    finish(&out, Err("Record hotkey did not stop recording".into()));
                }
                let Some(doc) = self.doc.as_ref() else {
                    finish(&out, Err("recording produced no macro".into()));
                };
                let keys: Vec<_> = doc
                    .mac
                    .events
                    .iter()
                    .filter_map(|e| match e.kind {
                        EventKind::Key { key, down } => Some((key.vk, down)),
                        _ => None,
                    })
                    .collect();
                if keys != vec![(vk::F1 + 21, true), (vk::F1 + 21, false)] {
                    finish(
                        &out,
                        Err(format!(
                            "recorded keys should be just F22, got {keys:?}; foreground at stop: {:?}; own pid {}",
                            t.fg_at_keys,
                            platform::own_pid()
                        )),
                    );
                }
                if doc.path.is_none() || doc.dirty {
                    finish(&out, Err("recording was not auto-saved".into()));
                }
                let path = doc.path.clone().unwrap();
                let _ = std::fs::remove_file(path);
                finish(&out, Ok(()));
            }
            _ => {}
        }
    }
}
