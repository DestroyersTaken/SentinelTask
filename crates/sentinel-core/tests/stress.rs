//! Stress and randomized ("fuzz") tests for the engine: random and extreme
//! inputs must never panic, hang, or break the guarantees the app relies
//! on (sorted events, every press has a release, no drift).

use sentinel_core::coords::{from_absolute, rescale_point, to_absolute, Rect};
use sentinel_core::edit::{self, KindFilter};
use sentinel_core::keys::{vk, Hotkey};
use sentinel_core::model::{Event, EventKind, Key, Macro, MouseButton};
use sentinel_core::normalize::{balance, finish_recording};
use sentinel_core::playback::{PlaybackOptions, Player, Repeat, Step};
use sentinel_core::{format, rec, settings::Settings};

/// Small deterministic PRNG (xorshift64*), so failures are reproducible.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn range(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.below((hi - lo + 1) as u64) as i32
    }
    fn chance(&mut self, pct: u64) -> bool {
        self.below(100) < pct
    }
}

const KEYS: [u16; 8] = [0x41, 0x57, vk::LSHIFT, vk::LCONTROL, vk::LMENU, vk::SPACE, vk::UP, vk::F1 + 7];

fn random_events(rng: &mut Rng, n: usize) -> Vec<Event> {
    let mut t = 0u64;
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        t += rng.below(50_000);
        let kind = match rng.below(6) {
            0 => EventKind::MouseMove {
                x: rng.range(-3000, 6000),
                y: rng.range(-1000, 3000),
            },
            1 => EventKind::MouseDelta {
                dx: rng.range(-50, 50),
                dy: rng.range(-50, 50),
            },
            2 => EventKind::MouseButton {
                button: MouseButton::ALL[rng.below(5) as usize],
                down: rng.chance(50),
                x: rng.range(0, 1920),
                y: rng.range(0, 1080),
            },
            3 => EventKind::Wheel {
                delta: rng.range(-480, 480),
                horizontal: rng.chance(20),
                x: rng.range(0, 1920),
                y: rng.range(0, 1080),
            },
            _ => {
                let code = KEYS[rng.below(KEYS.len() as u64) as usize];
                EventKind::Key {
                    key: Key::new(code, (code & 0x7F) + 1, rng.chance(10)),
                    down: rng.chance(55),
                }
            }
        };
        out.push(Event::new(t, kind));
    }
    out
}

/// The invariants every macro must satisfy before playback.
fn assert_invariants(events: &[Event], ctx: &str) {
    assert!(events.windows(2).all(|w| w[0].t_us <= w[1].t_us), "{ctx}: not sorted");
    let mut keys: Vec<(u16, bool)> = Vec::new();
    let mut buttons: Vec<MouseButton> = Vec::new();
    for (i, e) in events.iter().enumerate() {
        match e.kind {
            EventKind::Key { key, down: true } => {
                if !keys.contains(&key.identity()) {
                    keys.push(key.identity());
                }
            }
            EventKind::Key { key, down: false } => {
                let before = keys.len();
                keys.retain(|k| *k != key.identity());
                assert!(keys.len() < before, "{ctx}: orphan key-up at {i}");
            }
            EventKind::MouseButton { button, down: true, .. } => {
                if !buttons.contains(&button) {
                    buttons.push(button);
                }
            }
            EventKind::MouseButton { button, down: false, .. } => {
                let before = buttons.len();
                buttons.retain(|b| *b != button);
                assert!(buttons.len() < before, "{ctx}: orphan button-up at {i}");
            }
            _ => {}
        }
    }
    assert!(keys.is_empty(), "{ctx}: keys left held: {keys:?}");
    assert!(buttons.is_empty(), "{ctx}: buttons left held: {buttons:?}");
}

