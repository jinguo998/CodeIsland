//! CodeIsland IPC transport.
//!
//! The desktop app runs [`serve`] forever. One-shot agent hooks call
//! [`client::send`]. Messages are newline-delimited JSON (one object per line)
//! carried over a Windows named pipe or a Unix domain socket; see
//! [`endpoint_name`] for the platform-specific address.

pub mod client;
pub mod endpoint;
pub mod protocol;
mod server;

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

pub use endpoint::endpoint_name;
pub use protocol::{decode_line, encode_line, ClientMessage, ServerMessage, PROTOCOL_VERSION};
pub use server::{serve, Handler};
