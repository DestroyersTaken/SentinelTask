//! Self-update from GitHub releases.
//!
//! * **Installed** copies (the folder has `Uninstall SentinelTask.exe`)
//!   download the new `…-setup.exe` (or `…-installer.zip`) in the
//!   background and run it silently (`/S /UPDATE`) when SentinelTask closes,
//!   or straight away on "Restart now". The installer waits for SentinelTask
//!   to exit, keeps the shortcuts and file associations the user chose, and
//!   can start the new version (`/RELAUNCH`).
//! * **Portable** copies ask first. On "Replace", the new `…-portable.zip`
//!   is downloaded, its `SentinelTask.exe` is written next to this one as
//!   `SentinelTask.exe.new`, the running exe is renamed to `.old` (Windows
//!   allows renaming a running program, not overwriting it), the new one
//!   takes its name, and it is started. The `.old` file is deleted by the
//!   new copy. Settings and macros in the folder are untouched.
//!
//! Every download is checked against the SHA-256 digest GitHub publishes for
//! release assets (or a `SHA256SUMS.txt` asset), its size, and that it is a
//! Windows program, before anything is replaced. Exported standalone players
//! never update themselves.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::platform;
use crate::{log_info, log_warn};

pub const LATEST_RELEASE_API: &str = "https://api.github.com/repos/DestroyersTaken/SentinelTask/releases/latest";
pub const RELEASES_PAGE: &str = "https://github.com/DestroyersTaken/SentinelTask/releases/latest";

