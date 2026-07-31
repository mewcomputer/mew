//! Framework-independent client state and transport contracts.
//!
//! This crate is deliberately unaware of GPUI, UniFFI, and platform services.
//! Desktop, mobile, and headless clients use it to decode daemon messages,
//! assemble conversation state, and issue typed protocol commands.

mod codec;
mod engine;
mod reducer;
mod transport;

pub use codec::{decode_server_message_lenient, encode_client_message};
pub use engine::ClientEngine;
pub use reducer::{
    ActionKind, ClientEvent, ClientSession, ClientState, ConnectionStatus, PendingAction,
};
pub use transport::{
    ClientConnection, ClientTransport, InMemoryConnection, InMemoryTransport, TransportError,
};
