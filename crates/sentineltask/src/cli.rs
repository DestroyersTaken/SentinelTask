//! Command-line interface, for scripting and Windows Task Scheduler.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use sentinel_core::{MouseMode, Repeat};

use crate::engine::player::{self, FinishReason, PlayRequest, RunState};
use crate::platform::{self, HotkeyAction, PlatformEvent, Service};

pub const HELP: &str = "\
SentinelTask — modern mouse & keyboard macro recorder

USAGE
  SentinelTask.exe                      open the app
  SentinelTask.exe <file>               open a .stmacro or TinyTask .rec file
  SentinelTask.exe play <file> [options]
                                        play a macro without the window
  SentinelTask.exe record <out.stmacro> [--seconds <n>]
                                        record without the window; stop with the
                                        record hotkey, emergency stop or Ctrl+C
  SentinelTask.exe info <file>          show details about a macro
  SentinelTask.exe convert <in.rec> <out.stmacro>
                                        convert a TinyTask recording

PLAY OPTIONS
  --loops <n>         play n times
  --forever           repeat until stopped
  --for <time>        repeat for a duration, e.g. 90s, 45m, 2h
  --speed <x>         speed multiplier, 0.1 – 100 (default: saved in macro)
  --delay <ms>        pause between loops in milliseconds
  --relative          relative mouse mode (3D games)
  --absolute          exact-position mouse mode
  --countdown <s>     wait s seconds before starting

The emergency-stop hotkey (default: Pause) and Ctrl+C stop playback.
Every held key and button is released when playback ends.
";

#[derive(Debug, Clone, PartialEq)]
pub struct PlayArgs {
    pub file: PathBuf,
    pub repeat: Option<Repeat>,
    pub speed: Option<f64>,
    pub delay_ms: Option<u64>,
    pub mouse_mode: Option<MouseMode>,
    pub countdown: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Gui { file: Option<PathBuf> },
    Play(PlayArgs),
    Record { out: PathBuf, seconds: Option<u64> },
    Info(PathBuf),
    Convert(PathBuf, PathBuf),
    Help,
    Version,
}

fn parse_duration(s: &str) -> Option<u64> {
    let s = s.trim().to_ascii_lowercase();
    let (num, mult) = if let Some(n) = s.strip_suffix('h') {
        (n, 3600)
    } else if let Some(n) = s.strip_suffix('m') {
        (n, 60)
    } else if let Some(n) = s.strip_suffix('s') {
        (n, 1)
    } else {
        (s.as_str(), 1)
    };
    let v: f64 = num.trim().parse().ok()?;
    if v <= 0.0 || !v.is_finite() {
        return None;
    }
    Some((v * mult as f64).round() as u64)
}

pub fn parse(args: &[String]) -> Result<Command, String> {
    let Some(first) = args.first() else {
        return Ok(Command::Gui { file: None });
    };
    match first.as_str() {
        "-h" | "--help" | "help" | "/?" => Ok(Command::Help),
        "-V" | "--version" | "version" => Ok(Command::Version),
        "info" => args
            .get(1)
            .map(|f| Command::Info(PathBuf::from(f)))
            .ok_or_else(|| "info: missing <file>".into()),
        "record" => {
            let out = args.get(1).ok_or("record: missing <out.stmacro>")?;
            let seconds = match (args.get(2).map(String::as_str), args.get(3)) {
                (None, _) => None,
                (Some("--seconds"), Some(n)) => Some(n.parse::<u64>().map_err(|_| "--seconds needs a whole number")?),
                (Some(o), _) => return Err(format!("record: unexpected {o}")),
            };
            Ok(Command::Record {
                out: PathBuf::from(out),
                seconds,
            })
        }
        "convert" => match (args.get(1), args.get(2)) {
            (Some(a), Some(b)) => Ok(Command::Convert(PathBuf::from(a), PathBuf::from(b))),
            _ => Err("convert: usage: convert <in.rec> <out.stmacro>".into()),
        },
        "play" => {
            let file = args.get(1).ok_or("play: missing <file>")?;
            let mut pa = PlayArgs {
                file: PathBuf::from(file),
                repeat: None,
                speed: None,
                delay_ms: None,
                mouse_mode: None,
                countdown: 0,
            };
            let mut it = args[2..].iter();
            while let Some(a) = it.next() {
                let mut val = |name: &str| it.next().cloned().ok_or(format!("{name} needs a value"));
                match a.as_str() {
                    "--loops" | "-n" => {
                        let n: u32 = val(a)?.parse().map_err(|_| "--loops needs a whole number")?;
                        if n == 0 {
                            return Err("--loops must be at least 1".into());
                        }
                        pa.repeat = Some(Repeat::Times { count: n });
                    }
                    "--forever" => pa.repeat = Some(Repeat::Forever),
                    "--for" => {
                        let d = parse_duration(&val(a)?).ok_or("--for needs a time like 90s, 45m or 2h")?;
                        pa.repeat = Some(Repeat::Duration { secs: d });
                    }
                    "--speed" => {
                        let s: f64 = val(a)?
                            .trim_end_matches(['x', '×'])
                            .parse()
                            .map_err(|_| "--speed needs a number")?;
                        if !(0.1..=100.0).contains(&s) {
                            return Err("--speed must be between 0.1 and 100".into());
                        }
                        pa.speed = Some(s);
                    }
                    "--delay" => pa.delay_ms = Some(val(a)?.parse().map_err(|_| "--delay needs milliseconds")?),
                    "--relative" => pa.mouse_mode = Some(MouseMode::Relative),
                    "--absolute" => pa.mouse_mode = Some(MouseMode::Absolute),
                    "--countdown" => pa.countdown = val(a)?.parse().map_err(|_| "--countdown needs seconds")?,
                    other => return Err(format!("unknown option {other}")),
                }
            }
            Ok(Command::Play(pa))
        }
        f if !f.starts_with('-') => Ok(Command::Gui {
            file: Some(PathBuf::from(f)),
        }),
        other => Err(format!("unknown option {other} (try --help)")),
    }
}

