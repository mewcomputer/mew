//! Runtime module — the single dispatch path and event loop.
//!
//! All `Action` and `SlashResult` matching lives in [`dispatch::handle_action`].
//! The drain loop never interprets actions — it coalesces and replays them
//! through `handle_action`. The daemon is the only runtime path: the TUI talks
//! to a `mew` daemon via `daemon::DaemonTarget`, which owns the `DaemonClient`.

pub mod daemon;
pub mod dispatch;
pub mod mentions;
pub mod target;

pub use dispatch::{handle_action, Ctx, Flow};
#[allow(unused_imports)]
pub use target::{CommandTarget, Unsupported};
