//! The wire format and the limits the server enforces.

use serde::{Deserialize, Serialize};

/// A message body is capped so one agent in a loop cannot fill the disk.
pub const MAX_BODY: usize = 32 * 1024;

/// Pending (unacknowledged) messages per recipient.
pub const MAX_PENDING: i64 = 200;

/// A message that nobody collected is dropped.
pub const TTL_SECS: i64 = 7 * 24 * 3600;

pub const MAX_PROJECT: usize = 64;
pub const MAX_MAILBOX: usize = 64;
pub const MAX_CLIENT_ID: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub from: String,
    pub project: String,
    pub body: String,
    /// Unix seconds, UTC. Formatted for people by the CLI.
    pub created: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendRequest {
    /// Chosen by the sender so that a retry does not duplicate the message.
    pub client_id: String,
    pub to: String,
    pub project: String,
    pub body: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SendResponse {
    pub id: String,
    pub duplicate: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MessagesResponse {
    pub messages: Vec<Message>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MeResponse {
    pub mailbox: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct PeersResponse {
    pub mailboxes: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OkResponse {
    pub ok: bool,
}
