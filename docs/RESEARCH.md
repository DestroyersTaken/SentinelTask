# SentinelTask — research notes

SentinelTask is a ground-up replacement for TinyTask. Before writing any code we
collected the problems TinyTask users and developers of TinyTask-style tools
keep running into, the Windows API behaviour behind them, and the features
people ask for. Each finding below is mapped to the design decision that
addresses it. Section numbers (§) are referenced from the source code.

## 1. Known TinyTask problems and their causes

| # | Problem reported | Root cause | SentinelTask fix |
|---|------------------|------------|------------------|
| 1 | Clicks land in the wrong place on laptops / 4K screens / scaled displays, or on a second monitor | TinyTask is not DPI aware, so Windows feeds it *virtualised* (scaled) coordinates; it also has no notion of monitors left of / above the primary one (negative coordinates) | Process is **Per-Monitor-V2 DPI aware** (manifest + runtime call). Positions are stored in physical pixels together with the virtual-desktop rectangle. Playback normalises against the whole virtual desktop (`MOUSEEVENTF_VIRTUALDESK`) using 64-bit math with rounding (§coords) |
| 2 | Macro recorded on one resolution misses targets on another | Raw pixel coordinates with no record of the screen | Each macro stores the screen geometry it was recorded on; playback offers *Keep as recorded* or *Scale to current screen* and warns when they differ |
| 3 | Does not work in many games (Roblox, Minecraft, DirectX games) | Games read **DirectInput / Raw Input scan codes** and ignore virtual-key-only injection; 3D cameras read **relative mouse deltas** while the cursor is locked | Keys are injected with `KEYEVENTF_SCANCODE` (+ `KEYEVENTF_EXTENDEDKEY` from a hard-coded extended-key table, because `MapVirtualKey` is unreliable for the E0 prefix). Raw mouse deltas are recorded through Raw Input, and a **Relative (games / 3D camera)** playback mode replays them as relative motion |
| 4 | Nothing happens when the target app runs as administrator | **UIPI**: a normal-integrity process cannot hook or inject into an elevated window | SentinelTask detects that the foreground window is elevated while it is not, shows a clear banner, and offers *Restart as administrator* |
| 5 | Keys / mouse buttons stay "stuck down" after stopping, crashing or closing | Playback is aborted between a key-down and its key-up | Every key and button pressed during playback is tracked; **all are released** on stop, pause, error, panic and application exit. Recordings are normalised so every press has a release |
| 6 | Hotkey keystrokes or the click on the Stop button end up inside the macro | Recorder does not filter its own input | The record/stop chord is stripped precisely (only those exact key events, never neighbouring events). Clicks and keys aimed at SentinelTask's own windows are ignored while recording |
| 7 | Playback replays its own injected input into a new recording; hotkeys fire from replayed keys | Injected events are not distinguished from physical ones | The hooks check `LLKHF_INJECTED` / `LLMHF_INJECTED`; injected input is never recorded and never counts as "user activity" |
| 8 | Recording silently stops capturing after a while / on a busy PC | Windows removes a low-level hook **without notification** if its callback exceeds `LowLevelHooksTimeout` (max 1000 ms since Win10 1709) | Hook callbacks only copy the event into a lock-free queue and return immediately, on a dedicated thread with its own message pump |
| 9 | Fast taps and very short clicks go missing | Polling input on a timer (TinyTask-style 10 ms polling) can miss a press+release between polls | Event-driven capture via low-level hooks and Raw Input — nothing is polled |
| 10 | Timing drifts on long or looped macros; replays slower than recorded | Sleeping event-by-event accumulates error; default Windows timer tick is ~15.6 ms | The scheduler computes every event's **absolute deadline** from the start of the run, so errors never accumulate. Waiting uses a high-resolution waitable timer (`CREATE_WAITABLE_TIMER_HIGH_RESOLUTION`, Win10 1803+) with a short final spin, falling back to `timeBeginPeriod(1)` only while playing |
| 11 | Long waits in a macro are skipped, or idle time at the start/end wastes time | Idle time not represented / not editable | Idle time is preserved exactly. The editor can trim leading/trailing idle time and cap long pauses |
| 12 | PC goes to sleep or the screen turns off during an overnight autofarm, ruining it | Nothing tells Windows the machine is in use | `SetThreadExecutionState(ES_SYSTEM_REQUIRED | ES_DISPLAY_REQUIRED)` while playing (optional) |
| 13 | Can't stop continuous playback; stop hotkey ignored | Stop key captured by another app, or hard-coded keys | Fully rebindable hotkeys registered with `RegisterHotKey`, with **conflict detection** (tells you if another app owns the combo). Separate always-available **emergency stop** (default `Pause`, plus `Ctrl+Shift+Esc`-style panic options) and optional *stop when I move the mouse / press a key* |
| 14 | Macro clicks the wrong thing after a window is moved, minimised or loses focus | Blind replay | Optional **"only play while this window is focused"** guard: playback auto-pauses when the recorded target window is not in front and resumes when it is |
| 15 | Compiled EXEs flagged as trojans | Unsigned stub that records/replays input looks like a keylogger to heuristics | Macros are plain, readable JSON (`.stmacro`). Standalone export is optional and honest about this; the build is reproducible from source in CI so users can verify it |
| 16 | `.rec` files can't be inspected, edited, or trimmed | Opaque binary format; no editor | Full event editor (delete, change delays, insert waits, disable mouse moves, trim), plus **import of TinyTask `.rec` files** |
| 17 | Abandoned / closed source, 32-bit only | Developer stopped public development | Open source, 64-bit, CI-built |
| 18 | Mouse button held during recording is lost; drag operations break | Missing button-state tracking | Button down/up recorded independently with positions; drags replay as down → moves → up |
| 19 | Mouse "teleports" to a screen corner when a keyboard-only macro plays | Player moves the cursor even if no mouse events were recorded | Mouse is only touched when the macro contains mouse events |
| 20 | Wheel scrolls replay wrong amount / horizontal wheel missing | Partial wheel handling | Vertical and horizontal wheel recorded with exact deltas |
| 21 | Hotkeys die when a keyboard is replugged | Hook bound to a device | `RegisterHotKey` is device-independent |
| 22 | Speed only from a few presets; can't run *for* a duration | Limited UI | Speed 0.1×–100×, loops: once / N times / forever / for a duration, delay between loops, start countdown |

