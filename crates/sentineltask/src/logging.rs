//! Minimal file logger: one rolling log in the log folder, so problems
//! reported by users can be diagnosed.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use parking_lot::Mutex;

static LOG: OnceLock<Mutex<Option<std::fs::File>>> = OnceLock::new();
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("sentineltask.log");
    // Keep the log small: start over when it passes 2 MB.
    if std::fs::metadata(&path).map(|m| m.len() > 2_000_000).unwrap_or(false) {
        let _ = std::fs::rename(&path, dir.join("sentineltask.old.log"));
    }
    let file = std::fs::OpenOptions::new().create(true).append(true).open(&path).ok();
    let _ = LOG.set(Mutex::new(file));
    let _ = LOG_PATH.set(path);
}

pub fn path() -> Option<&'static PathBuf> {
    LOG_PATH.get()
}

pub fn log(level: &str, msg: &str) {
    let line = format!("{} [{level}] {msg}\n", crate::platform::local_timestamp());
    if let Some(m) = LOG.get() {
        if let Some(f) = m.lock().as_mut() {
            let _ = f.write_all(line.as_bytes());
        }
    }
    #[cfg(debug_assertions)]
    eprint!("{line}");
}

#[macro_export]
macro_rules! log_info {
    ($($t:tt)*) => { $crate::logging::log("info", &format!($($t)*)) };
}

#[macro_export]
macro_rules! log_warn {
    ($($t:tt)*) => { $crate::logging::log("warn", &format!($($t)*)) };
}
