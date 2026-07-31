//! Compatibility re-exports for the mobile UniFFI facade.
//!
//! Wire decoding belongs to `mew-client-core`; this module keeps the old
//! mobile path stable while callers migrate to the shared crate directly.

pub use mew_client_core::{decode_server_message_lenient, encode_client_message};