pub fn info(path: &std::path::Path) -> i32 {
    match crate::library::load_macro(path) {
        Ok(m) => {
            let s = m.stats();
            println!("Name:        {}", m.meta.name);
            if !m.meta.description.is_empty() {
                println!("Description: {}", m.meta.description);
            }
            println!(
                "Length:      {}",
                sentinel_core::util::format_duration_us(s.duration_us)
            );
            println!(
                "Events:      {} ({} clicks, {} key presses, {} moves, {} raw motion)",
                s.events, s.clicks, s.key_presses, s.moves, s.deltas
            );
            if let Some(sc) = m.meta.screen {
                println!("Recorded on: {}x{} desktop, {} DPI", sc.desktop.w, sc.desktop.h, sc.dpi);
            }
            if let Some(t) = &m.meta.target {
                println!("Target:      {}", t.process);
            }
            println!(
                "Playback:    {} at {}x, mouse {:?}",
                m.playback.repeat.describe(),
                m.playback.speed,
                m.playback.mouse_mode
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn convert(input: &std::path::Path, output: &std::path::Path) -> i32 {
    match crate::library::load_macro(input) {
        Ok(m) => match crate::library::save_macro(output, &m) {
            Ok(()) => {
                println!("Wrote {} ({} events)", output.display(), m.events.len());
                0
            }
            Err(e) => {
                eprintln!("error: {e}");
                1
            }
        },
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn record(out: &std::path::Path, seconds: Option<u64>) -> i32 {
    use crate::engine::recorder::RecordingSession;
    let paths = crate::paths::Paths::detect();
    let settings = std::fs::read_to_string(paths.settings_file())
        .map(|s| sentinel_core::settings::Settings::from_json(&s))
        .unwrap_or_default();
    let (service, events, captured) = Service::start(Arc::new(|| {}));
    service.register_hotkeys(&settings.hotkeys);
    platform::install_console_handler();
    let screen = platform::screen_info();
    let start = platform::now_us();
    let mut session = RecordingSession::new(start, &settings.recording);
    service.set_capture(Some(RecordingSession::capture_config(&settings.recording)));
    println!("Recording… press {} (or Ctrl+C) to stop.", settings.hotkeys.record);
    let mut stop_key = None;
    loop {
        while let Ok(c) = captured.try_recv() {
            if session.push(c) && session.target.is_none() {
                session.target = platform::foreground_window()
                    .filter(|w| !w.process.is_empty() && w.pid != platform::own_pid())
                    .map(|w| sentinel_core::TargetWindow {
                        process: w.process,
                        title: w.title,
                    });
            }
        }
        if let Ok(PlatformEvent::Hotkey(a)) = events.try_recv() {
            if matches!(
                a,
                HotkeyAction::Record | HotkeyAction::EmergencyStop | HotkeyAction::Play
            ) {
                stop_key = Some(a.of(&settings.hotkeys));
                break;
            }
        }
        if platform::console_interrupted() {
            break;
        }
        if seconds.is_some_and(|s| platform::now_us() - start >= s * 1_000_000) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    service.set_capture(None);
    std::thread::sleep(Duration::from_millis(30));
    while let Ok(c) = captured.try_recv() {
        session.push(c);
    }
    let (ev, target) = session.finish(stop_key.as_ref());
    let mut m = sentinel_core::Macro::new(crate::library::file_stem(out));
    m.meta.created = sentinel_core::util::now_unix();
    m.meta.modified = m.meta.created;
    m.meta.app_version = sentinel_core::APP_VERSION.into();
    m.meta.screen = Some(screen);
    m.meta.target = target;
    m.meta.source = "Recorded (command line)".into();
    m.playback = settings.playback_defaults;
    m.events = ev;
    m.playback.mouse_mode = m.suggested_mouse_mode();
    match crate::library::save_macro(out, &m) {
        Ok(()) => {
            let s = m.stats();
            println!(
                "Saved {} — {} events ({} clicks, {} key presses), {}.",
                out.display(),
                s.events,
                s.clicks,
                s.key_presses,
                sentinel_core::util::format_duration_us(s.duration_us)
            );
            0
        }
        Err(e) => {
            eprintln!("error: {e}");
            1
        }
    }
}

pub fn play(args: PlayArgs) -> i32 {
    let mut m = match crate::library::load_macro(&args.file) {
        Ok(m) => m,
        Err(e) => {
            eprintln!("error: {e}");
            return 1;
        }
    };
    if let Some(r) = args.repeat {
        m.playback.repeat = r;
    }
    if let Some(s) = args.speed {
        m.playback.speed = s;
    }
    if let Some(d) = args.delay_ms {
        m.playback.loop_delay_ms = d;
    }
    if let Some(mm) = args.mouse_mode {
        m.playback.mouse_mode = mm;
    }
    sentinel_core::normalize::balance(&mut m.events);
    let settings = {
        let paths = crate::paths::Paths::detect();
        std::fs::read_to_string(paths.settings_file())
            .map(|s| sentinel_core::settings::Settings::from_json(&s))
            .unwrap_or_default()
    };
    platform::set_virtual_key_injection(settings.safety.virtual_key_injection);
    let (service, events, _captured) = Service::start(Arc::new(|| {}));
    service.register_hotkeys(&settings.hotkeys);
    platform::install_console_handler();

    println!(
        "Playing “{}” — {} at {}x. Press {} or Ctrl+C to stop.",
        m.meta.name,
        m.playback.repeat.describe(),
        m.playback.speed,
        settings.hotkeys.emergency_stop
    );
    let req = PlayRequest {
        events: Arc::new(m.events.clone()),
        opts: m.playback,
        recorded_screen: m.meta.screen,
        focus_process: m.meta.target.as_ref().map(|t| t.process.clone()),
        countdown_secs: args.countdown,
        keep_awake: settings.safety.prevent_sleep,
        wait_for_modifiers: false,
    };
    let handle = player::start(req, Arc::new(|| {}));
    let stopped = AtomicBool::new(false);
    let mut last_pass = 0;
    while !handle.is_finished() {
        while let Ok(ev) = events.try_recv() {
            if let PlatformEvent::Hotkey(HotkeyAction::EmergencyStop | HotkeyAction::Play) = ev {
                handle.stop();
                stopped.store(true, Ordering::SeqCst);
            } else if let PlatformEvent::Hotkey(HotkeyAction::Pause) = ev {
                handle.toggle_pause();
            }
        }
        if platform::console_interrupted() {
            handle.stop();
        }
        let s = handle.status();
        if s.pass != last_pass && matches!(s.state, RunState::Playing) {
            last_pass = s.pass;
            match s.total_passes {
                Some(t) => println!("  pass {}/{t}", s.pass),
                None => println!("  pass {}", s.pass),
            }
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let s = handle.join();
    platform::emergency_release_all();
    let summary = sentinel_core::util::format_duration_us(s.elapsed_us);
    match s.state {
        RunState::Finished(FinishReason::Completed) => {
            println!("Done: {} pass(es) in {summary}.", s.pass);
            0
        }
        _ => {
            println!("Stopped after {summary}.");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse(&[]).unwrap(), Command::Gui { file: None });
        assert_eq!(
            parse(&a(&["x.stmacro"])).unwrap(),
            Command::Gui {
                file: Some("x.stmacro".into())
            }
        );
        assert_eq!(parse(&a(&["--help"])).unwrap(), Command::Help);
        let Command::Play(p) = parse(&a(&[
            "play",
            "f.stmacro",
            "--for",
            "2h",
            "--speed",
            "2x",
            "--relative",
            "--delay",
            "500",
        ]))
        .unwrap() else {
            panic!()
        };
        assert_eq!(p.repeat, Some(Repeat::Duration { secs: 7200 }));
        assert_eq!(p.speed, Some(2.0));
        assert_eq!(p.mouse_mode, Some(MouseMode::Relative));
        assert_eq!(p.delay_ms, Some(500));
        assert!(parse(&a(&["play"])).is_err());
        assert!(parse(&a(&["play", "f", "--loops", "0"])).is_err());
        assert!(parse(&a(&["play", "f", "--speed", "1000"])).is_err());
        assert!(parse(&a(&["play", "f", "--bogus"])).is_err());
        assert!(parse(&a(&["--bogus"])).is_err());
    }

    #[test]
    fn durations() {
        assert_eq!(parse_duration("90s"), Some(90));
        assert_eq!(parse_duration("45m"), Some(2700));
        assert_eq!(parse_duration("1.5h"), Some(5400));
        assert_eq!(parse_duration("30"), Some(30));
        assert_eq!(parse_duration("-1m"), None);
        assert_eq!(parse_duration("abc"), None);
    }
}
