//! The commands that talk to the mailbox.

use crate::api::{
    ErrorResponse, MeResponse, Message, MessagesResponse, PeersResponse, SendRequest, SendResponse,
    StatusResponse,
};
use crate::failure::Failure;
use crate::http::{self, Endpoint};
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;
use std::time::Duration;

/// A send is idempotent on the server because the request carries the same
/// `client_id` on every attempt, so a dropped connection is worth retrying.
/// Three attempts is enough for a network hiccup; the whole command still
/// fails fast when the mailbox is truly down.
const SEND_ATTEMPTS: usize = 3;
const RETRY_BACKOFF: Duration = Duration::from_millis(100);

pub struct Api {
    pub endpoint: Endpoint,
    token: String,
}

impl Api {
    pub fn new(url: &str, token: String) -> Result<Self> {
        crate::config::validate_url(url)?;
        Ok(Self {
            endpoint: Endpoint::parse(url)?,
            token,
        })
    }

    fn call(&self, method: &str, path: &str, body: Option<&[u8]>) -> Result<(u16, Vec<u8>)> {
        http::call(&self.endpoint, method, path, Some(&self.token), body)
            .map_err(|error| Failure::unreachable(format!("{error:#}")).into())
    }

    fn json<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<T> {
        let (status, raw) = self.call(method, path, body)?;
        if !(200..300).contains(&status) {
            return Err(Failure::refused(describe_error(status, &raw)).into());
        }
        serde_json::from_slice(&raw).with_context(|| {
            format!("the server answered HTTP {status} with something that is not JSON")
        })
    }

    pub fn me(&self) -> Result<MeResponse> {
        self.json("GET", "/me", None)
    }

    pub fn peers(&self) -> Result<PeersResponse> {
        self.json("GET", "/peers", None)
    }

    pub fn status(&self) -> Result<StatusResponse> {
        self.json("GET", "/status", None)
    }

    pub fn message(&self, id: &str) -> Result<Message> {
        self.json("GET", &format!("/messages/{id}"), None)
    }

    pub fn send(&self, request: &SendRequest) -> Result<SendResponse> {
        let body = serde_json::to_vec(request)?;
        // Only the transport is retried. Once the server answers, that answer
        // is the result: a 4xx is a decision, not a hiccup.
        let (status, raw) = with_retries(SEND_ATTEMPTS, |attempt| {
            if attempt > 0 {
                std::thread::sleep(RETRY_BACKOFF * attempt as u32);
            }
            self.call("POST", "/messages", Some(&body))
        })?;
        if !(200..300).contains(&status) {
            return Err(Failure::refused(describe_error(status, &raw)).into());
        }
        serde_json::from_slice(&raw).with_context(|| {
            format!("the server answered HTTP {status} with something that is not JSON")
        })
    }

    pub fn pull(&self, limit: i64) -> Result<Vec<Message>> {
        let path = format!("/messages?unread=1&limit={limit}");
        let response: MessagesResponse = self.json("GET", &path, None)?;
        Ok(response.messages)
    }

    pub fn ack(&self, id: &str) -> Result<()> {
        let (status, raw) = self.call("POST", &format!("/messages/{id}/ack"), Some(b""))?;
        if !(200..300).contains(&status) {
            return Err(Failure::refused(describe_error(status, &raw)).into());
        }
        Ok(())
    }

    /// Mark a message as delivered without writing a draft. Idempotent, like
    /// `ack`; the server keeps the row until its TTL.
    pub fn discard(&self, id: &str) -> Result<()> {
        let (status, raw) = self.call("POST", &format!("/messages/{id}/discard"), Some(b""))?;
        if !(200..300).contains(&status) {
            return Err(Failure::refused(describe_error(status, &raw)).into());
        }
        Ok(())
    }
}

fn describe_error(status: u16, raw: &[u8]) -> String {
    let detail = serde_json::from_slice::<ErrorResponse>(raw)
        .map(|e| e.error)
        .unwrap_or_else(|_| String::from_utf8_lossy(raw).trim().to_string());
    let hint = if status == 401 {
        " (check the token: `stele login`)"
    } else {
        ""
    };
    if detail.is_empty() {
        format!("the server answered HTTP {status}{hint}")
    } else {
        format!("the server answered HTTP {status}: {detail}{hint}")
    }
}

/// Runs an operation up to `attempts` times. The caller decides what counts
/// as retryable by where it puts the boundary; here it wraps only the
/// transport call, never an HTTP answer.
fn with_retries<T>(attempts: usize, mut operation: impl FnMut(usize) -> Result<T>) -> Result<T> {
    let mut last = None;
    for attempt in 0..attempts {
        match operation(attempt) {
            Ok(value) => return Ok(value),
            Err(error) => last = Some(error),
        }
    }
    match last {
        Some(error) => Err(error),
        None => bail!("no attempt was made"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn retries_until_success() {
        let calls = Cell::new(0);
        let result: Result<u8> = with_retries(3, |_| {
            calls.set(calls.get() + 1);
            if calls.get() < 3 {
                bail!("connection reset")
            } else {
                Ok(7)
            }
        });
        assert_eq!(result.unwrap(), 7);
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn returns_the_last_error_after_every_attempt() {
        let calls = Cell::new(0);
        let result: Result<u8> = with_retries(3, |_| {
            calls.set(calls.get() + 1);
            bail!("connection refused")
        });
        assert!(result.is_err());
        assert_eq!(calls.get(), 3);
    }
}
