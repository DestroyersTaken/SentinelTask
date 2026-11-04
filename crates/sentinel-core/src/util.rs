//! Small formatting helpers shared by the UI and CLI.

/// `1h 02m 03s`, `4m 05s`, `12.3s`, `250 ms`.
pub fn format_duration_us(us: u64) -> String {
    let ms = us / 1000;
    if ms < 1000 {
        return format!("{ms} ms");
    }
    let secs = us as f64 / 1_000_000.0;
    if secs < 60.0 {
        return format!("{secs:.1}s");
    }
    format_duration_secs(us / 1_000_000)
}

/// `1h 02m 03s`, `4m 05s`, `7s`.
pub fn format_duration_secs(total: u64) -> String {
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}h {m:02}m {s:02}s")
    } else if m > 0 {
        format!("{m}m {s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Clock-style `HH:MM:SS` or `MM:SS`.
pub fn format_clock_us(us: u64) -> String {
    let total = us / 1_000_000;
    let h = total / 3600;
    let m = (total % 3600) / 60;
    let s = total % 60;
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

pub fn now_unix() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Make a string safe to use as a file name on Windows.
pub fn sanitize_file_name(name: &str) -> String {
    const RESERVED: [&str; 22] = [
        "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "LPT1",
        "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
    ];
    let mut s: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    s = s.trim().trim_end_matches(['.', ' ']).to_string();
    if s.is_empty() {
        s = "Untitled".into();
    }
    if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&s)) {
        s.push('_');
    }
    if s.chars().count() > 120 {
        s = s.chars().take(120).collect();
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(format_duration_us(250_000), "250 ms");
        assert_eq!(format_duration_us(12_340_000), "12.3s");
        assert_eq!(format_duration_us(245_000_000), "4m 05s");
        assert_eq!(format_duration_secs(3723), "1h 02m 03s");
        assert_eq!(format_clock_us(65_000_000), "01:05");
        assert_eq!(format_clock_us(3_725_000_000), "1:02:05");
    }

    #[test]
    fn file_names() {
        assert_eq!(sanitize_file_name("a/b:c*?"), "a_b_c__");
        assert_eq!(sanitize_file_name("  "), "Untitled");
        assert_eq!(sanitize_file_name("con"), "con_");
        assert_eq!(sanitize_file_name("farm..."), "farm");
    }
}
