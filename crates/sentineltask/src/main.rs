//! SentinelTask — a modern, reliable mouse & keyboard macro recorder.
//! See docs/RESEARCH.md for why it is built the way it is.

// No console window for the GUI in release builds; the CLI re-attaches to
// the parent console when it is used.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod cli;
mod engine;
mod library;
mod logging;
mod paths;
mod payload;
mod platform;
mod ui;
mod update;

use std::process::ExitCode;
use std::sync::atomic::{AtomicIsize, Ordering};

/// Counts live heap bytes so long-run memory can be checked
/// (`SENTINEL_MEMLOG=1` logs it once a minute). Negligible overhead.
struct CountingAlloc;

static LIVE_BYTES: AtomicIsize = AtomicIsize::new(0);

unsafe impl std::alloc::GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: std::alloc::Layout) -> *mut u8 {
        let p = unsafe { std::alloc::System.alloc(layout) };
        if !p.is_null() {
            LIVE_BYTES.fetch_add(layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: std::alloc::Layout) {
        unsafe { std::alloc::System.dealloc(ptr, layout) };
        LIVE_BYTES.fetch_sub(layout.size() as isize, Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: std::alloc::Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { std::alloc::System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE_BYTES.fetch_add(new_size as isize - layout.size() as isize, Ordering::Relaxed);
        }
        p
    }
}

#[global_allocator]
static ALLOC: CountingAlloc = CountingAlloc;

/// Live heap bytes allocated by SentinelTask's own code (Rust allocations).
pub fn live_heap_bytes() -> isize {
    LIVE_BYTES.load(Ordering::Relaxed)
}

fn start_memlog() {
    if std::env::var("SENTINEL_MEMLOG").is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("sentinel-memlog".into())
        .spawn(|| loop {
            logging::log("mem", &format!("live heap {} KB", live_heap_bytes() / 1024));
            std::thread::sleep(std::time::Duration::from_secs(60));
        })
        .ok();
}

use eframe::egui;

fn install_panic_hook() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Never leave keys or buttons held down (RESEARCH §1 #5).
        platform::emergency_release_all();
        let msg = format!("{info}");
        logging::log("panic", &msg);
        default(info);
    }));
}

fn run_gui(launch: ui::app::LaunchOptions) -> ExitCode {
    let title = match &launch.embedded {
        Some(m) => format!("{} — SentinelTask player", m.meta.name),
        None => "SentinelTask".to_string(),
    };
    let icon = egui::IconData {
        rgba: ui::icon_rgba(64),
        width: 64,
        height: 64,
    };
    // Open the window at the right size straight away (compact mode), rather
    // than resizing it after it appears.
    let saved = std::fs::read_to_string(paths::Paths::detect().settings_file())
        .map(|s| sentinel_core::settings::Settings::from_json(&s))
        .unwrap_or_default();
    let start_compact = saved.ui.compact;
    let start_on_top = saved.ui.always_on_top;
    let make_options = |renderer: eframe::Renderer| eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_app_id("SentinelTask")
            .with_inner_size(if start_compact {
                ui::app::COMPACT_SIZE
            } else {
                ui::app::FULL_SIZE
            })
            .with_min_inner_size([540.0, 92.0])
            .with_resizable(!start_compact)
            .with_window_level(if start_compact || start_on_top {
                egui::WindowLevel::AlwaysOnTop
            } else {
                egui::WindowLevel::Normal
            })
            .with_icon(icon.clone())
            .with_drag_and_drop(true),
        renderer,
        centered: true,
        ..Default::default()
    };
    let renderers: Vec<eframe::Renderer> = match std::env::var("SENTINEL_RENDERER").ok().as_deref() {
        Some("glow") => vec![eframe::Renderer::Glow],
        Some("wgpu") => vec![eframe::Renderer::Wgpu],
        // Default: GPU-accelerated wgpu (DirectX 12 / Vulkan), falling back
        // to OpenGL on old drivers and virtual machines.
        _ => vec![eframe::Renderer::Wgpu, eframe::Renderer::Glow],
    };
    let mut launch = Some(launch);
    let mut last_err = String::new();
    for r in renderers {
        let Some(l) = launch.take() else { break };
        let slot = std::rc::Rc::new(std::cell::RefCell::new(Some(l)));
        let slot2 = slot.clone();
        let result = eframe::run_native(
            "SentinelTask",
            make_options(r),
            Box::new(move |cc| {
                let l = slot2.borrow_mut().take().unwrap_or_default();
                Ok(Box::new(ui::app::SentinelApp::new(cc, l)))
            }),
        );
        match result {
            Ok(()) => return ExitCode::SUCCESS,
            Err(e) => {
                last_err = e.to_string();
                logging::log("warn", &format!("renderer {r:?} failed: {last_err}"));
                // Retry with the next renderer if the app never started.
                launch = slot.borrow_mut().take();
                if launch.is_none() {
                    break;
                }
            }
        }
    }
    if let Ok(out) = std::env::var("SENTINEL_SELFTEST") {
        let _ = std::fs::write(out, format!("FAIL: could not open a window: {last_err}"));
    } else {
        platform::show_error_box(
            "SentinelTask",
            &format!("SentinelTask could not open its window:\n\n{last_err}"),
        );
    }
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    // Started by an update: let the previous copy finish exiting first (it
    // still holds the hotkeys and the files being replaced).
    if let Some(pid) = std::env::var(update::AFTER_UPDATE_ENV)
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
    {
        std::env::remove_var(update::AFTER_UPDATE_ENV);
        platform::wait_for_process_exit(pid, std::time::Duration::from_secs(20));
    }
    platform::init_process();
    let paths = paths::Paths::detect();
    logging::init(&paths.log_dir);
    install_panic_hook();
    start_memlog();
    log_info!("SentinelTask {} starting", sentinel_core::APP_VERSION);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let embedded = std::env::current_exe().ok().and_then(|p| payload::read_embedded(&p));

    let cmd = match cli::parse(&args) {
        Ok(c) => c,
        Err(e) => {
            platform::attach_parent_console();
            eprintln!("error: {e}\n\n{}", cli::HELP);
            return ExitCode::from(64);
        }
    };
    let code = match cmd {
        cli::Command::Gui { file } => {
            return run_gui(ui::app::LaunchOptions {
                open_file: file,
                embedded,
            })
        }
        cli::Command::Help => {
            platform::attach_parent_console();
            println!("{}", cli::HELP);
            0
        }
        cli::Command::Version => {
            platform::attach_parent_console();
            println!("SentinelTask {}", sentinel_core::APP_VERSION);
            0
        }
        cli::Command::Info(p) => {
            platform::attach_parent_console();
            cli::info(&p)
        }
        cli::Command::Update(mode) => {
            platform::attach_parent_console();
            cli::update(mode, embedded.is_some())
        }
        cli::Command::Convert(a, b) => {
            platform::attach_parent_console();
            cli::convert(&a, &b)
        }
        cli::Command::Record { out, seconds } => {
            platform::attach_parent_console();
            cli::record(&out, seconds)
        }
        cli::Command::Play(pa) => {
            platform::attach_parent_console();
            cli::play(pa)
        }
    };
    ExitCode::from(code as u8)
}