## 2. Features people want that TinyTask lacks

From comparisons with Pulover's Macro Creator, Macro Recorder, OntyTask,
atbswp issues (#76 infinite loops on exported executables, #91 held buttons,
#98 multiple mice, #100/#89/#93 keyboard layouts) and TinyTask-Linux's fix list:

- Rebindable hotkeys, including single keys (F-keys) — **done**
- Pause / resume — **done**
- Macro library with names, descriptions, last-used, durations — **done**
- Event editor and timeline — **done**
- Per-macro default speed & loop settings saved in the file — **done**
- Built-in **auto-clicker** (interval, button, single/double, hold-key repeat, click count) — **done**
- On-screen status overlay (elapsed, loop n/N, stop key) that never intercepts clicks — **done**
- Window-focus guard and elevation detection — **done**
- Command line (`play`, `--loops`, `--speed`) for Task Scheduler / scripts — **done**
- Layout-independent keyboard playback (scan codes, so AZERTY/QWERTZ users and games see the same physical key) — **done**
- Image / pixel search and conditional logic — *out of scope for v1* (see roadmap)

## 3. Windows API facts the implementation depends on

- **Journaling hooks** (`WH_JOURNALRECORD` / `WH_JOURNALPLAYBACK`, the classic
  TinyTask-era mechanism) are *unsupported starting in Windows 11 and will be
  removed*; Microsoft recommends `SendInput`. We never use them.
- `LowLevelMouseProc`/`LowLevelKeyboardProc`: callback must return within
  `LowLevelHooksTimeout`; on Windows 7+ a slow hook is removed silently. Hooks
  need a thread that pumps messages.
- `KBDLLHOOKSTRUCT.flags`: `LLKHF_INJECTED` (bit 4) marks injected events.
- `MOUSEINPUT`: absolute coordinates are 0..65535; `MOUSEEVENTF_VIRTUALDESK`
  maps them to the entire virtual desktop (which may have negative origin).
- `KEYEVENTF_SCANCODE`: `wVk` is ignored; key-up must also carry the flag.
  Extended keys (arrows, Ins/Del/Home/End/PgUp/PgDn, right Ctrl/Alt, numpad
  Enter and `/`, Win keys, Apps) need `KEYEVENTF_EXTENDEDKEY`.
- `timeBeginPeriod` no longer affects global resolution since Win10 2004;
  high-resolution waitable timers exist since Win10 1803.
- TinyTask `.rec`: optional small header followed by 20-byte little-endian
  `EVENTMSG` records `{message, paramL, paramH, time, hwnd}`. Keyboard:
  low byte of `paramL` = VK, next byte = scan code, `paramH & 0x8000` =
  extended. Mouse: `paramL`/`paramH` = x/y. Wheel: `paramH` = signed delta.
  `time` is a `GetTickCount` value (may wrap).

## 4. Lessons from testing the real Windows build

Found by driving the actual `SentinelTask.exe` (real keystrokes, a real
window manager, GitHub's Windows runners) rather than unit tests alone:

| Symptom | Cause | Fix |
|---|---|---|
| Record hotkey (Ctrl+Alt+Shift+R) did nothing for some users | GPU overlays (e.g. the NVIDIA App's Alt+R) grab the keystroke with their own low-level hook before `RegisterHotKey` sees it | Hotkeys are also detected by SentinelTask's own keyboard hook, which Windows calls first (newest hook first); the matched key is swallowed so it never fires twice |
| No hotkey worked — not even the emergency stop — while SentinelTask was minimized | eframe doesn't run the UI pass for a minimized window, and event handling lived there | All non-drawing work runs in `App::logic`, which also runs while minimized |
| Crash when minimizing during playback | The status overlay was an *immediate* viewport; eframe panics if it can't create it during a hidden pass | Overlay is a *deferred* viewport drawing from shared state |
| Keystrokes went to the status overlay after minimizing | Windows activates the next window in z-order — the always-on-top overlay | Overlay is `WS_EX_NOACTIVATE \| WS_EX_TOOLWINDOW`; it never counts as "SentinelTask in front" |
| A stray Alt press at the end of macros | The stop chord's modifier key-ups were recorded before the UI reacted | Capture stops inside the hook the instant a stop hotkey is pressed; chord stripping also removes a released chord |
| Clicks sometimes landed at the previous position | `SendInput` is asynchronous; reading the cursor right after a move is racy | The click carries its own position; a separate move goes first in the same `SendInput` call (games such as Roblox need to see the cursor arrive) |
| Compact mode stayed full-size | A single resize request can be undone by the DPI adjustment after the window appears, or by a maximized/snapped window | Window is created at the right size; the size is verified every frame and the request repeated until it sticks |
| Rebinding a hotkey could get stuck on "press keys" | The system hook path had no fallback and no way out | Cancel button, Esc, and a fallback that reads keys from SentinelTask's own window |
| Fast double-press of a hotkey counted once | The duplicate guard merged genuine presses | Only the *same* press arriving via both detection paths is merged |

### Updates

- Downloads use WinHTTP, so the Windows proxy settings and certificate store
  apply (as in the browser) and nothing extra ships in the exe. A file
  downloaded this way has no "downloaded from the internet" mark, so
  SmartScreen doesn't block the update.
- Windows can't overwrite a running exe, but it can rename one. A portable
  update writes `SentinelTask.exe.new`, renames the running exe to `.old`,
  moves the new one into place (and moves the old one back if that fails),
  and starts it. The new copy waits for the old process to exit, because the
  old one still holds the global hotkeys, then deletes `.old`.
- The installer's update mode (`/S /UPDATE`) waits on the `SentinelTask.Running`
  mutex instead of showing a "please close" box, and it re-selects only the
  optional parts that were installed before. Otherwise a silent install
  would bring back a desktop shortcut the user had deleted.
- GitHub publishes a SHA-256 `digest` for every release file. The updater
  checks it, the size, and that the file is a Windows program before
  replacing anything.

## 5. Responsible use

SentinelTask automates *your own* mouse and keyboard. Many online games
prohibit automation in their terms of service; SentinelTask does not include
anti-cheat evasion or "humanisation" features and the README says so.

## Sources

- Microsoft: JournalPlaybackProc (journaling hooks unsupported in Windows 11) — https://learn.microsoft.com/en-us/windows/win32/winmsg/journalplaybackproc
- Microsoft: LowLevelMouseProc (LowLevelHooksTimeout, silent removal) — https://learn.microsoft.com/windows/win32/winmsg/lowlevelmouseproc
- Raymond Chen, Old New Thing (hooks on thread-pool threads) — https://devblogs.microsoft.com/oldnewthing/20150514-00/?p=45601
- Microsoft: KBDLLHOOKSTRUCT — https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-kbdllhookstruct
- Microsoft: MOUSEINPUT — https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-mouseinput
- Microsoft: timeBeginPeriod — https://learn.microsoft.com/windows/win32/api/timeapi/nf-timeapi-timebeginperiod
- Python issue 45429 (high-resolution waitable timers) — https://www.python.org/pipermail/python-bugs-list/2021-October/492804.html
- GameDev.net: SendInput in DirectInput applications — https://www.gamedev.net/topic/581515-simulating-keyboard-with-sendinput-api-in-directinput-applications/
- Microsoft Q&A: virtual key codes / extended keys — https://learn.microsoft.com/en-us/answers/questions/226490/virtual-key-codes-not-working-as-expected
- Razer Insider: relative mouse macros vs Raw Input games — https://insider.razer.com/razer-synapse-4-55/bug-synapse-4-macro-mouse-movement-always-jumps-to-top-left-corner-0-0-relative-mode-saves-as-foreground-91570
- Mojang MC-230466 (automated input with raw input on) — https://bugs.mojang.com/browse/MC-230466
- TinyTask-Linux v1.0.0 release notes (list of TinyTask-style bugs) — https://github.com/burlone0/TinyTask-Linux/releases/tag/v1.0.0
- OntyTask comparison table — https://github.com/Agzes/OntyTask
- atbswp issue tracker — https://github.com/RMPR/atbswp/issues
- TinyTask antivirus false-positive analysis — https://blog.gridinsoft.com/tinytask-virus/
- AlternativeTo: TinyTask (development status) — https://alternativeto.net/software/tinytask/about
- Pulover's Macro Creator — https://www.autohotkey.com/boards/viewtopic.php?p=357901
- pixelactions-core virtual desktop notes — https://docs.rs/crate/pixelactions-core/latest/source/src/virtualdesk.rs