const MAX_JSON: u64 = 4 * 1024 * 1024;
const MAX_DOWNLOAD: u64 = 200 * 1024 * 1024;
/// Smallest plausible SentinelTask.exe / setup.exe.
const MIN_PROGRAM: usize = 512 * 1024;
/// Environment variable telling a freshly started copy to wait for the
/// previous one (which still holds the hotkeys) to exit.
pub const AFTER_UPDATE_ENV: &str = "SENTINEL_AFTER_UPDATE";
const AUTO_FIRST_CHECK: Duration = Duration::from_secs(8);
const AUTO_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// `SENTINEL_UPDATE_API` points the updater at another release feed (tests).
/// Only then are `file://` and plain `http://` URLs allowed.
fn test_feed() -> Option<String> {
    std::env::var("SENTINEL_UPDATE_API")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

fn api_url() -> String {
    test_feed().unwrap_or_else(|| LATEST_RELEASE_API.to_string())
}

/// The running version (`SENTINEL_VERSION_OVERRIDE` pretends to be another,
/// for testing).
pub fn current_version() -> Version {
    std::env::var("SENTINEL_VERSION_OVERRIDE")
        .ok()
        .and_then(|v| Version::parse(&v))
        .or_else(|| Version::parse(sentinel_core::APP_VERSION))
        .unwrap_or(Version(0, 0, 0))
}

// ---------------------------------------------------------------------------
// Versions, URLs, release metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Version(pub u32, pub u32, pub u32);

impl Version {
    /// "1.2.3", "v1.2", "SentinelTask-1.2.3", "1.2.3-beta" → numbers.
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        let start = s.find(|c: char| c.is_ascii_digit())?;
        let rest = &s[start..];
        let end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        let nums: Vec<&str> = rest[..end].trim_end_matches('.').split('.').collect();
        if nums.is_empty() || nums.len() > 4 || nums.iter().any(|n| n.is_empty()) {
            return None;
        }
        let n = |i: usize| -> Option<u32> { nums.get(i).map_or(Some(0), |v| v.parse().ok()) };
        Some(Version(n(0)?, n(1)?, n(2)?))
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpUrl {
    pub secure: bool,
    pub host: String,
    pub port: u16,
    /// Path and query, starting with '/'.
    pub path: String,
}

#[cfg_attr(not(windows), allow(dead_code))]
impl HttpUrl {
    pub fn parse(url: &str) -> Result<Self, String> {
        let (secure, rest) = if let Some(r) = url.strip_prefix("https://") {
            (true, r)
        } else if let Some(r) = url.strip_prefix("http://") {
            (false, r)
        } else {
            return Err(format!("unsupported address: {url}"));
        };
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        let path = path.split('#').next().unwrap_or("/");
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) if !h.is_empty() => (h, p.parse::<u16>().map_err(|_| format!("bad port in {url}"))?),
            _ => (authority, if secure { 443 } else { 80 }),
        };
        if host.is_empty() || host.contains('@') {
            return Err(format!("bad address: {url}"));
        }
        Ok(Self {
            secure,
            host: host.to_string(),
            port,
            path: if path.is_empty() { "/".into() } else { path.to_string() },
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
    /// Lower-case hex SHA-256, when GitHub published one.
    pub sha256: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    pub tag: String,
    pub notes: String,
    pub page_url: String,
    pub assets: Vec<Asset>,
}

#[derive(Deserialize)]
struct GhRelease {
    tag_name: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    html_url: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GhAsset>,
}

#[derive(Deserialize)]
struct GhAsset {
    name: String,
    #[serde(default)]
    size: u64,
    browser_download_url: String,
    #[serde(default)]
    digest: Option<String>,
    #[serde(default)]
    state: Option<String>,
}

pub fn parse_release(json: &str) -> Result<Release, String> {
    let r: GhRelease = serde_json::from_str(json).map_err(|e| format!("unexpected answer from GitHub ({e})"))?;
    if r.draft || r.prerelease {
        return Err("the latest release is not a stable release".into());
    }
    let version = Version::parse(&r.tag_name)
        .or_else(|| r.name.as_deref().and_then(Version::parse))
        .ok_or_else(|| format!("can't read the version of release “{}”", r.tag_name))?;
    let assets = r
        .assets
        .into_iter()
        .filter(|a| a.state.as_deref().is_none_or(|s| s == "uploaded"))
        .map(|a| Asset {
            sha256: a
                .digest
                .as_deref()
                .and_then(|d| d.strip_prefix("sha256:"))
                .map(|h| h.trim().to_ascii_lowercase())
                .filter(|h| h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit())),
            name: a.name,
            url: a.browser_download_url,
            size: a.size,
        })
        .collect();
    Ok(Release {
        version,
        tag: r.tag_name,
        notes: r.body.unwrap_or_default().replace("\r\n", "\n"),
        page_url: r.html_url.unwrap_or_else(|| RELEASES_PAGE.to_string()),
        assets,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallKind {
    /// Installed with the setup program.
    Installed,
    /// Portable zip, or a loose SentinelTask.exe.
    Portable,
}

impl InstallKind {
    pub fn detect(exe: &Path, portable: bool) -> Self {
        let installed = !portable
            && exe
                .parent()
                .is_some_and(|d| d.join("Uninstall SentinelTask.exe").is_file());
        if installed {
            InstallKind::Installed
        } else {
            InstallKind::Portable
        }
    }
}

/// Which download fits this copy.
pub fn pick_asset(r: &Release, kind: InstallKind) -> Option<&Asset> {
    let named = |suffix: &str| {
        r.assets.iter().find(|a| {
            let n = a.name.to_ascii_lowercase();
            n.starts_with("sentineltask") && n.ends_with(suffix)
        })
    };
    match kind {
        InstallKind::Installed => named("-setup.exe").or_else(|| named("-installer.zip")),
        InstallKind::Portable => named("-portable.zip"),
    }
}

// ---------------------------------------------------------------------------
// Downloading and checking
// ---------------------------------------------------------------------------

fn fetch(
    url: &str,
    accept: &str,
    max: u64,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<Vec<u8>, String> {
    let testing = test_feed().is_some();
    if let Some(path) = url.strip_prefix("file://") {
        if !testing {
            return Err("refusing a local file outside of tests".into());
        }
        let data = std::fs::read(path).map_err(|e| format!("{path}: {e}"))?;
        if !progress(data.len() as u64, Some(data.len() as u64)) {
            return Err("cancelled".into());
        }
        return Ok(data);
    }
    if url.starts_with("http://") && !testing {
        return Err("refusing an insecure (http) download".into());
    }
    platform::http_get(url, accept, max, progress)
}

pub fn sha256_hex(data: &[u8]) -> String {
    let d = Sha256::digest(data);
    d.iter().map(|b| format!("{b:02x}")).collect()
}

/// Find `name`'s hash in a `sha256sum`-style list.
fn hash_from_sums(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let hash = it.next()?;
        let file = it.next()?.trim_start_matches('*').trim_start_matches("./");
        (file.eq_ignore_ascii_case(name) && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

pub fn verify(asset: &Asset, data: &[u8], sums: Option<&str>) -> Result<(), String> {
    if asset.size > 0 && data.len() as u64 != asset.size {
        return Err(format!(
            "the download is incomplete ({} of {} bytes)",
            data.len(),
            asset.size
        ));
    }
    let expected = asset
        .sha256
        .clone()
        .or_else(|| sums.and_then(|s| hash_from_sums(s, &asset.name)));
    match expected {
        Some(want) if want != sha256_hex(data) => {
            Err("the download is damaged (its SHA-256 checksum doesn't match); nothing was changed".into())
        }
        Some(_) => Ok(()),
        None => {
            log_warn!("update: no checksum published for {}; relying on HTTPS", asset.name);
            Ok(())
        }
    }
}

fn check_program(data: &[u8], what: &str) -> Result<(), String> {
    if data.len() < MIN_PROGRAM || &data[..2] != b"MZ" {
        return Err(format!("{what} in the download is not a valid Windows program"));
    }
    Ok(())
}

fn le16(d: &[u8], at: usize) -> Option<usize> {
    d.get(at..at + 2).map(|b| u16::from_le_bytes([b[0], b[1]]) as usize)
}

fn le32(d: &[u8], at: usize) -> Option<u32> {
    d.get(at..at + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Extract the first file whose name (without folders) satisfies `want`
/// from a zip archive. Supports stored and deflated entries and checks the
/// CRC-32. Returns the entry's full name and its bytes.
pub fn zip_extract(data: &[u8], want: &dyn Fn(&str) -> bool) -> Result<(String, Vec<u8>), String> {
    use std::io::Read;
    let bad = || "the downloaded zip is damaged".to_string();
    // End of central directory: the last 22 bytes, plus up to 64 KB comment.
    let lo = data.len().saturating_sub(22 + 65_535);
    let eocd = (lo..=data.len().saturating_sub(22))
        .rev()
        .find(|&i| le32(data, i) == Some(0x0605_4b50))
        .ok_or_else(bad)?;
    let entries = le16(data, eocd + 10).ok_or_else(bad)?;
    let mut p = le32(data, eocd + 16).ok_or_else(bad)? as usize;
    for _ in 0..entries {
        if le32(data, p) != Some(0x0201_4b50) {
            return Err(bad());
        }
        let flags = le16(data, p + 8).ok_or_else(bad)?;
        let method = le16(data, p + 10).ok_or_else(bad)?;
        let crc = le32(data, p + 16).ok_or_else(bad)?;
        let csize = le32(data, p + 20).ok_or_else(bad)? as usize;
        let usize_ = le32(data, p + 24).ok_or_else(bad)? as usize;
        let nlen = le16(data, p + 28).ok_or_else(bad)?;
        let xlen = le16(data, p + 30).ok_or_else(bad)?;
        let clen = le16(data, p + 32).ok_or_else(bad)?;
        let local = le32(data, p + 42).ok_or_else(bad)? as usize;
        let name_bytes = data.get(p + 46..p + 46 + nlen).ok_or_else(bad)?;
        let name = String::from_utf8_lossy(name_bytes).replace('\\', "/");
        p += 46 + nlen + xlen + clen;
        let base = name.rsplit('/').next().unwrap_or(&name);
        if base.is_empty() || !want(base) {
            continue;
        }
        if flags & 1 != 0 {
            return Err("the downloaded zip is encrypted".into());
        }
        if usize_ as u64 > MAX_DOWNLOAD || csize as u64 > MAX_DOWNLOAD {
            return Err(bad());
        }
        if le32(data, local) != Some(0x0403_4b50) {
            return Err(bad());
        }
        let lnlen = le16(data, local + 26).ok_or_else(bad)?;
        let lxlen = le16(data, local + 28).ok_or_else(bad)?;
        let start = local + 30 + lnlen + lxlen;
        let raw = data.get(start..start + csize).ok_or_else(bad)?;
        let out = match method {
            0 => raw.to_vec(),
            8 => {
                let mut out = Vec::with_capacity(usize_);
                flate2::read::DeflateDecoder::new(raw)
                    .take(usize_ as u64 + 1)
                    .read_to_end(&mut out)
                    .map_err(|_| bad())?;
                out
            }
            m => return Err(format!("the downloaded zip uses an unsupported compression ({m})")),
        };
        if out.len() != usize_ || crc32fast::hash(&out) != crc {
            return Err(bad());
        }
        return Ok((name, out));
    }
    Err("the download doesn't contain SentinelTask".into())
}

/// `dir/SentinelTask.exe` + ".new" → `dir/SentinelTask.exe.new`.
pub fn sibling(exe: &Path, suffix: &str) -> PathBuf {
    let mut name = exe.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(suffix);
    exe.with_file_name(name)
}

fn update_temp_dir() -> PathBuf {
    std::env::temp_dir().join("SentinelTask-update")
}

/// A downloaded, checked update, ready to apply.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    pub version: Version,
    pub kind: InstallKind,
    /// The setup program (installed) or the new `SentinelTask.exe.new`
    /// (portable).
    pub file: PathBuf,
}

/// Download the right file for `kind`, check it, and stage it.
pub fn download_and_prepare(
    release: &Release,
    kind: InstallKind,
    exe: &Path,
    progress: &mut dyn FnMut(u64, Option<u64>) -> bool,
) -> Result<Prepared, String> {
    let asset = pick_asset(release, kind).ok_or_else(|| {
        format!(
            "release {} has no {} download",
            release.version,
            match kind {
                InstallKind::Installed => "installer",
                InstallKind::Portable => "portable",
            }
        )
    })?;
    let sums = if asset.sha256.is_none() {
        release
            .assets
            .iter()
            .find(|a| a.name.eq_ignore_ascii_case("SHA256SUMS.txt"))
            .and_then(|a| fetch(&a.url, "application/octet-stream", 1024 * 1024, &mut |_, _| true).ok())
            .map(|b| String::from_utf8_lossy(&b).into_owned())
    } else {
        None
    };
    log_info!("update: downloading {} ({} bytes)", asset.name, asset.size);
    let data = fetch(&asset.url, "application/octet-stream", MAX_DOWNLOAD, progress)?;
    verify(asset, &data, sums.as_deref())?;
    match kind {
        InstallKind::Installed => {
            let setup = if asset.name.to_ascii_lowercase().ends_with(".zip") {
                zip_extract(&data, &|n| n.to_ascii_lowercase().ends_with("setup.exe"))?.1
            } else {
                data
            };
            check_program(&setup, "The installer")?;
            let dir = update_temp_dir();
            std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let file = dir.join(format!("SentinelTask-{}-setup.exe", release.version));
            crate::paths::write_atomic(&file, &setup).map_err(|e| format!("{}: {e}", file.display()))?;
            Ok(Prepared {
                version: release.version,
                kind,
                file,
            })
        }
        InstallKind::Portable => {
            let (_, new_exe) = zip_extract(&data, &|n| n.eq_ignore_ascii_case("SentinelTask.exe"))?;
            check_program(&new_exe, "SentinelTask.exe")?;
            let file = sibling(exe, ".new");
            crate::paths::write_atomic(&file, &new_exe).map_err(|e| {
                format!(
                    "can't write to {} ({e}). Move SentinelTask to a folder you can write to, \
                     or download the new version from GitHub",
                    file.parent().map(|d| d.display().to_string()).unwrap_or_default()
                )
            })?;
            Ok(Prepared {
                version: release.version,
                kind,
                file,
            })
        }
    }
}

/// Swap a staged `SentinelTask.exe.new` in for the (running) exe.
pub fn swap_exe(exe: &Path, new: &Path) -> Result<(), String> {
    let mut old = sibling(exe, ".old");
    if old.exists() && std::fs::remove_file(&old).is_err() {
        old = sibling(exe, &format!(".old{}", std::process::id()));
    }
    std::fs::rename(exe, &old).map_err(|e| format!("couldn't move the old SentinelTask.exe aside: {e}"))?;
    if let Err(e) = std::fs::rename(new, exe) {
        let _ = std::fs::rename(&old, exe);
        return Err(format!("couldn't put the new SentinelTask.exe in place: {e}"));
    }
    Ok(())
}

/// Start `program` detached. The new process inherits nothing it needs
/// from us except `SENTINEL_AFTER_UPDATE`, which makes a new SentinelTask
/// wait for this one to exit first.
fn spawn(program: &Path, args: &[&str]) -> Result<(), String> {
    std::process::Command::new(program)
        .args(args)
        .env(AFTER_UPDATE_ENV, std::process::id().to_string())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("couldn't start {}: {e}", program.display()))
}

/// Apply now. On `Ok` the caller must exit promptly: the installer waits
/// for SentinelTask to close; a portable copy has already been replaced.
pub fn apply(p: &Prepared, exe: &Path, restart: bool) -> Result<(), String> {
    match p.kind {
        InstallKind::Installed => {
            let mut args = vec!["/S", "/UPDATE"];
            if restart {
                args.push("/RELAUNCH");
            }
            spawn(&p.file, &args)?;
            log_info!("update: started installer for {}", p.version);
            Ok(())
        }
        InstallKind::Portable => {
            swap_exe(exe, &p.file)?;
            log_info!("update: replaced {} with {}", exe.display(), p.version);
            if restart {
                spawn(exe, &[])?;
            }
            Ok(())
        }
    }
}

/// Clean up what an earlier update left behind (in the background — the
/// old copy may still be exiting).
pub fn cleanup_after_update(exe: &Path) {
    let exe = exe.to_path_buf();
    std::thread::Builder::new()
        .name("sentinel-update-cleanup".into())
        .spawn(move || {
            let Some(dir) = exe.parent() else { return };
            let prefix = format!("{}.old", exe.file_name().unwrap_or_default().to_string_lossy());
            for _ in 0..40 {
                let mut left = false;
                if let Ok(rd) = std::fs::read_dir(dir) {
                    for e in rd.flatten() {
                        if e.file_name().to_string_lossy().starts_with(&prefix) {
                            left |= std::fs::remove_file(e.path()).is_err();
                        }
                    }
                }
                if !left {
                    break;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            let _ = std::fs::remove_file(sibling(&exe, ".new"));
            let _ = std::fs::remove_dir_all(update_temp_dir());
        })
        .ok();
}

/// Ask GitHub for the latest release; `Some` if it is newer than us.
pub fn check_latest() -> Result<Option<Release>, String> {
    let json = fetch(&api_url(), "application/vnd.github+json", MAX_JSON, &mut |_, _| true)?;
    let rel = parse_release(&String::from_utf8_lossy(&json))?;
    let cur = current_version();
    log_info!("update: latest release {} (running {cur})", rel.version);
    Ok((rel.version > cur).then_some(rel))
}

// ---------------------------------------------------------------------------
// Background updater used by the app
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Downloading {
        version: Version,
        done: u64,
        total: Option<u64>,
    },
    Ready(Prepared),
    Failed(String),
}

#[derive(Debug, Clone)]
pub enum UpdateEvent {
    UpToDate { manual: bool },
    Available { release: Release, manual: bool },
    Ready { prepared: Prepared, apply: bool },
    Failed { msg: String, manual: bool },
}

pub struct Updater {
    pub kind: InstallKind,
    pub exe: PathBuf,
    pub enabled: bool,
    phase: Arc<Mutex<Phase>>,
    working: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    tx: Sender<UpdateEvent>,
    pub rx: Receiver<UpdateEvent>,
    started: Instant,
    last_auto: Option<Instant>,
    waker: platform::Waker,
}

impl Updater {
    pub fn new(exe: PathBuf, kind: InstallKind, enabled: bool, waker: platform::Waker) -> Self {
        let (tx, rx) = unbounded();
        Self {
            kind,
            exe,
            enabled,
            phase: Arc::new(Mutex::new(Phase::Idle)),
            working: Arc::new(AtomicBool::new(false)),
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
            started: Instant::now(),
            last_auto: None,
            waker,
        }
    }

    pub fn phase(&self) -> Phase {
        self.phase.lock().clone()
    }

    pub fn is_working(&self) -> bool {
        self.working.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::SeqCst);
    }

    /// Automatic checks: shortly after start-up, then every few hours.
    pub fn tick(&mut self, auto_check: bool, auto_download: bool) {
        if !self.enabled || !auto_check || self.is_working() {
            return;
        }
        let due = match self.last_auto {
            None => self.started.elapsed() >= AUTO_FIRST_CHECK,
            Some(t) => t.elapsed() >= AUTO_INTERVAL,
        };
        // Nothing to do once an update is ready to install.
        if due && !matches!(self.phase(), Phase::Ready(_)) {
            self.last_auto = Some(Instant::now());
            self.check(false, auto_download);
        }
    }

    /// Check now. `auto_download`: installed copies download straight away.
    pub fn check(&mut self, manual: bool, auto_download: bool) {
        if !self.enabled || self.working.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.phase.lock() = Phase::Checking;
        let download = auto_download && self.kind == InstallKind::Installed;
        self.run(move |w| match check_latest() {
            Ok(None) => (Phase::UpToDate, UpdateEvent::UpToDate { manual }),
            Ok(Some(rel)) if download => w.download(&rel, false),
            Ok(Some(rel)) => (
                Phase::Available(rel.clone()),
                UpdateEvent::Available { release: rel, manual },
            ),
            Err(msg) => {
                log_warn!("update check failed: {msg}");
                (Phase::Failed(msg.clone()), UpdateEvent::Failed { msg, manual })
            }
        });
    }

    /// Download `release`; `apply` = install/replace as soon as it's ready.
    pub fn download(&mut self, release: Release, apply: bool) {
        if !self.enabled || self.working.swap(true, Ordering::SeqCst) {
            return;
        }
        self.run(move |w| w.download(&release, apply));
    }

    /// Run `job` on a worker thread. Its final phase is published and the
    /// updater marked idle *before* its event is sent, so whoever reacts to
    /// the event can start the next step straight away.
    fn run(&mut self, job: impl FnOnce(&Worker) -> (Phase, UpdateEvent) + Send + 'static) {
        self.cancel.store(false, Ordering::SeqCst);
        let w = Worker {
            phase: self.phase.clone(),
            kind: self.kind,
            exe: self.exe.clone(),
            cancel: self.cancel.clone(),
            waker: self.waker.clone(),
        };
        let working = self.working.clone();
        let tx = self.tx.clone();
        let waker = self.waker.clone();
        let spawned = std::thread::Builder::new()
            .name("sentinel-update".into())
            .spawn(move || {
                let (phase, event) = job(&w);
                *w.phase.lock() = phase;
                working.store(false, Ordering::SeqCst);
                let _ = tx.send(event);
                waker();
            });
        if spawned.is_err() {
            *self.phase.lock() = Phase::Failed("couldn't start the updater".into());
            self.working.store(false, Ordering::SeqCst);
        }
    }
}

struct Worker {
    phase: Arc<Mutex<Phase>>,
    kind: InstallKind,
    exe: PathBuf,
    cancel: Arc<AtomicBool>,
    waker: platform::Waker,
}

impl Worker {
    fn download(&self, rel: &Release, apply: bool) -> (Phase, UpdateEvent) {
        let version = rel.version;
        *self.phase.lock() = Phase::Downloading {
            version,
            done: 0,
            total: None,
        };
        let mut last_wake = Instant::now();
        let mut progress = |done: u64, total: Option<u64>| {
            *self.phase.lock() = Phase::Downloading { version, done, total };
            if last_wake.elapsed() > Duration::from_millis(200) {
                last_wake = Instant::now();
                (self.waker)();
            }
            !self.cancel.load(Ordering::SeqCst)
        };
        match download_and_prepare(rel, self.kind, &self.exe, &mut progress) {
            Ok(prepared) => {
                log_info!("update: {} ready at {}", version, prepared.file.display());
                (Phase::Ready(prepared.clone()), UpdateEvent::Ready { prepared, apply })
            }
            Err(msg) => {
                log_warn!("update download failed: {msg}");
                (Phase::Failed(msg.clone()), UpdateEvent::Failed { msg, manual: apply })
            }
        }
    }
}

pub fn describe(phase: &Phase) -> String {
    match phase {
        Phase::Idle => "Not checked yet".into(),
        Phase::Checking => "Checking for updates…".into(),
        Phase::UpToDate => format!("You have the latest version ({})", sentinel_core::APP_VERSION),
        Phase::Available(r) => format!("Version {} is available", r.version),
        Phase::Downloading { version, done, total } => match total {
            Some(t) if *t > 0 => format!(
                "Downloading {version}… {:.0} %",
                (*done as f64 / *t as f64 * 100.0).min(100.0)
            ),
            _ => format!("Downloading {version}… {:.1} MB", *done as f64 / 1_048_576.0),
        },
        Phase::Ready(p) => match p.kind {
            InstallKind::Installed => format!("Version {} is ready — it installs when SentinelTask closes", p.version),
            InstallKind::Portable => format!("Version {} is downloaded", p.version),
        },
        Phase::Failed(m) => format!("Couldn't check for updates: {m}"),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::io::Write;

    /// Minimal zip writer (stored or deflated) for tests.
    pub fn make_zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let comp = if *deflate {
                let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
                e.write_all(data).unwrap();
                e.finish().unwrap()
            } else {
                data.to_vec()
            };
            let method: u16 = if *deflate { 8 } else { 0 };
            let crc = crc32fast::hash(data);
            let off = out.len() as u32;
            out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0]);
            out.extend_from_slice(&method.to_le_bytes());
            out.extend_from_slice(&[0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&0u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&comp);
            central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
            central.extend_from_slice(&[20, 0, 20, 0, 0, 0]);
            central.extend_from_slice(&method.to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            central.extend_from_slice(&(comp.len() as u32).to_le_bytes());
            central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&[0; 12]);
            central.extend_from_slice(&off.to_le_bytes());
            central.extend_from_slice(name.as_bytes());
        }
        let cd_off = out.len() as u32;
        out.extend_from_slice(&central);
        out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(files.len() as u16).to_le_bytes());
        out.extend_from_slice(&(central.len() as u32).to_le_bytes());
        out.extend_from_slice(&cd_off.to_le_bytes());
        out.extend_from_slice(&0u16.to_le_bytes());
        out
    }

    pub fn fake_program(tag: u8) -> Vec<u8> {
        let mut v = vec![tag; MIN_PROGRAM + 1000];
        v[0] = b'M';
        v[1] = b'Z';
        v
    }

    #[test]
    fn versions() {
        assert_eq!(Version::parse("1.1.0"), Some(Version(1, 1, 0)));
        assert_eq!(Version::parse("v1.1"), Some(Version(1, 1, 0)));
        assert_eq!(Version::parse("SentinelTask-1.0.0"), Some(Version(1, 0, 0)));
        assert_eq!(Version::parse("2.10.3-beta.1"), Some(Version(2, 10, 3)));
        assert_eq!(Version::parse("10"), Some(Version(10, 0, 0)));
        assert_eq!(Version::parse("latest"), None);
        assert_eq!(Version::parse("1..2"), None);
        assert!(Version(1, 10, 0) > Version(1, 9, 9));
        assert!(Version(1, 1, 0) > Version(1, 0, 0));
        assert!(Version(2, 0, 0) > Version(1, 99, 99));
        assert_eq!(Version(1, 2, 3).to_string(), "1.2.3");
        assert_eq!(
            Version::parse(sentinel_core::APP_VERSION).unwrap().to_string(),
            sentinel_core::APP_VERSION
        );
    }

    #[test]
    fn urls() {
        let u = HttpUrl::parse("https://api.github.com/repos/a/b/releases/latest").unwrap();
        assert_eq!((u.secure, u.host.as_str(), u.port), (true, "api.github.com", 443));
        assert_eq!(u.path, "/repos/a/b/releases/latest");
        let u = HttpUrl::parse("http://127.0.0.1:8123/x?y=1#frag").unwrap();
        assert_eq!(
            (u.secure, u.host.as_str(), u.port, u.path.as_str()),
            (false, "127.0.0.1", 8123, "/x?y=1")
        );
        assert_eq!(HttpUrl::parse("https://example.com").unwrap().path, "/");
        assert!(HttpUrl::parse("ftp://x").is_err());
        assert!(HttpUrl::parse("https://user@evil/").is_err());
        assert!(HttpUrl::parse("https://h:99999/").is_err());
    }

    /// The real 1.0.0 release as GitHub returned it (trimmed).
    const REAL: &str = r#"{"tag_name":"1.0.0","name":"SentinelTask-1.0.0","body":"notes\r\nline","draft":false,"prerelease":false,
      "html_url":"https://github.com/DestroyersTaken/SentinelTask/releases/tag/1.0.0","assets":[
      {"name":"SentinelTask-1.0.0-installer.zip","state":"uploaded","size":4306629,
       "browser_download_url":"https://github.com/DestroyersTaken/SentinelTask/releases/download/1.0.0/SentinelTask-1.0.0-installer.zip",
       "digest":"sha256:c308a7571780b0aca6a57a0df02b12907507c9011dcdd1f3207bf9b88cf1a8e9"},
      {"name":"SentinelTask-1.0.0-portable.zip","state":"uploaded","size":5718639,
       "browser_download_url":"https://github.com/DestroyersTaken/SentinelTask/releases/download/1.0.0/SentinelTask-1.0.0-portable.zip",
       "digest":"sha256:a29cb19cd77b7c6fcc3d2cd3717d670eb0e455078cc51074779ef0382b7414df"}]}"#;

    #[test]
    fn parses_real_release_and_picks_assets() {
        let r = parse_release(REAL).unwrap();
        assert_eq!(r.version, Version(1, 0, 0));
        assert_eq!(r.notes, "notes\nline");
        // No setup.exe uploaded: installed copies use the installer zip.
        assert_eq!(
            pick_asset(&r, InstallKind::Installed).unwrap().name,
            "SentinelTask-1.0.0-installer.zip"
        );
        let p = pick_asset(&r, InstallKind::Portable).unwrap();
        assert_eq!(p.name, "SentinelTask-1.0.0-portable.zip");
        assert_eq!(
            p.sha256.as_deref(),
            Some("a29cb19cd77b7c6fcc3d2cd3717d670eb0e455078cc51074779ef0382b7414df")
        );
        // With a setup.exe present, it is preferred.
        let mut r2 = r.clone();
        r2.assets.push(Asset {
            name: "SentinelTask-1.0.0-setup.exe".into(),
            url: "u".into(),
            size: 1,
            sha256: None,
        });
        assert_eq!(
            pick_asset(&r2, InstallKind::Installed).unwrap().name,
            "SentinelTask-1.0.0-setup.exe"
        );
        // Drafts, pre-releases and garbage are refused.
        assert!(parse_release(&REAL.replace("\"prerelease\":false", "\"prerelease\":true")).is_err());
        assert!(parse_release(&REAL.replace("\"draft\":false", "\"draft\":true")).is_err());
        assert!(parse_release("<html>rate limited</html>").is_err());
        assert!(parse_release(r#"{"tag_name":"nightly"}"#).is_err());
        // Minimal release with no assets parses (and has nothing to pick).
        let m = parse_release(r#"{"tag_name":"v9.0.0"}"#).unwrap();
        assert!(pick_asset(&m, InstallKind::Portable).is_none());
    }

    #[test]
    fn checksums() {
        let data = b"hello";
        let h = sha256_hex(data);
        assert_eq!(h, "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824");
        let mut a = Asset {
            name: "x.zip".into(),
            url: String::new(),
            size: 5,
            sha256: Some(h.clone()),
        };
        assert!(verify(&a, data, None).is_ok());
        assert!(verify(&a, b"hellO", None).is_err());
        assert!(verify(&a, b"hell", None).is_err(), "size mismatch");
        a.sha256 = None;
        let sums = format!("{h}  ./x.zip\n{}  ./other.zip\n", "0".repeat(64));
        assert!(verify(&a, data, Some(&sums)).is_ok());
        assert!(verify(&a, b"hellO", Some(&sums)).is_err());
        assert!(verify(&a, data, None).is_ok(), "no checksum published: accepted");
        assert_eq!(hash_from_sums(&format!("{h} *X.ZIP"), "x.zip"), Some(h));
    }

    #[test]
    fn zip_roundtrip_and_damage() {
        let exe = fake_program(7);
        let z = make_zip(&[
            ("SentinelTask/README.md", b"readme", false),
            ("SentinelTask/SentinelTask.exe", &exe, true),
            ("SentinelTask/portable.txt", b"", false),
        ]);
        let (name, got) = zip_extract(&z, &|n| n.eq_ignore_ascii_case("sentineltask.exe")).unwrap();
        assert_eq!(name, "SentinelTask/SentinelTask.exe");
        assert_eq!(got, exe);
        let (_, r) = zip_extract(&z, &|n| n == "README.md").unwrap();
        assert_eq!(r, b"readme");
        assert!(zip_extract(&z, &|n| n == "missing").is_err());
        // Every single-byte corruption is caught (or harmless), never a panic.
        for i in (0..z.len()).step_by(997) {
            let mut bad = z.clone();
            bad[i] ^= 0x5a;
            if let Ok((_, out)) = zip_extract(&bad, &|n| n.eq_ignore_ascii_case("sentineltask.exe")) {
                assert_eq!(out, exe);
            }
        }
        for cut in [0, 10, z.len() / 2, z.len() - 1] {
            let _ = zip_extract(&z[..cut], &|_| true);
        }
        // A zip made by Info-ZIP (the release packaging tool).
        if let Ok(zip) = std::process::Command::new("zip").arg("-v").output() {
            if zip.status.success() {
                let dir = std::env::temp_dir().join(format!("stk-zip-{}", std::process::id()));
                std::fs::create_dir_all(dir.join("SentinelTask")).unwrap();
                std::fs::write(dir.join("SentinelTask/SentinelTask.exe"), &exe).unwrap();
                std::process::Command::new("zip")
                    .current_dir(&dir)
                    .args(["-q", "-9", "-r", "p.zip", "SentinelTask"])
                    .status()
                    .unwrap();
                let z = std::fs::read(dir.join("p.zip")).unwrap();
                assert_eq!(zip_extract(&z, &|n| n == "SentinelTask.exe").unwrap().1, exe);
                let _ = std::fs::remove_dir_all(dir);
            }
        }
    }

    #[test]
    fn install_kind_detection() {
        let dir = std::env::temp_dir().join(format!("stk-kind-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("SentinelTask.exe");
        assert_eq!(InstallKind::detect(&exe, false), InstallKind::Portable);
        std::fs::write(dir.join("Uninstall SentinelTask.exe"), b"x").unwrap();
        assert_eq!(InstallKind::detect(&exe, false), InstallKind::Installed);
        assert_eq!(InstallKind::detect(&exe, true), InstallKind::Portable);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn swap_keeps_a_working_exe() {
        let dir = std::env::temp_dir().join(format!("stk-swap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("SentinelTask.exe");
        std::fs::write(&exe, b"old").unwrap();
        // A missing .new must leave the old exe in place.
        assert!(swap_exe(&exe, &sibling(&exe, ".new")).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
        std::fs::write(sibling(&exe, ".new"), b"new").unwrap();
        swap_exe(&exe, &sibling(&exe, ".new")).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new");
        assert_eq!(std::fs::read(sibling(&exe, ".old")).unwrap(), b"old");
        // A second update works even though .old exists.
        std::fs::write(sibling(&exe, ".new"), b"newer").unwrap();
        swap_exe(&exe, &sibling(&exe, ".new")).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"newer");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn full_pipeline_with_a_local_feed() {
        let dir = std::env::temp_dir().join(format!("stk-feed-{}", std::process::id()));
        let app = dir.join("app");
        std::fs::create_dir_all(&app).unwrap();
        let exe = app.join("SentinelTask.exe");
        std::fs::write(&exe, fake_program(1)).unwrap();
        let new_exe = fake_program(2);
        let setup = fake_program(3);
        let portable = make_zip(&[("SentinelTask/SentinelTask.exe", &new_exe, true)]);
        let installer = make_zip(&[("SentinelTask-9.9.9-setup.exe", &setup, true)]);
        std::fs::write(dir.join("p.zip"), &portable).unwrap();
        std::fs::write(dir.join("i.zip"), &installer).unwrap();
        let url = |f: &str| format!("file://{}", dir.join(f).display()).replace('\\', "/");
        let feed = |portable_hash: &str| {
            format!(
                r#"{{"tag_name":"v9.9.9","assets":[
                {{"name":"SentinelTask-9.9.9-portable.zip","size":{},"browser_download_url":"{}","digest":"sha256:{}"}},
                {{"name":"SentinelTask-9.9.9-installer.zip","size":{},"browser_download_url":"{}","digest":"sha256:{}"}}]}}"#,
                portable.len(),
                url("p.zip"),
                portable_hash,
                installer.len(),
                url("i.zip"),
                sha256_hex(&installer)
            )
        };
        std::fs::write(dir.join("release.json"), feed(&sha256_hex(&portable))).unwrap();
        std::env::set_var("SENTINEL_UPDATE_API", url("release.json"));
        std::env::set_var("SENTINEL_VERSION_OVERRIDE", "1.0.0");

        let rel = check_latest().unwrap().expect("9.9.9 is newer than 1.0.0");
        assert_eq!(rel.version, Version(9, 9, 9));

        // Portable: staged next to the exe, then swapped in.
        let mut calls = 0;
        let p = download_and_prepare(&rel, InstallKind::Portable, &exe, &mut |_, _| {
            calls += 1;
            true
        })
        .unwrap();
        assert!(calls > 0);
        assert_eq!(p.file, sibling(&exe, ".new"));
        assert_eq!(std::fs::read(&p.file).unwrap(), new_exe);
        apply(&p, &exe, false).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), new_exe);
        assert_eq!(std::fs::read(sibling(&exe, ".old")).unwrap(), fake_program(1));

        // Installed: setup extracted from the installer zip.
        let i = download_and_prepare(&rel, InstallKind::Installed, &exe, &mut |_, _| true).unwrap();
        assert_eq!(std::fs::read(&i.file).unwrap(), setup);
        assert!(i.file.ends_with("SentinelTask-9.9.9-setup.exe"));

        // Cancelling stops the download.
        assert!(download_and_prepare(&rel, InstallKind::Portable, &exe, &mut |_, _| false).is_err());

        // A wrong checksum never stages anything.
        let _ = std::fs::remove_file(sibling(&exe, ".new"));
        std::fs::write(dir.join("release.json"), feed(&"ab".repeat(32))).unwrap();
        let bad = check_latest().unwrap().unwrap();
        let e = download_and_prepare(&bad, InstallKind::Portable, &exe, &mut |_, _| true).unwrap_err();
        assert!(e.contains("checksum"), "{e}");
        assert!(!sibling(&exe, ".new").exists());
        std::fs::write(dir.join("release.json"), feed(&sha256_hex(&portable))).unwrap();

        // The background updater reports through events.
        let mut u = Updater::new(exe.clone(), InstallKind::Portable, true, Arc::new(|| {}));
        u.check(true, true); // portable copies never auto-download
        match u.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            UpdateEvent::Available { release, manual } => {
                assert!(manual);
                u.download(release, true);
            }
            e => panic!("{e:?}"),
        }
        match u.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
            UpdateEvent::Ready { prepared, apply } => {
                assert!(apply);
                assert_eq!(prepared.version, Version(9, 9, 9));
            }
            e => panic!("{e:?}"),
        }
        let mut inst = Updater::new(exe.clone(), InstallKind::Installed, true, Arc::new(|| {}));
        inst.check(false, true); // installed copies download straight away
        assert!(matches!(
            inst.rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            UpdateEvent::Ready { apply: false, .. }
        ));
        assert!(matches!(inst.phase(), Phase::Ready(_)));

        // Up to date when we're already on the newest version.
        std::env::set_var("SENTINEL_VERSION_OVERRIDE", "9.9.9");
        assert!(check_latest().unwrap().is_none());
        assert!(!u.is_working(), "idle as soon as the event arrives");
        u.check(true, false);
        assert!(matches!(
            u.rx.recv_timeout(Duration::from_secs(10)).unwrap(),
            UpdateEvent::UpToDate { manual: true }
        ));
        std::env::remove_var("SENTINEL_UPDATE_API");
        std::env::remove_var("SENTINEL_VERSION_OVERRIDE");
        let _ = std::fs::remove_dir_all(dir);
    }
}
