//! The mailbox itself: one blocking accept loop, one SQLite connection, no
//! workers and no async runtime. At the traffic this is for — tens of messages
//! a day — a thread per request would only add state to keep in sync.

use crate::api::{
    self, ErrorResponse, MeResponse, MessagesResponse, OkResponse, PeersResponse, SendRequest,
    SendResponse, StatusResponse,
};
use crate::http::{self, ReadError, Request};
use crate::store::{InsertOutcome, Store};
use crate::time;
use crate::util::{random_hex, sha256_hex};
use anyhow::{Context, Result};
use serde::Serialize;
use std::io::Write;
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::time::Duration;

const IO_TIMEOUT: Duration = Duration::from_secs(10);

pub fn serve(bind: &str, db: &Path, verbose: bool) -> Result<()> {
    let store = Store::open(db)?;
    let listener = TcpListener::bind(bind).with_context(|| format!("cannot listen on {bind}"))?;
    println!("listening on http://{}", listener.local_addr()?);
    std::io::stdout().flush()?;

    for connection in listener.incoming() {
        let mut stream = match connection {
            Ok(s) => s,
            Err(e) => {
                eprintln!("stele: accept: {e}");
                continue;
            }
        };
        let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
        let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
        if let Err(e) = handle(&store, &mut stream, verbose) {
            eprintln!("stele: {e:#}");
        }
    }
    Ok(())
}

fn handle(store: &Store, stream: &mut TcpStream, verbose: bool) -> Result<()> {
    let request = match http::read_request(stream, api::MAX_REQUEST) {
        Ok(r) => r,
        Err(ReadError::Io(e)) => {
            // A client that hangs up is not an event worth a line in the log.
            return if matches!(
                e.kind(),
                std::io::ErrorKind::UnexpectedEof
                    | std::io::ErrorKind::ConnectionReset
                    | std::io::ErrorKind::BrokenPipe
                    | std::io::ErrorKind::TimedOut
                    | std::io::ErrorKind::WouldBlock
            ) {
                Ok(())
            } else {
                Err(e.into())
            };
        }
        Err(ReadError::Bad(why)) => {
            http::write_response(stream, 400, &json(&ErrorResponse { error: why.into() }))?;
            return Ok(());
        }
        Err(ReadError::TooLarge) => {
            http::write_response(
                stream,
                413,
                &json(&ErrorResponse {
                    error: format!("request body over {} bytes", api::MAX_REQUEST),
                }),
            )?;
            return Ok(());
        }
    };

    let reply = match route(store, &request) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("stele: request failed: {e:#}");
            Reply::error(500, "internal error")
        }
    };
    if verbose {
        eprintln!("{} {} -> {}", request.method, request.path, reply.status);
    }
    http::write_response(stream, reply.status, &reply.body)?;
    Ok(())
}

struct Reply {
    status: u16,
    body: Vec<u8>,
}

impl Reply {
    fn json<T: Serialize>(status: u16, value: &T) -> Self {
        Self {
            status,
            body: json(value),
        }
    }

    fn error(status: u16, message: impl Into<String>) -> Self {
        Self::json(
            status,
            &ErrorResponse {
                error: message.into(),
            },
        )
    }

    fn empty(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
        }
    }
}

fn json<T: Serialize>(value: &T) -> Vec<u8> {
    serde_json::to_vec(value).unwrap_or_else(|_| b"{\"error\":\"internal error\"}".to_vec())
}

fn route(store: &Store, request: &Request) -> Result<Reply> {
    let (method, path) = (request.method.as_str(), request.path.as_str());
    let now = time::now();

    match (method, path) {
        ("GET", "/health") => Ok(Reply::json(200, &OkResponse { ok: true })),

        ("GET", "/me") => {
            let Some(mailbox) = authenticated(store, request)? else {
                return Ok(unauthorized());
            };
            Ok(Reply::json(200, &MeResponse { mailbox }))
        }

        ("GET", "/peers") => {
            if authenticated(store, request)?.is_none() {
                return Ok(unauthorized());
            }
            Ok(Reply::json(
                200,
                &PeersResponse {
                    mailboxes: store.mailboxes()?,
                },
            ))
        }

        ("GET", "/status") => {
            let Some(mailbox) = authenticated(store, request)? else {
                return Ok(unauthorized());
            };
            let pending = store.pending(&mailbox, time::now())?;
            Ok(Reply::json(200, &StatusResponse { mailbox, pending }))
        }

        ("POST", "/messages") => send(store, request),
        ("GET", "/messages") => inbox(store, request, now),
        ("GET", p) if p.starts_with("/messages/") => message_detail(store, request, now),
        ("POST", p) if p.starts_with("/messages/") && p.ends_with("/ack") => {
            acknowledge(store, request, now)
        }
        ("POST", p) if p.starts_with("/messages/") && p.ends_with("/discard") => {
            discard(store, request, now)
        }

        (_, "/messages") | (_, "/me") | (_, "/peers") | (_, "/status") => {
            Ok(Reply::error(405, "method not allowed"))
        }
        _ => Ok(Reply::error(404, "no such endpoint")),
    }
}

