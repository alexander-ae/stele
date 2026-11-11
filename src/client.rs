//! The commands that talk to the mailbox.

use crate::api::{
    ErrorResponse, MeResponse, Message, MessagesResponse, PeersResponse, SendRequest, SendResponse,
};
use crate::http::{self, Endpoint};
use anyhow::{bail, Context, Result};
use serde::de::DeserializeOwned;

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
            .with_context(|| format!("talking to {}", self.endpoint.authority))
    }

    fn json<T: DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<&[u8]>,
    ) -> Result<T> {
        let (status, raw) = self.call(method, path, body)?;
        if !(200..300).contains(&status) {
            bail!(describe_error(status, &raw));
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

    pub fn send(&self, request: &SendRequest) -> Result<SendResponse> {
        let body = serde_json::to_vec(request)?;
        self.json("POST", "/messages", Some(&body))
    }

    pub fn pull(&self, limit: i64) -> Result<Vec<Message>> {
        let path = format!("/messages?unread=1&limit={limit}");
        let response: MessagesResponse = self.json("GET", &path, None)?;
        Ok(response.messages)
    }

    pub fn ack(&self, id: &str) -> Result<()> {
        let (status, raw) = self.call("POST", &format!("/messages/{id}/ack"), Some(b""))?;
        if !(200..300).contains(&status) {
            bail!(describe_error(status, &raw));
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
