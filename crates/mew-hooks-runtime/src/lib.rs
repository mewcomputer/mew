//! Plugin runtime implementing the `Dispatcher` trait.
//!
//! M7 implementation: each plugin is a subprocess communicating via
//! stdin/stdout newline-delimited JSON-RPC. This reuses the MCP-style
//! pattern already established in `mew-mcp`.
//!
//! Upgrade path: the `Dispatcher` trait doesn't change. Wasmtime component
//! model can replace the subprocess transport without touching agent code.

mod loader;
pub use loader::PluginLoader;

mod runtime;
pub use runtime::SubprocessDispatcher;

mod dynamic_tool;
pub use dynamic_tool::DynamicTool;

mod transport;
pub use transport::{call_via_handles, ExtensionConnection, PluginHandles, PluginSlot};