fn send(store: &Store, request: &Request) -> Result<Reply> {
    let Some(from) = authenticated(store, request)? else {
        return Ok(unauthorized());
    };
    let payload: SendRequest = match serde_json::from_slice(&request.body) {
        Ok(p) => p,
        Err(e) => return Ok(Reply::error(400, format!("invalid JSON: {e}"))),
    };
    let to = payload.to.trim().to_string();
    let project = payload.project.trim().to_string();
    let client_id = payload.client_id.trim().to_string();

    if let Err(why) = check_field(&to, api::MAX_MAILBOX, "to") {
        return Ok(Reply::error(400, why));
    }
    if let Err(why) = check_field(&project, api::MAX_PROJECT, "project") {
        return Ok(Reply::error(400, why));
    }
    if let Err(why) = check_field(&client_id, api::MAX_CLIENT_ID, "client_id") {
        return Ok(Reply::error(400, why));
    }
    if let Some(c) = api::first_invalid(&to, api::MAILBOX_CHARS) {
        return Ok(Reply::error(
            400,
            format!("to has {c:?}; use letters, digits and . _ - @"),
        ));
    }
    if let Some(c) = api::first_invalid(&project, api::PROJECT_CHARS) {
        return Ok(Reply::error(
            400,
            format!("project has {c:?}; use letters, digits and . _ - @ /"),
        ));
    }
    if let Some(c) = api::first_invalid(&client_id, api::CLIENT_ID_CHARS) {
        return Ok(Reply::error(
            400,
            format!("client_id has {c:?}; use letters, digits and . _ : @ -"),
        ));
    }
    if payload.body.trim().is_empty() {
        return Ok(Reply::error(400, "the body is empty"));
    }
    if payload.body.len() > api::MAX_BODY {
        return Ok(Reply::error(
            413,
            format!("body over {} bytes", api::MAX_BODY),
        ));
    }
    if !store.mailbox_exists(&to)? {
        return Ok(Reply::error(404, format!("there is no mailbox {to:?}")));
    }

    let now = time::now();
    store.purge(now)?;
    let id = random_hex(16)?;
    match store.insert(crate::store::NewMessage {
        id: &id,
        from: &from,
        to: &to,
        client_id: &client_id,
        project: &project,
        body: &payload.body,
        now,
    })? {
        InsertOutcome::New(id) => Ok(Reply::json(
            201,
            &SendResponse {
                id,
                duplicate: false,
            },
        )),
        InsertOutcome::Duplicate(id) => Ok(Reply::json(
            200,
            &SendResponse {
                id,
                duplicate: true,
            },
        )),
        InsertOutcome::Full => Ok(Reply::error(
            429,
            format!(
                "{to:?} has {} unread messages; try again later",
                api::MAX_PENDING
            ),
        )),
    }
}

fn inbox(store: &Store, request: &Request, now: i64) -> Result<Reply> {
    let Some(mailbox) = authenticated(store, request)? else {
        return Ok(unauthorized());
    };
    store.purge(now)?;
    let limit = request
        .query_get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .map(|v| v.clamp(1, api::MAX_PENDING))
        .unwrap_or(50);
    let messages = store.unread(&mailbox, limit, now, api::MAX_INBOX_BYTES)?;
    Ok(Reply::json(200, &MessagesResponse { messages }))
}

fn acknowledge(store: &Store, request: &Request, now: i64) -> Result<Reply> {
    let Some(mailbox) = authenticated(store, request)? else {
        return Ok(unauthorized());
    };
    let id = request
        .path
        .trim_start_matches("/messages/")
        .trim_end_matches("/ack")
        .trim_end_matches('/');
    if id.is_empty() || id.len() > 64 || id.contains('/') {
        return Ok(Reply::error(400, "bad message id"));
    }
    if !store.ack(&mailbox, id, now)? {
        return Ok(Reply::error(404, "no such message for this mailbox"));
    }
    Ok(Reply::empty(204))
}

fn message_detail(store: &Store, request: &Request, now: i64) -> Result<Reply> {
    let Some(mailbox) = authenticated(store, request)? else {
        return Ok(unauthorized());
    };
    let id = request.path.trim_start_matches("/messages/");
    if id.is_empty() || id.len() > 64 || id.contains('/') {
        return Ok(Reply::error(400, "bad message id"));
    }
    match store.message(&mailbox, id, now)? {
        Some(message) => Ok(Reply::json(200, &message)),
        None => Ok(Reply::error(
            404,
            "no pending message with that id for this mailbox",
        )),
    }
}

/// A deliberate loss: acknowledge one message without writing a draft. The
/// row stays until its TTL, so the queue keeps the record of what was thrown
/// away; only the recipient's token can do it.
fn discard(store: &Store, request: &Request, now: i64) -> Result<Reply> {
    let Some(mailbox) = authenticated(store, request)? else {
        return Ok(unauthorized());
    };
    let id = request
        .path
        .trim_start_matches("/messages/")
        .trim_end_matches("/discard")
        .trim_end_matches('/');
    if id.is_empty() || id.len() > 64 || id.contains('/') {
        return Ok(Reply::error(400, "bad message id"));
    }
    if !store.ack(&mailbox, id, now)? {
        return Ok(Reply::error(404, "no such message for this mailbox"));
    }
    Ok(Reply::empty(204))
}

fn check_field(value: &str, max: usize, name: &str) -> std::result::Result<(), String> {
    if value.is_empty() {
        return Err(format!("{name} is empty"));
    }
    if value.len() > max {
        return Err(format!("{name} is over {max} bytes"));
    }
    if value.chars().any(|c| c.is_control()) {
        return Err(format!("{name} has control characters"));
    }
    Ok(())
}

fn authenticated(store: &Store, request: &Request) -> Result<Option<String>> {
    let Some(header) = request.header("authorization") else {
        return Ok(None);
    };
    let Some((scheme, token)) = header.split_once(' ') else {
        return Ok(None);
    };
    if !scheme.eq_ignore_ascii_case("bearer") {
        return Ok(None);
    }
    let token = token.trim();
    if token.is_empty() {
        return Ok(None);
    }
    store.mailbox_for_hash(&sha256_hex(token))
}

fn unauthorized() -> Reply {
    Reply::error(401, "missing or unknown token")
}
