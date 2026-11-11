//! The HTTP/1.1 that stele actually needs: one request per connection, no
//! chunked encoding, no keep-alive, a cap on everything. `httparse` does the
//! parsing and the rest is a few dozen lines, which is why neither tiny_http
//! nor a client crate is in the tree.

use anyhow::{bail, Context, Result};
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

const MAX_HEAD: usize = 16 * 1024;
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);

pub struct Request {
    pub method: String,
    pub path: String,
    pub query: Vec<(String, String)>,
    headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    pub fn query_get(&self, name: &str) -> Option<&str> {
        self.query
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }
}

pub enum ReadError {
    Io(std::io::Error),
    Bad(&'static str),
    TooLarge,
}

impl From<std::io::Error> for ReadError {
    fn from(e: std::io::Error) -> Self {
        ReadError::Io(e)
    }
}

/// Read one request. Anything that is not a plain `Content-Length` request is
/// refused: this server has no reason to accept chunked bodies or upgrades.
pub fn read_request(stream: &mut TcpStream, max_body: usize) -> Result<Request, ReadError> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_HEAD {
            return Err(ReadError::Bad("request headers too large"));
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(ReadError::Bad("connection closed before the request"));
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Request::new(&mut headers);
    match parsed.parse(&buf[..head_end]) {
        Ok(httparse::Status::Complete(_)) => {}
        Ok(httparse::Status::Partial) => return Err(ReadError::Bad("incomplete request")),
        Err(_) => return Err(ReadError::Bad("malformed request")),
    }
    let method = parsed.method.unwrap_or_default().to_string();
    let target = parsed.path.unwrap_or_default().to_string();
    let mut name_values = Vec::with_capacity(parsed.headers.len());
    for h in parsed.headers.iter() {
        name_values.push((
            h.name.to_ascii_lowercase(),
            String::from_utf8_lossy(h.value).trim().to_string(),
        ));
    }

    if name_values.iter().any(|(n, _)| n == "transfer-encoding") {
        return Err(ReadError::Bad("chunked bodies are not supported"));
    }
    let lengths: Vec<&(String, String)> = name_values
        .iter()
        .filter(|(n, _)| n == "content-length")
        .collect();
    if lengths.len() > 1 {
        // Two different lengths are how request smuggling starts; there is no
        // reason to accept even two identical ones.
        return Err(ReadError::Bad("duplicate Content-Length"));
    }
    let content_length = lengths
        .first()
        .map(|(_, v)| v.parse::<usize>())
        .transpose()
        .map_err(|_| ReadError::Bad("bad Content-Length"))?
        .unwrap_or(0);
    if content_length > max_body {
        return Err(ReadError::TooLarge);
    }

    let mut body = buf[head_end..].to_vec();
    while body.len() < content_length {
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            return Err(ReadError::Bad("connection closed before the body"));
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > max_body {
            return Err(ReadError::TooLarge);
        }
    }
    body.truncate(content_length);

    let (path, query) = split_query(&target);
    Ok(Request {
        method,
        path,
        query,
        headers: name_values,
        body,
    })
}

fn split_query(target: &str) -> (String, Vec<(String, String)>) {
    match target.split_once('?') {
        Some((path, qs)) => {
            let mut query = Vec::new();
            for pair in qs.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
                query.push((k.to_string(), v.to_string()));
            }
            (path.to_string(), query)
        }
        None => (target.to_string(), Vec::new()),
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

pub fn write_response(stream: &mut TcpStream, status: u16, body: &[u8]) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        reason(status),
        body.len()
    );
    stream.write_all(head.as_bytes())?;
    stream.write_all(body)?;
    stream.flush()
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        201 => "Created",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        404 => "Not Found",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        _ => "Unknown",
    }
}

/// Where the mailbox lives. Only `http://` and no path: the server is a
/// single origin, and pretending to support URLs it cannot reach would only
/// turn a configuration mistake into a puzzling error later.
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub authority: String,
}

impl Endpoint {
    pub fn parse(url: &str) -> Result<Self> {
        let rest = url.strip_prefix("http://").with_context(|| {
            format!(
                "{url:?} is not an http:// url; stele does not do TLS itself (use a private network or a tunnel)"
            )
        })?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, ""),
        };
        // A trailing slash is something people type; a real base path is not
        // supported, and pretending it is would fail later in a worse place.
        if !path.is_empty() && path != "/" {
            bail!("the url must not have a path (got {url:?})");
        }
        if authority.contains('@') || authority.is_empty() {
            bail!("the url must be http://host[:port] (got {url:?})");
        }
        let (host, port) = parse_authority(authority)?;
        Ok(Self {
            host,
            port,
            authority: authority.to_string(),
        })
    }
}

