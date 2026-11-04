//! Standalone player export (TinyTask's "Compile EXE").
//!
//! A standalone player is a copy of SentinelTask.exe with a macro appended:
//! `[exe][macro JSON][u64 length LE][8-byte magic]`. On start-up we look for
//! the trailer; if present the app opens in a minimal player mode. Nothing
//! is unpacked or executed — the JSON is just read back.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use sentinel_core::{format, Macro};

const MAGIC: &[u8; 8] = b"STKMACR1";
const MAX_PAYLOAD: u64 = 256 * 1024 * 1024;

pub fn read_embedded(exe: &Path) -> Option<Macro> {
    let mut f = std::fs::File::open(exe).ok()?;
    let len = f.metadata().ok()?.len();
    if len < 16 {
        return None;
    }
    f.seek(SeekFrom::End(-16)).ok()?;
    let mut trailer = [0u8; 16];
    f.read_exact(&mut trailer).ok()?;
    if &trailer[8..] != MAGIC {
        return None;
    }
    let size = u64::from_le_bytes(trailer[..8].try_into().ok()?);
    if size == 0 || size > MAX_PAYLOAD || size + 16 > len {
        return None;
    }
    f.seek(SeekFrom::End(-16 - size as i64)).ok()?;
    let mut buf = vec![0u8; size as usize];
    f.read_exact(&mut buf).ok()?;
    format::from_str(std::str::from_utf8(&buf).ok()?).ok()
}

/// Strip an existing payload so exporting from a player doesn't nest them.
fn base_exe_bytes(exe: &Path) -> std::io::Result<Vec<u8>> {
    let mut bytes = std::fs::read(exe)?;
    if bytes.len() >= 16 && &bytes[bytes.len() - 8..] == MAGIC {
        let size = u64::from_le_bytes(bytes[bytes.len() - 16..bytes.len() - 8].try_into().unwrap()) as usize;
        if size + 16 <= bytes.len() {
            bytes.truncate(bytes.len() - 16 - size);
        }
    }
    Ok(bytes)
}

pub fn export_player(source_exe: &Path, out: &Path, m: &Macro) -> std::io::Result<()> {
    let mut bytes = base_exe_bytes(source_exe)?;
    let json = format::to_string(m);
    bytes.extend_from_slice(json.as_bytes());
    bytes.extend_from_slice(&(json.len() as u64).to_le_bytes());
    bytes.extend_from_slice(MAGIC);
    crate::paths::write_atomic(out, &bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sentinel_core::{Event, EventKind};

    #[test]
    fn export_and_read_back_without_nesting() {
        let dir = std::env::temp_dir().join(format!("stk-pay-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("fake.exe");
        std::fs::write(&exe, b"MZ fake executable body").unwrap();
        assert!(read_embedded(&exe).is_none());
        let mut m = Macro::new("Embedded");
        m.events.push(Event::new(7, EventKind::MouseMove { x: 3, y: 4 }));
        let out1 = dir.join("p1.exe");
        export_player(&exe, &out1, &m).unwrap();
        assert_eq!(read_embedded(&out1).unwrap(), m);
        // Export again from the player: payload replaced, not stacked.
        let mut m2 = m.clone();
        m2.meta.name = "Second".into();
        let out2 = dir.join("p2.exe");
        export_player(&out1, &out2, &m2).unwrap();
        assert_eq!(read_embedded(&out2).unwrap(), m2);
        let base_len = std::fs::metadata(&exe).unwrap().len();
        let json_len = format::to_string(&m2).len() as u64;
        assert_eq!(std::fs::metadata(&out2).unwrap().len(), base_len + json_len + 16);
        let _ = std::fs::remove_dir_all(dir);
    }
}
