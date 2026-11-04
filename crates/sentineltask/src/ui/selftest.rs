//! GUI self-test, run in CI on a real Windows machine:
//! `SENTINEL_SELFTEST=<result file> SentinelTask.exe`
//! (optionally `SENTINEL_SELFTEST_ONLY=name1,name2`).
//!
//! Drives the real app the way a user would — hotkeys pressed with real
//! (simulated) keystrokes, the window minimized, settings changed — and
//! checks the results. Every scenario restores what it changed. The result
//! file gets one line per scenario and a final PASS/FAIL; the process exits
//! with 0 (all passed) or 1.

use std::time::{Duration, Instant};

use eframe::egui;
use sentinel_core::keys::{vk, Hotkey};
use sentinel_core::{Event, EventKind, Key, Macro, MouseMode, PlaybackOptions, Repeat};

use super::app::{Document, Mode, SentinelApp, COMPACT_SIZE, FULL_SIZE};
use crate::engine::player::{PauseReason, RunState};
use crate::platform;

type Check = Box<dyn FnMut(&mut SentinelApp, &egui::Context) -> Result<bool, String>>;

/// One step: returns Ok(true) when done, Ok(false) to be called again next
/// tick, Err to fail the scenario. Each step has a timeout.
struct Step {
    what: &'static str,
    timeout: Duration,
    run: Check,
}

struct Scenario {
    name: &'static str,
    steps: Vec<Step>,
}

pub struct SelfTest {
    out: String,
    start: Instant,
    scenarios: Vec<Scenario>,
    current: usize,
    step: usize,
    step_started: Instant,
    report: Vec<String>,
    failed: bool,
}

fn step(
    what: &'static str,
    timeout_ms: u64,
    run: impl FnMut(&mut SentinelApp, &egui::Context) -> Result<bool, String> + 'static,
) -> Step {
    Step {
        what,
        timeout: Duration::from_millis(timeout_ms),
        run: Box::new(run),
    }
}

/// Run an action once.
fn act(what: &'static str, mut f: impl FnMut(&mut SentinelApp, &egui::Context) + 'static) -> Step {
    step(what, 1000, move |app, ctx| {
        f(app, ctx);
        Ok(true)
    })
}

/// Wait a fixed time.
fn wait(ms: u64) -> Step {
    let mut start: Option<Instant> = None;
    step("wait", ms + 2000, move |_, _| {
        let s = *start.get_or_insert_with(Instant::now);
        Ok(s.elapsed() >= Duration::from_millis(ms))
    })
}

/// Wait until a condition holds.
fn until(what: &'static str, timeout_ms: u64, cond: impl Fn(&SentinelApp) -> bool + 'static) -> Step {
    step(what, timeout_ms, move |app, _| Ok(cond(app)))
}