fn parse_authority(authority: &str) -> Result<(String, u16)> {
    if let Some(rest) = authority.strip_prefix('[') {
        let (host, tail) = rest
            .split_once(']')
            .with_context(|| format!("bad IPv6 address in {authority:?}"))?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().context("bad port")?,
            None if tail.is_empty() => 80,
            None => bail!("bad address {authority:?}"),
        };
        return Ok((host.to_string(), port));
    }
    match authority.rsplit_once(':') {
        Some((host, p)) => {
            if host.is_empty() {
                bail!("bad address {authority:?}");
            }
            Ok((host.to_string(), p.parse().context("bad port")?))
        }
        None => Ok((authority.to_string(), 80)),
    }
}

/// One request, one connection, close. Returns the status and the body.
pub fn call(
    endpoint: &Endpoint,
    method: &str,
    path: &str,
    token: Option<&str>,
    body: Option<&[u8]>,
) -> Result<(u16, Vec<u8>)> {
    let addrs: Vec<_> = (endpoint.host.as_str(), endpoint.port)
        .to_socket_addrs()
        .with_context(|| format!("cannot resolve {}", endpoint.host))?
        .collect();
    let mut last_error = None;
    let mut stream = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, TIMEOUT) {
            Ok(s) => {
                stream = Some(s);
                break;
            }
            Err(e) => last_error = Some(e),
        }
    }
    let mut stream = stream.with_context(|| {
        format!(
            "cannot connect to {} ({})",
            endpoint.authority,
            last_error.map(|e| e.to_string()).unwrap_or_default()
        )
    })?;
    stream.set_read_timeout(Some(TIMEOUT))?;
    stream.set_write_timeout(Some(TIMEOUT))?;

    let payload = body.unwrap_or(b"");
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n",
        endpoint.authority,
        payload.len()
    );
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {token}\r\n"));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(payload)?;
    stream.flush()?;

    let mut buf = Vec::with_capacity(2048);
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        if let Some(pos) = find(&buf, b"\r\n\r\n") {
            break pos + 4;
        }
        if buf.len() > MAX_HEAD {
            bail!("response headers from {} are too large", endpoint.authority);
        }
        let n = stream.read(&mut chunk)?;
        if n == 0 {
            bail!(
                "connection closed before the response from {}",
                endpoint.authority
            );
        }
        buf.extend_from_slice(&chunk[..n]);
    };

    let mut headers = [httparse::EMPTY_HEADER; 32];
    let mut parsed = httparse::Response::new(&mut headers);
    match parsed.parse(&buf[..head_end]) {
        Ok(httparse::Status::Complete(_)) => {}
        Ok(httparse::Status::Partial) => bail!("incomplete response from {}", endpoint.authority),
        Err(e) => bail!("malformed response from {}: {e}", endpoint.authority),
    }
    let status = parsed.code.context("the response has no status code")?;
    if parsed
        .headers
        .iter()
        .any(|h| h.name.eq_ignore_ascii_case("transfer-encoding"))
    {
        bail!("chunked responses are not supported");
    }
    let content_length = parsed
        .headers
        .iter()
        .find(|h| h.name.eq_ignore_ascii_case("content-length"))
        .map(|h| {
            std::str::from_utf8(h.value)
                .ok()
                .and_then(|v| v.trim().parse::<usize>().ok())
                .context("bad Content-Length in the response")
        })
        .transpose()?;

    let mut body = buf[head_end..].to_vec();
    match content_length {
        Some(length) => {
            if length > MAX_RESPONSE {
                bail!("response from {} is too large", endpoint.authority);
            }
            while body.len() < length {
                let n = stream.read(&mut chunk)?;
                if n == 0 {
                    bail!("connection closed before the response body was complete");
                }
                body.extend_from_slice(&chunk[..n]);
                if body.len() > MAX_RESPONSE {
                    bail!("response from {} is too large", endpoint.authority);
                }
            }
            body.truncate(length);
        }
        None => loop {
            let n = stream.read(&mut chunk)?;
            if n == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..n]);
            if body.len() > MAX_RESPONSE {
                bail!("response from {} is too large", endpoint.authority);
            }
        },
    }
    Ok((status, body))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoints_are_strict() {
        let ep = Endpoint::parse("http://203.0.113.10:3900").unwrap();
        assert_eq!(ep.host, "203.0.113.10");
        assert_eq!(ep.port, 3900);
        assert_eq!(ep.authority, "203.0.113.10:3900");

        let ep = Endpoint::parse("http://dell-server").unwrap();
        assert_eq!(ep.port, 80);

        let ep = Endpoint::parse("http://[::1]:3900").unwrap();
        assert_eq!(ep.host, "::1");
        assert_eq!(ep.port, 3900);

        assert!(Endpoint::parse("http://203.0.113.10:3900/").is_ok());
        assert!(Endpoint::parse("https://example.com").is_err());
        assert!(Endpoint::parse("http://example.com/mail").is_err());
        assert!(Endpoint::parse("example.com:3900").is_err());
    }

    #[test]
    fn query_is_split() {
        let (path, query) = split_query("/messages?unread=1&limit=5");
        assert_eq!(path, "/messages");
        assert_eq!(query.len(), 2);
        assert_eq!(query[1], ("limit".to_string(), "5".to_string()));
    }
}
