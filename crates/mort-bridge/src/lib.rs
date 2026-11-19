//! mort's REAPER extension (ADR 0001, ADR 0002).
//!
//! Serves JSON-RPC on a Unix socket and runs requests against REAPER's API
//! on the main thread.
//!
//! - [`server`]: the socket threads. They never call REAPER.
//! - [`dispatch`]: the main-thread dispatcher, generic over
//!   [`mort_session::Session`] so it runs over the fake in tests.
//! - [`methods`]: the RPC methods, generic over the session.
//! - `reaper`: the extension entry point and the REAPER session.

pub mod dispatch;
pub mod methods;
pub mod runs;
pub mod server;

mod chunk;
mod raw;

mod reaper;