fn check(what: &'static str, cond: impl Fn(&SentinelApp) -> Result<(), String> + 'static) -> Step {
    step(what, 1000, move |app, _| cond(app).map(|_| true))
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

fn press(app_hotkey: fn(&SentinelApp) -> Hotkey) -> impl FnMut(&mut SentinelApp, &egui::Context) {
    move |app, _| platform::simulate_user_keys(&chord(&app_hotkey(app)))
}

fn playing(app: &SentinelApp) -> bool {
    matches!(&app.mode, Mode::Playing { handle } if handle.status().state == RunState::Playing)
}

fn run_state(app: &SentinelApp) -> Option<RunState> {
    match &app.mode {
        Mode::Playing { handle } => Some(handle.status().state),
        _ => None,
    }
}

fn centre() -> (i32, i32) {
    let s = platform::screen_info().primary;
    (s.x + s.w / 2, s.y + s.h / 2)
}

/// A harmless macro: slow mouse moves around the middle of the screen.
pub fn demo_macro() -> Macro {
    let (cx, cy) = centre();
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

fn load(app: &mut SentinelApp, m: Macro) {
    app.doc = Some(Document {
        mac: m,
        path: None,
        dirty: false,
    });
}

fn idle(app: &SentinelApp) -> bool {
    app.mode.is_idle()
}

fn scenarios() -> Vec<Scenario> {
    let f22 = vk::F1 + 21;
    let f23 = vk::F1 + 22;
    let f24 = vk::F24;
    vec![
        Scenario {
            name: "hotkeys_registered",
            steps: vec![check("all hotkeys usable", |app| {
                match app.hotkey_status.iter().find(|r| r.is_broken()) {
                    Some(r) => Err(format!("{:?} not registered: {:?}", r.action, r.error)),
                    None if app.hotkey_status.len() == 5 => Ok(()),
                    None => Err(format!("only {} hotkeys reported", app.hotkey_status.len())),
                }
            })],
        },
        Scenario {
            name: "minimized_play_and_emergency_stop",
            steps: vec![
                act("load + minimize", |app, ctx| {
                    load(app, demo_macro());
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }),
                wait(1500),
                act("press Play hotkey", press(|a| a.settings.hotkeys.play)),
                until("playing while minimized", 3000, playing),
                wait(800),
                check("overlay never takes focus", |_| {
                    let (exists, noact, through, fg) = platform::overlay_window_state();
                    // Wine's X11 window manager focuses every new window; on
                    // real Windows WS_EX_NOACTIVATE forbids it.
                    let fg = fg && !platform::running_under_wine();
                    if !cfg!(windows) || (exists && noact && through && !fg) {
                        Ok(())
                    } else {
                        Err(format!(
                            "overlay exists={exists} noactivate={noact} clickthrough={through} foreground={fg}"
                        ))
                    }
                }),
                act("press emergency stop", press(|a| a.settings.hotkeys.emergency_stop)),
                until("stopped", 3000, idle),
                check("reported as stopped", |app| match &app.last_run {
                    Some(r) if r.starts_with("Stopped") => Ok(()),
                    other => Err(format!("last run {other:?}")),
                }),
            ],
        },
        Scenario {
            name: "pause_resume_freezes_playback",
            steps: vec![
                act("load + play", |app, ctx| {
                    load(app, demo_macro());
                    app.start_playback(ctx, false);
                }),
                until("playing", 3000, playing),
                wait(500),
                act("press Pause hotkey", press(|a| a.settings.hotkeys.pause)),
                until("paused", 2000, |app| {
                    matches!(run_state(app), Some(RunState::Paused(PauseReason::User)))
                }),
                {
                    let mut first: Option<(i32, i32)> = None;
                    let mut since: Option<Instant> = None;
                    step("cursor frozen while paused", 4000, move |_, _| {
                        let p = platform::cursor_pos();
                        let s = *since.get_or_insert_with(Instant::now);
                        let f = *first.get_or_insert(p);
                        if p != f {
                            return Err(format!("cursor moved while paused: {f:?} -> {p:?}"));
                        }
                        Ok(s.elapsed() >= Duration::from_millis(1200))
                    })
                },
                act("press Pause again", press(|a| a.settings.hotkeys.pause)),
                until("resumed", 2000, playing),
                act("press Play hotkey to stop", press(|a| a.settings.hotkeys.play)),
                until("stopped", 3000, idle),
            ],
        },
        Scenario {
            name: "playback_accuracy_and_loops",
            steps: vec![
                act("load 3-pass macro + play", |app, ctx| {
                    let (cx, cy) = centre();
                    let mut m = Macro::new("accuracy");
                    m.events = vec![
                        Event::new(
                            0,
                            EventKind::MouseMove {
                                x: cx - 100,
                                y: cy - 50,
                            },
                        ),
                        Event::new(150_000, EventKind::MouseMove { x: cx + 37, y: cy + 21 }),
                        Event::new(
                            300_000,
                            EventKind::MouseMove {
                                x: cx + 101,
                                y: cy - 77,
                            },
                        ),
                    ];
                    m.playback = PlaybackOptions {
                        repeat: Repeat::Times { count: 3 },
                        ..Default::default()
                    };
                    load(app, m);
                    app.start_playback(ctx, false);
                }),
                until("finished", 6000, idle),
                check("finished all passes on time", |app| match &app.last_run {
                    Some(r) if r.starts_with("Finished — 3 passes") => Ok(()),
                    other => Err(format!("last run {other:?}")),
                }),
                step("cursor at final point", 1500, |_, _| {
                    let (cx, cy) = centre();
                    Ok(platform::cursor_pos() == (cx + 101, cy - 77))
                }),
            ],
        },
        Scenario {
            name: "emergency_stop_releases_held_keys",
            steps: vec![
                act("macro holds F24 then waits", move |app, ctx| {
                    let mut m = Macro::new("hold");
                    m.events = vec![
                        Event::new(
                            0,
                            EventKind::Key {
                                key: Key::from_vk(f24),
                                down: true,
                            },
                        ),
                        Event::new(
                            5_000_000,
                            EventKind::MouseMove {
                                x: centre().0,
                                y: centre().1,
                            },
                        ),
                    ];
                    load(app, m);
                    app.start_playback(ctx, false);
                }),
                until("playing", 3000, playing),
                step("F24 is held", 2000, move |_, _| {
                    Ok(!cfg!(windows) || platform::key_is_down(f24))
                }),
                act("press emergency stop", press(|a| a.settings.hotkeys.emergency_stop)),
                until("stopped", 3000, idle),
                step("F24 released", 2000, move |_, _| Ok(!platform::key_is_down(f24))),
            ],
        },
        Scenario {
            name: "record_hotkey_while_minimized",
            steps: vec![
                act("minimize", |_, ctx| {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true))
                }),
                wait(1200),
                act("press Record hotkey", press(|a| a.settings.hotkeys.record)),
                until("recording", 3000, |app| matches!(app.mode, Mode::Recording { .. })),
                wait(300),
                act("type F22", move |_, _| {
                    platform::simulate_user_keys(&[(f22, true), (f22, false)])
                }),
                wait(300),
                act("press Record hotkey again", press(|a| a.settings.hotkeys.record)),
                until("stopped", 3000, idle),
                step("recorded exactly F22 and auto-saved", 1000, move |app, _| {
                    let doc = app.doc.as_ref().ok_or("no macro")?;
                    let keys: Vec<_> = doc
                        .mac
                        .events
                        .iter()
                        .filter_map(|e| match e.kind {
                            EventKind::Key { key, down } => Some((key.vk, down)),
                            _ => None,
                        })
                        .collect();
                    if keys != vec![(f22, true), (f22, false)] {
                        return Err(format!("recorded keys {keys:?}"));
                    }
                    let path = doc.path.clone().ok_or("not auto-saved")?;
                    if doc.dirty || !path.exists() {
                        return Err("not auto-saved".into());
                    }
                    let _ = std::fs::remove_file(path);
                    Ok(true)
                }),
                act("restore window", |_, ctx| {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false))
                }),
                wait(800),
            ],
        },
        Scenario {
            name: "rebind_record_hotkey_and_use_it",
            steps: vec![
                act("start choosing a new Record key", |app, _| {
                    app.begin_key_capture(crate::platform::HotkeyAction::Record);
                }),
                wait(300),
                act("press Ctrl+F21", |_, _| {
                    platform::simulate_user_keys(&chord(&Hotkey::with(true, false, false, vk::F1 + 20)));
                }),
                until("captured", 3000, |app| app.capturing_hotkey.is_none()),
                check("Record is now Ctrl+F21", |app| {
                    let want = Hotkey::with(true, false, false, vk::F1 + 20);
                    if app.settings.hotkeys.record == want {
                        Ok(())
                    } else {
                        Err(format!("record = {}", app.settings.hotkeys.record))
                    }
                }),
                act("start choosing again, then Esc", |app, _| {
                    app.begin_key_capture(crate::platform::HotkeyAction::Record);
                }),
                wait(300),
                act("press Esc", |_, _| {
                    platform::simulate_user_keys(&[(vk::ESCAPE, true), (vk::ESCAPE, false)])
                }),
                until("cancelled", 3000, |app| app.capturing_hotkey.is_none()),
                check("Esc kept Ctrl+F21", |app| {
                    if app.settings.hotkeys.record == Hotkey::with(true, false, false, vk::F1 + 20) {
                        Ok(())
                    } else {
                        Err(format!("record = {}", app.settings.hotkeys.record))
                    }
                }),
                act("minimize + use the new hotkey", |_, ctx| {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(true));
                }),
                wait(1000),
                act("press Ctrl+F21", press(|a| a.settings.hotkeys.record)),
                until("recording", 3000, |app| matches!(app.mode, Mode::Recording { .. })),
                // Deliberately fast: a quick second press must still count.
                act("press Ctrl+F21 to stop", press(|a| a.settings.hotkeys.record)),
                until("stopped", 3000, idle),
                act("clean up + restore default", |app, ctx| {
                    if let Some(p) = app.doc.as_ref().and_then(|d| d.path.clone()) {
                        let _ = std::fs::remove_file(p);
                    }
                    app.set_hotkey(
                        crate::platform::HotkeyAction::Record,
                        sentinel_core::settings::Hotkeys::default().record,
                    );
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                }),
                wait(800),
            ],
        },
        Scenario {
            name: "stop_when_user_presses_a_key",
            steps: vec![
                act("enable safety stop + play", |app, ctx| {
                    app.settings.safety.stop_on_user_key = true;
                    load(app, demo_macro());
                    app.start_playback(ctx, false);
                }),
                until("playing", 3000, playing),
                wait(600),
                act("user presses F23", move |_, _| {
                    platform::simulate_user_keys(&[(f23, true), (f23, false)])
                }),
                until("stopped", 3000, idle),
                check("stopped because of the key", |app| match &app.last_run {
                    Some(r) if r.contains("a key was pressed") => Ok(()),
                    other => Err(format!("last run {other:?}")),
                }),
                act("restore", |app, _| app.settings.safety.stop_on_user_key = false),
            ],
        },
        Scenario {
            name: "countdown_then_play",
            steps: vec![
                act("2 s countdown + play", |app, ctx| {
                    app.settings.safety.play_countdown_secs = 2;
                    load(app, demo_macro());
                    app.start_playback(ctx, false);
                }),
                until("counting down", 1500, |app| {
                    matches!(run_state(app), Some(RunState::Countdown { .. }))
                }),
                until("playing after countdown", 4000, playing),
                act("stop", |app, _| app.stop_playback()),
                until("stopped", 3000, idle),
                act("restore", |app, _| app.settings.safety.play_countdown_secs = 0),
            ],
        },
        Scenario {
            name: "focus_guard_waits_for_target",
            steps: vec![
                act("target a program that isn't running", |app, ctx| {
                    let mut m = demo_macro();
                    m.playback.focus_guard = true;
                    m.meta.target = Some(sentinel_core::TargetWindow {
                        process: "definitely-not-running.exe".into(),
                        title: String::new(),
                    });
                    load(app, m);
                    app.start_playback(ctx, false);
                }),
                until("waiting for the target window", 3000, |app| {
                    matches!(run_state(app), Some(RunState::Paused(PauseReason::WaitingForWindow(_))))
                }),
                act("stop", |app, _| app.stop_playback()),
                until("stopped", 3000, idle),
            ],
        },
        Scenario {
            name: "relative_mouse_mode",
            steps: vec![
                act("park cursor + play raw deltas", |app, ctx| {
                    let (cx, cy) = centre();
                    let mut inj = platform::Injector::new(MouseMode::Absolute);
                    inj.inject(&EventKind::MouseMove { x: cx, y: cy }, false);
                    drop(inj);
                    let mut m = Macro::new("relative");
                    for i in 0..20u64 {
                        m.events
                            .push(Event::new(i * 10_000, EventKind::MouseDelta { dx: 10, dy: 0 }));
                    }
                    m.playback.mouse_mode = MouseMode::Relative;
                    load(app, m);
                    app.start_playback(ctx, false);
                }),
                until("finished", 4000, idle),
                step("cursor moved right", 1500, |_, _| {
                    let (cx, _) = centre();
                    let (x, _) = platform::cursor_pos();
                    // Pointer acceleration changes the exact distance.
                    Ok(!cfg!(windows) || x > cx + 20)
                }),
            ],
        },
        Scenario {
            name: "auto_clicker_exact_count",
            steps: vec![
                act("press F23 25 times, 20 ms apart", move |app, ctx| {
                    app.settings.clicker = sentinel_core::clicker::ClickerConfig {
                        action: sentinel_core::clicker::ClickAction::Key { key: Hotkey::key(f23) },
                        interval_ms: 20,
                        hold_ms: 5,
                        location: sentinel_core::clicker::ClickLocation::Cursor,
                        limit: sentinel_core::clicker::ClickLimit::Count { clicks: 25 },
                    };
                    app.last_clicker_count = None;
                    app.toggle_clicker(ctx);
                }),
                until("finished", 5000, idle),
                check("exactly 25", |app| {
                    if app.last_clicker_count == Some(25) {
                        Ok(())
                    } else {
                        Err(format!("clicks {:?}", app.last_clicker_count))
                    }
                }),
                act("clicker hotkey starts/stops it", |app, _| {
                    app.settings.clicker.limit = sentinel_core::clicker::ClickLimit::Unlimited;
                    app.settings.clicker.interval_ms = 200;
                    platform::simulate_user_keys(&chord(&app.settings.hotkeys.clicker));
                }),
                until("clicking", 3000, |app| matches!(app.mode, Mode::Clicking { .. })),
                wait(500),
                act("press clicker hotkey again", press(|a| a.settings.hotkeys.clicker)),
                until("stopped", 3000, idle),
                act("restore defaults", |app, _| app.settings.clicker = Default::default()),
            ],
        },
        Scenario {
            name: "compact_mode_resizes",
            steps: vec![
                act("compact on", |app, _| app.settings.ui.compact = true),
                step("window is compact", 5000, |_, ctx| {
                    Ok(ctx.input(|i| i.viewport().inner_rect).is_some_and(|r| {
                        (r.width() - COMPACT_SIZE.x).abs() <= 6.0 && (r.height() - COMPACT_SIZE.y).abs() <= 6.0
                    }))
                }),
                act("compact off", |app, _| app.settings.ui.compact = false),
                step("window is full size", 5000, |_, ctx| {
                    Ok(ctx.input(|i| i.viewport().inner_rect).is_some_and(|r| {
                        (r.width() - FULL_SIZE.x).abs() <= 6.0 && (r.height() - FULL_SIZE.y).abs() <= 6.0
                    }))
                }),
            ],
        },
        Scenario {
            name: "files_save_open_rename_import_export",
            steps: vec![
                step("save, reopen, rename, delete", 3000, |app, _| {
                    let mut m = demo_macro();
                    m.meta.name = "Selftest file".into();
                    load(app, m.clone());
                    if !app.save() {
                        return Err("save failed".into());
                    }
                    let path = app.doc.as_ref().and_then(|d| d.path.clone()).ok_or("no path")?;
                    app.doc = None;
                    app.open_path(&path);
                    let back = app.doc.as_ref().ok_or("reopen failed")?;
                    if back.mac.events != m.events {
                        return Err("events changed after save/open".into());
                    }
                    app.rename_file(&path, "Selftest renamed");
                    let renamed = app
                        .doc
                        .as_ref()
                        .and_then(|d| d.path.clone())
                        .ok_or("rename lost path")?;
                    if !renamed.to_string_lossy().contains("Selftest renamed") || path.exists() {
                        return Err(format!("rename: {}", renamed.display()));
                    }
                    app.delete_file(&renamed);
                    if renamed.exists() {
                        return Err("delete failed".into());
                    }
                    Ok(true)
                }),
                step("import TinyTask .rec", 3000, |app, _| {
                    let mut bytes = Vec::new();
                    bytes.extend_from_slice(&3u32.to_le_bytes());
                    bytes.extend_from_slice(&1u32.to_le_bytes());
                    for (m, l, h, t) in [
                        (0x200u32, 100u32, 200u32, 1000u32),
                        (0x201, 100, 200, 1050),
                        (0x202, 100, 200, 1100),
                    ] {
                        for x in [m, l, h, t, 0] {
                            bytes.extend_from_slice(&x.to_le_bytes());
                        }
                    }
                    let p = std::env::temp_dir().join("sentinel-selftest.rec");
                    std::fs::write(&p, bytes).map_err(|e| e.to_string())?;
                    app.open_path(&p);
                    let _ = std::fs::remove_file(&p);
                    let d = app.doc.as_ref().ok_or("import failed")?;
                    if d.mac.events.len() != 3 || !d.dirty || d.path.is_some() {
                        return Err(format!(
                            "import: {} events dirty={} path={:?}",
                            d.mac.events.len(),
                            d.dirty,
                            d.path
                        ));
                    }
                    Ok(true)
                }),
                step("export standalone player", 10_000, |app, _| {
                    let m = demo_macro();
                    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
                    let out = std::env::temp_dir().join("sentinel-selftest-player.exe");
                    crate::payload::export_player(&exe, &out, &m).map_err(|e| e.to_string())?;
                    let back = crate::payload::read_embedded(&out).ok_or("payload not found")?;
                    let _ = std::fs::remove_file(&out);
                    let _ = app;
                    if back.events != m.events {
                        return Err("payload changed".into());
                    }
                    Ok(true)
                }),
            ],
        },
        Scenario {
            name: "settings_survive_restart",
            steps: vec![step("save + reload settings", 2000, |app, _| {
                let before = app.settings.clone();
                app.settings.ui.theme = sentinel_core::settings::Theme::Light;
                app.settings.recording.countdown_secs = 4;
                app.save_settings_now();
                let text = std::fs::read_to_string(app.paths.settings_file()).map_err(|e| e.to_string())?;
                let loaded = sentinel_core::settings::Settings::from_json(&text);
                app.settings = before;
                app.save_settings_now();
                if loaded.ui.theme != sentinel_core::settings::Theme::Light || loaded.recording.countdown_secs != 4 {
                    return Err("settings not persisted".into());
                }
                Ok(true)
            })],
        },
    ]
}

