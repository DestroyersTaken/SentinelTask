//! # sentinel-core
//!
//! The platform-independent heart of SentinelTask: the macro model, file
//! formats (native `.stmacro` and TinyTask `.rec` import), the drift-free
//! playback scheduler, recording clean-up, editing operations and settings.
//!
//! Everything here is deterministic and unit-tested; the Windows-specific
//! input capture and injection lives in the `sentineltask` binary crate.
//! See `docs/RESEARCH.md` for the reasoning behind each design decision.

pub mod clicker;
pub mod coords;
pub mod edit;
pub mod format;
pub mod keys;
pub mod model;
pub mod normalize;
pub mod playback;
pub mod rec;
pub mod settings;
pub mod state;
pub mod util;

pub use coords::Rect;
pub use model::{Event, EventKind, Key, Macro, MacroMeta, MouseButton, ScreenInfo, TargetWindow};
pub use playback::{MouseMode, PlaybackOptions, Player, Repeat, Step};

pub const APP_NAME: &str = "SentinelTask";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
