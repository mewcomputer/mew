//! Runtime module — the single dispatch path and event loop.
//!
//! All `Action` and `SlashResult` matching lives in [`dispatch::handle_action`].
//! The drain loop never interprets actions — it coalesces and replays them
//! through `handle_action`. Two `CommandTarget` impls provide the
//! backend-specific operations:
//!
//! - [`local::LocalTarget`] — owns the `Agent`, used in standalone TUI mode.
//! - `daemon::DaemonTarget` — owns the `DaemonClient`, used in `--connect` mode.

pub mod daemon;
pub mod dispatch;
pub mod local;
pub mod mentions;
pub mod target;

pub use dispatch::{handle_action, Ctx, Flow};
#[allow(unused_imports)]
pub use target::{CommandTarget, Unsupported};