impl SelfTest {
    pub fn from_env() -> Option<Self> {
        let out = std::env::var("SENTINEL_SELFTEST").ok()?;
        let only: Vec<String> = std::env::var("SENTINEL_SELFTEST_ONLY")
            .ok()
            .map(|s| s.split(',').map(|x| x.trim().to_string()).collect())
            .unwrap_or_default();
        let scenarios = scenarios()
            .into_iter()
            .filter(|s| only.is_empty() || only.iter().any(|o| o == s.name))
            .collect();
        Some(Self {
            out,
            start: Instant::now(),
            scenarios,
            current: 0,
            step: 0,
            step_started: Instant::now(),
            report: Vec::new(),
            failed: false,
        })
    }

    fn write(&self, final_line: &str) {
        let mut text = self.report.join("\n");
        text.push('\n');
        text.push_str(final_line);
        text.push('\n');
        let _ = std::fs::write(&self.out, text);
    }
}

impl SentinelApp {
    pub fn drive_selftest(&mut self, ctx: &egui::Context) {
        let Some(mut t) = self.selftest.take() else { return };
        ctx.request_repaint_after(Duration::from_millis(30));
        // Let the app settle before the first scenario.
        if t.start.elapsed() < Duration::from_secs(3) {
            t.step_started = Instant::now(); // timeouts start after settling
            self.selftest = Some(t);
            return;
        }
        loop {
            let Some(sc) = t.scenarios.get_mut(t.current) else {
                let line = if t.failed { "FAIL" } else { "PASS" };
                t.write(line);
                crate::logging::log("selftest", line);
                platform::emergency_release_all();
                std::process::exit(if t.failed { 1 } else { 0 });
            };
            let name = sc.name;
            let Some(st) = sc.steps.get_mut(t.step) else {
                t.report.push(format!("ok   {name}"));
                t.current += 1;
                t.step = 0;
                t.step_started = Instant::now();
                t.write("RUNNING");
                continue;
            };
            let what = st.what;
            let result = if t.step_started.elapsed() > st.timeout {
                Err(format!("timed out waiting for: {what}"))
            } else {
                (st.run)(self, ctx)
            };
            match result {
                Ok(true) => {
                    t.step += 1;
                    t.step_started = Instant::now();
                    continue; // try the next step in the same tick
                }
                Ok(false) => break,
                Err(e) => {
                    t.failed = true;
                    t.report.push(format!("FAIL {name}: [{what}] {e}"));
                    crate::logging::log("selftest", &format!("FAIL {name}: [{what}] {e}"));
                    // Clean up so the next scenario starts fresh.
                    self.stop_playback();
                    if let Mode::Clicking { handle } = &self.mode {
                        handle.stop();
                    }
                    if matches!(self.mode, Mode::Recording { .. }) {
                        self.stop_recording(ctx, None);
                    }
                    if self.capturing_hotkey.is_some() {
                        self.finish_key_capture(None);
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                    t.current += 1;
                    t.step = 0;
                    t.step_started = Instant::now();
                    t.write("RUNNING");
                    // Give the stop a moment before the next scenario.
                    if let Some(next) = t.scenarios.get_mut(t.current) {
                        next.steps.insert(0, until("previous run ended", 5000, idle));
                    }
                    break;
                }
            }
        }
        if t.start.elapsed() > Duration::from_secs(300) {
            t.report.push("FAIL overall timeout".into());
            t.write("FAIL");
            std::process::exit(1);
        }
        self.selftest = Some(t);
    }
}