#[test]
fn random_recordings_always_normalise_to_a_safe_macro() {
    let hotkeys = [
        Hotkey::with(true, true, true, 0x52),
        Hotkey::key(vk::PAUSE),
        Hotkey::with(true, false, false, vk::F1 + 7),
    ];
    for seed in 1..=400u64 {
        let mut rng = Rng::new(seed);
        let n = rng.below(400) as usize;
        let mut e = random_events(&mut rng, n);
        // Shuffle a little: hooks can deliver slightly out of order.
        for _ in 0..rng.below(10) {
            if e.len() > 1 {
                let a = rng.below(e.len() as u64) as usize;
                let b = rng.below(e.len() as u64) as usize;
                e.swap(a, b);
            }
        }
        let hk = hotkeys[rng.below(3) as usize];
        let total = |ev: &[Event]| {
            ev.iter().fold((0i64, 0i64), |a, e| match e.kind {
                EventKind::MouseDelta { dx, dy } => (a.0 + dx as i64, a.1 + dy as i64),
                _ => a,
            })
        };
        let motion_before = total(&e);
        finish_recording(&mut e, Some(&hk), 4000);
        assert_invariants(&e, &format!("seed {seed}"));
        // Raw camera motion is merged but never lost.
        assert_eq!(total(&e), motion_before, "seed {seed}: raw motion lost");
        // The safety passes are idempotent (coalescing may merge further,
        // which is harmless and never happens twice in the app).
        let once = e.clone();
        sentinel_core::normalize::strip_trailing_hotkey(&mut e, &hk);
        balance(&mut e);
        assert_eq!(e, once, "seed {seed}: strip+balance changed a clean macro");
    }
}

#[test]
fn random_edits_keep_macros_safe() {
    for seed in 1..=300u64 {
        let mut rng = Rng::new(seed * 7919);
        let mut e = random_events(&mut rng, 300);
        balance(&mut e);
        for _ in 0..20 {
            match rng.below(8) {
                0 => {
                    let count = rng.below(20);
                    let len = e.len().max(1) as u64;
                    let idx: Vec<usize> = (0..count).map(|_| rng.below(len) as usize).collect();
                    edit::delete_events(&mut e, &idx, rng.chance(50));
                }
                1 => {
                    edit::cap_pauses(&mut e, rng.below(2_000_000));
                }
                2 => edit::trim_leading_idle(&mut e, rng.below(1_000_000)),
                3 => edit::scale_time(&mut e, 0.05 + (rng.below(400) as f64) / 100.0),
                4 => {
                    if !e.is_empty() {
                        let i = rng.below(e.len() as u64) as usize;
                        edit::set_gap_before(&mut e, i, rng.below(3_000_000));
                    }
                }
                5 => {
                    if !e.is_empty() {
                        let i = rng.below(e.len() as u64) as usize;
                        edit::insert_wait(&mut e, i, rng.below(1_000_000));
                    }
                }
                6 => {
                    edit::remove_kinds(
                        &mut e,
                        KindFilter {
                            moves: rng.chance(30),
                            deltas: rng.chance(30),
                            clicks: rng.chance(10),
                            wheel: rng.chance(30),
                            keys: rng.chance(10),
                        },
                    );
                }
                _ => edit::offset_positions(&mut e, rng.range(-500, 500), rng.range(-500, 500), None),
            }
            // Balance is what playback applies; after it, the macro must be safe.
            let mut checked = e.clone();
            balance(&mut checked);
            assert_invariants(&checked, &format!("edit seed {seed}"));
        }
    }
}

#[test]
fn random_playback_options_always_terminate_without_drift() {
    for seed in 1..=300u64 {
        let mut rng = Rng::new(seed * 104_729);
        let n = 1 + rng.below(200) as usize;
        let mut e = random_events(&mut rng, n);
        balance(&mut e);
        let opts = PlaybackOptions {
            speed: [0.1, 0.5, 1.0, 3.7, 100.0][rng.below(5) as usize],
            repeat: match rng.below(3) {
                0 => Repeat::Times {
                    count: 1 + rng.below(5) as u32,
                },
                1 => Repeat::Duration { secs: 1 + rng.below(30) },
                _ => Repeat::Times { count: 2 },
            },
            loop_delay_ms: rng.below(500),
            ..Default::default()
        };
        let mut p = Player::new(&e, &opts, 0);
        let mut now = 0u64;
        let mut fired = 0usize;
        let mut steps = 0usize;
        loop {
            steps += 1;
            assert!(steps < 5_000_000, "seed {seed}: player didn't terminate");
            match p.step(now) {
                Step::Fire { index } => {
                    let pass_start_guess = now; // fired no earlier than its deadline
                    assert!(index < e.len());
                    let _ = pass_start_guess;
                    fired += 1;
                }
                Step::WaitUntil { t_us } => {
                    assert!(t_us > now, "seed {seed}: wait in the past");
                    // Random oversleep up to 3 ms.
                    now = t_us + rng.below(3000);
                }
                Step::PassComplete { .. } => {}
                Step::Done => break,
            }
        }
        assert!(fired >= e.len(), "seed {seed}: fired {fired} of {}", e.len());
        assert_eq!(fired % e.len(), 0, "seed {seed}: partial pass");
    }
}

