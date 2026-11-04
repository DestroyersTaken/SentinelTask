//! Where settings, macros and logs live.
//!
//! *Installed* mode uses `%APPDATA%\SentinelTask` for settings,
//! `Documents\SentinelTask` for the macro library and
//! `%LOCALAPPDATA%\SentinelTask\logs` for logs.
//!
//! *Portable* mode (a file named `portable` or `portable.txt` next to the
//! executable, or a `settings.json` already there) keeps everything beside
//! the executable — handy on a USB stick, like TinyTask.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct Paths {
    pub portable: bool,
    pub config_dir: PathBuf,
    pub default_library: PathBuf,
    pub log_dir: PathBuf,
}

impl Paths {
    pub fn detect() -> Self {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
            .unwrap_or_else(|| PathBuf::from("."));
        let portable = ["portable", "portable.txt", "settings.json"]
            .iter()
            .any(|f| exe_dir.join(f).is_file());
        if portable {
            return Self {
                portable,
                config_dir: exe_dir.clone(),
                default_library: exe_dir.join("Macros"),
                log_dir: exe_dir.join("logs"),
            };
        }
        let config_dir = dirs::config_dir()
            .unwrap_or_else(|| exe_dir.clone())
            .join("SentinelTask");
        let default_library = dirs::document_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| exe_dir.clone())
            .join("SentinelTask");
        let log_dir = dirs::data_local_dir()
            .unwrap_or_else(|| config_dir.clone())
            .join("SentinelTask")
            .join("logs");
        Self {
            portable,
            config_dir,
            default_library,
            log_dir,
        }
    }

    pub fn settings_file(&self) -> PathBuf {
        self.config_dir.join("settings.json")
    }

    pub fn library_dir(&self, configured: &str) -> PathBuf {
        if configured.trim().is_empty() {
            self.default_library.clone()
        } else {
            PathBuf::from(configured)
        }
    }
}

/// Write a file atomically (write to a temp file, then rename) so a crash
/// or power loss never leaves a half-written macro or settings file.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp~");
    std::fs::write(&tmp, data)?;
    match std::fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = std::fs::remove_file(&tmp);
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_write_replaces_content() {
        let dir = std::env::temp_dir().join(format!("stk-test-{}", std::process::id()));
        let f = dir.join("x.json");
        write_atomic(&f, b"one").unwrap();
        write_atomic(&f, b"two").unwrap();
        assert_eq!(std::fs::read(&f).unwrap(), b"two");
        assert!(!f.with_extension("tmp~").exists());
        let _ = std::fs::remove_dir_all(dir);
    }
}
