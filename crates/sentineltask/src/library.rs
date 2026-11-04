//! The macro library: a folder of `.stmacro` files.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sentinel_core::format::{self, FILE_EXTENSION};
use sentinel_core::model::MacroStats;
use sentinel_core::Macro;

use crate::paths::write_atomic;

#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub path: PathBuf,
    pub name: String,
    pub description: String,
    pub stats: MacroStats,
    pub modified: SystemTime,
    pub error: Option<String>,
}

#[derive(Default)]
pub struct Library {
    pub dir: PathBuf,
    pub entries: Vec<LibraryEntry>,
    cache: HashMap<PathBuf, (SystemTime, u64, LibraryEntry)>,
}

impl Library {
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            ..Default::default()
        }
    }

    pub fn set_dir(&mut self, dir: PathBuf) {
        if dir != self.dir {
            self.dir = dir;
            self.cache.clear();
            self.refresh();
        }
    }

    /// Rescan the folder; unchanged files are served from cache.
    pub fn refresh(&mut self) {
        let mut out = Vec::new();
        if let Ok(rd) = std::fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let path = e.path();
                if path
                    .extension()
                    .and_then(|x| x.to_str())
                    .is_none_or(|x| !x.eq_ignore_ascii_case(FILE_EXTENSION))
                {
                    continue;
                }
                let Ok(meta) = e.metadata() else { continue };
                let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                let len = meta.len();
                if let Some((m, l, entry)) = self.cache.get(&path) {
                    if *m == modified && *l == len {
                        out.push(entry.clone());
                        continue;
                    }
                }
                let entry = match load_macro(&path) {
                    Ok(m) => LibraryEntry {
                        path: path.clone(),
                        name: if m.meta.name.is_empty() {
                            file_stem(&path)
                        } else {
                            m.meta.name.clone()
                        },
                        description: m.meta.description.clone(),
                        stats: m.stats(),
                        modified,
                        error: None,
                    },
                    Err(err) => LibraryEntry {
                        path: path.clone(),
                        name: file_stem(&path),
                        description: String::new(),
                        stats: MacroStats::default(),
                        modified,
                        error: Some(err),
                    },
                };
                self.cache.insert(path, (modified, len, entry.clone()));
                out.push(entry);
            }
        }
        out.sort_by_key(|e| std::cmp::Reverse(e.modified));
        self.entries = out;
    }

    /// A path in the library for a new macro called `name` that doesn't
    /// overwrite anything.
    pub fn unique_path(&self, name: &str) -> PathBuf {
        let base = sentinel_core::util::sanitize_file_name(name);
        let mut p = self.dir.join(format!("{base}.{FILE_EXTENSION}"));
        let mut n = 2;
        while p.exists() {
            p = self.dir.join(format!("{base} ({n}).{FILE_EXTENSION}"));
            n += 1;
        }
        p
    }
}

pub fn file_stem(p: &Path) -> String {
    p.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Load a `.stmacro` or TinyTask `.rec` file.
pub fn load_macro(path: &Path) -> Result<Macro, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("Couldn't read the file: {e}"))?;
    let is_rec = path
        .extension()
        .and_then(|x| x.to_str())
        .is_some_and(|x| x.eq_ignore_ascii_case("rec"));
    if is_rec || !format::looks_like_macro(&bytes) {
        if let Ok(m) = sentinel_core::rec::import(&bytes, &file_stem(path)) {
            return Ok(m);
        }
        if is_rec {
            return Err("This doesn't look like a TinyTask .rec recording.".into());
        }
    }
    let s = String::from_utf8(bytes).map_err(|_| "This file isn't a SentinelTask macro.".to_string())?;
    format::from_str(&s).map_err(|e| e.to_string())
}

pub fn save_macro(path: &Path, m: &Macro) -> Result<(), String> {
    write_atomic(path, format::to_string(m).as_bytes()).map_err(|e| format!("Couldn't save: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_core::{Event, EventKind};

    #[test]
    fn save_scan_and_unique_names() {
        let dir = std::env::temp_dir().join(format!("stk-lib-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut lib = Library::new(dir.clone());
        let mut m = Macro::new("Farm");
        m.events.push(Event::new(5, EventKind::MouseMove { x: 1, y: 1 }));
        let p1 = lib.unique_path("Farm");
        save_macro(&p1, &m).unwrap();
        let p2 = lib.unique_path("Farm");
        assert_ne!(p1, p2);
        assert!(p2.to_string_lossy().contains("Farm (2)"));
        std::fs::write(dir.join("broken.stmacro"), "{").unwrap();
        lib.refresh();
        assert_eq!(lib.entries.len(), 2);
        assert!(lib.entries.iter().any(|e| e.name == "Farm" && e.stats.events == 1));
        assert!(lib.entries.iter().any(|e| e.error.is_some()));
        assert_eq!(load_macro(&p1).unwrap(), m);
        let _ = std::fs::remove_dir_all(dir);
    }
}