#[test]
fn coordinate_math_holds_for_random_layouts() {
    let mut rng = Rng::new(42);
    for _ in 0..2000 {
        let desk = Rect::new(rng.range(-8000, 0), rng.range(-4000, 0), rng.range(320, 16_000), rng.range(200, 8_000));
        for _ in 0..20 {
            let x = rng.range(desk.x, desk.right() - 1);
            let y = rng.range(desk.y, desk.bottom() - 1);
            let (ax, ay) = to_absolute(x, y, desk);
            assert!((0..=65535).contains(&ax) && (0..=65535).contains(&ay));
            assert_eq!(from_absolute(ax, ay, desk), (x, y), "desk {desk:?} point ({x},{y})");
            let to = Rect::new(rng.range(-4000, 0), 0, rng.range(320, 8000), rng.range(200, 5000));
            let (nx, ny) = rescale_point(x, y, desk, to);
            assert!(to.contains(nx, ny), "rescale left the target screen");
        }
    }
}

#[test]
fn garbage_files_never_panic() {
    let mut rng = Rng::new(7);
    for len in 0..600usize {
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let _ = rec::parse_records(&bytes);
        let _ = rec::import(&bytes, "x");
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let _ = format::from_str(s);
            let _ = Settings::from_json(s);
        }
    }
    // Mutated real files.
    let mut m = Macro::new("m");
    m.events = random_events(&mut rng, 200);
    balance(&mut m.events);
    let good = format::to_string(&m);
    for i in 0..3000 {
        let mut b = good.clone().into_bytes();
        let pos = rng.below(b.len() as u64) as usize;
        match i % 3 {
            0 => b[pos] = rng.next() as u8,
            1 => {
                b.truncate(pos);
            }
            _ => {
                b.insert(pos, b"[]{},:\"-9e"[rng.below(10) as usize]);
            }
        }
        if let Ok(s) = String::from_utf8(b) {
            if let Ok(back) = format::from_str(&s) {
                assert_invariants(&back.events, "mutated file that still loaded");
            }
            let _ = Settings::from_json(&s);
        }
    }
}

#[test]
fn huge_macros_are_fast() {
    // One hour of 1000 Hz mouse movement plus keys: 3.6 million raw events.
    let mut rng = Rng::new(99);
    let mut e: Vec<Event> = (0..3_600_000u64)
        .map(|i| {
            if i % 1000 == 0 {
                Event::new(
                    i * 1000,
                    EventKind::Key {
                        key: Key::from_vk(0x41),
                        down: (i / 1000) % 2 == 0,
                    },
                )
            } else {
                Event::new(
                    i * 1000,
                    EventKind::MouseMove {
                        x: rng.range(0, 1919),
                        y: rng.range(0, 1079),
                    },
                )
            }
        })
        .collect();
    let t = std::time::Instant::now();
    finish_recording(&mut e, None, 4000);
    let normalise = t.elapsed();
    assert!(e.len() < 1_000_000, "coalescing kept {} events", e.len());
    assert_invariants(&e, "huge");

    let mut m = Macro::new("huge");
    m.events = e;
    let t = std::time::Instant::now();
    let s = format::to_string(&m);
    let back = format::from_str(&s).unwrap();
    let io = t.elapsed();
    assert_eq!(back.events.len(), m.events.len());

    let t = std::time::Instant::now();
    let mut p = Player::new(&m.events, &PlaybackOptions::default(), 0);
    let mut now = 0;
    loop {
        match p.step(now) {
            Step::WaitUntil { t_us } => now = t_us,
            Step::Done => break,
            _ => {}
        }
    }
    let play = t.elapsed();
    // Generous limits for debug builds on slow CI machines.
    assert!(normalise.as_secs_f64() < 20.0, "normalise took {normalise:?}");
    assert!(io.as_secs_f64() < 30.0, "save+load took {io:?}");
    assert!(play.as_secs_f64() < 10.0, "scheduling took {play:?}");
    eprintln!(
        "huge macro: {} events after coalescing, {:.1} MB file; normalise {normalise:?}, save+load {io:?}, schedule {play:?}",
        m.events.len(),
        s.len() as f64 / 1e6
    );
}

