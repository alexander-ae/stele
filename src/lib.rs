//! stele is a mailbox between coding agents on different machines.
//!
//! The design is in `docs/design.md`. The short version: a message arrives as
//! an anathem draft, never as active memory, and this program never writes
//! memory itself — it only hands the body to `mem save --external`.

pub mod api;
pub mod cli;
pub mod client;
pub mod config;
pub mod http;
pub mod mem;
pub mod pull;
pub mod server;
pub mod store;
pub mod time;
pub mod util;
