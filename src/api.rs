//! The wire format and the limits the server enforces.

use serde::{Deserialize, Serialize};

/// A message body is capped so one agent in a loop cannot fill the disk.
pub const MAX_BODY: usize = 32 * 1024;

/// The HTTP request carries the body inside a JSON envelope, and `serde_json`
/// expands `"` and `\` to two bytes and a control character to six. The
/// request cap is therefore sized from the worst case, so a body at
/// `MAX_BODY` always fits and the body check stays the one that speaks.
pub const MAX_REQUEST: usize = MAX_BODY * 6 + 4 * 1024;

/// One inbox read returns at most this many bytes of bodies. The client
/// acknowledges what it receives, so a full mailbox drains in several pulls
/// instead of producing a response the client refuses to read.
pub const MAX_INBOX_BYTES: usize = 1024 * 1024;

/// Pending (unacknowledged) messages per recipient.
pub const MAX_PENDING: i64 = 200;

/// A message that nobody collected is dropped.
pub const TTL_SECS: i64 = 7 * 24 * 3600;

pub const MAX_PROJECT: usize = 64;
pub const MAX_MAILBOX: usize = 64;
pub const MAX_CLIENT_ID: usize = 64;

/// Characters allowed in names. Deliberately narrow: these strings travel
/// into logs, headers, CLI arguments and anathem's `--from` label, and a
/// composite key assembled from unescaped parts is a known way to confuse a
/// scope (mem0's CVE-2026-116575 is the recent example).
pub const MAILBOX_CHARS: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._@-";
pub const CLIENT_ID_CHARS: &str =
    "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789._:@-";

pub fn first_invalid(value: &str, allowed: &str) -> Option<char> {
    value.chars().find(|c| !allowed.contains(*c))
}

/// The size a string takes inside a JSON document, before the envelope.
/// `serde_json` leaves non-ASCII characters alone, doubles quotes and
/// backslashes, and writes control characters as `\u00XX`.
pub fn json_escaped_len(value: &str) -> usize {
    value
        .chars()
        .map(|c| match c {
            '"' | '\\' => 2,
            c if (c as u32) < 0x20 => 6,
            c => c.len_utf8(),
        })
        .sum()
}

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
pub struct StatusResponse {
    pub mailbox: String,
    /// Unacknowledged, unexpired messages for this mailbox.
    pub pending: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub error: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OkResponse {
    pub ok: bool,
}
