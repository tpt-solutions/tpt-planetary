//! A read-only store over plain HTTP/1.1, with byte-range requests.
//!
//! [`HttpStore`] maps a key to `base + key` and issues `GET` requests over a
//! `std::net::TcpStream`. Each request uses `Connection: close`, so one
//! response is read to the end of the stream. Supported framing: `Content-Length`,
//! `Transfer-Encoding: chunked`, and read-until-close.
//!
//! Limits:
//! - Only `http://` URLs are accepted. TLS (`https://`) needs a TLS
//!   implementation, which this crate does not include; such URLs are rejected
//!   with [`Error::Unsupported`].
//! - Listing is not possible over plain HTTP, so [`Store::list`] fails with
//!   [`Error::Unsupported`]. Writes and deletes are rejected the same way.
//! - Redirects are not followed.
//! - Status codes 429, 500, 502, 503 and 504 map to [`Error::Transient`], so
//!   [`RetryStore`](crate::retry::RetryStore) retries them.
//!
//! No async runtime is involved; every call blocks the calling thread.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use super::{Store, validate_key};
use crate::error::{Error, Result};

/// Connect, read and write timeout for each request.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Largest response body accepted, to bound memory use on hostile servers.
const MAX_BODY: usize = 1 << 31;

/// A read-only store over `http://` URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpStore {
    host: String,
    port: u16,
    /// Path prefix ending in `/`, for example `/data/store/`.
    base_path: String,
}

impl HttpStore {
    /// Creates a store for `base_url`, such as `http://example.org:8080/data/`.
    /// Keys are appended to the path.
    pub fn new(base_url: &str) -> Result<Self> {
        if base_url.starts_with("https://") {
            return Err(Error::Unsupported(String::from(
                "https:// stores need TLS, which this crate does not include",
            )));
        }
        let rest = base_url.strip_prefix("http://").ok_or_else(|| {
            Error::InvalidMetadata(format!("store URL '{base_url}' must start with http://"))
        })?;
        let (authority, path) = match rest.find('/') {
            Some(i) => (&rest[..i], &rest[i..]),
            None => (rest, "/"),
        };
        if authority.is_empty() || authority.contains('@') {
            return Err(Error::InvalidMetadata(format!(
                "invalid host in store URL '{base_url}'"
            )));
        }
        let (host, port) = match authority.rsplit_once(':') {
            Some((h, p)) => {
                let port: u16 = p.parse().map_err(|_| {
                    Error::InvalidMetadata(format!("invalid port in store URL '{base_url}'"))
                })?;
                (h, port)
            }
            None => (authority, 80),
        };
        if host.is_empty() {
            return Err(Error::InvalidMetadata(format!(
                "invalid host in store URL '{base_url}'"
            )));
        }
        let mut base_path = String::from(path);
        if !base_path.ends_with('/') {
            base_path.push('/');
        }
        Ok(Self {
            host: host.to_string(),
            port,
            base_path,
        })
    }

    /// The request target (path and query) for `key`.
    fn target(&self, key: &str) -> String {
        format!("{}{}", self.base_path, key)
    }

    /// Sends a `GET` for `key`, with an optional `Range` header value, and
    /// returns the status code and the decoded body.
    fn request(&self, key: &str, range: Option<&str>) -> Result<Response> {
        validate_key(key)?;
        let mut stream = self.connect()?;
        let mut req = format!(
            "GET {} HTTP/1.1\r\nHost: {}\r\nConnection: close\r\nAccept-Encoding: identity\r\n",
            self.target(key),
            self.host_header()
        );
        if let Some(r) = range {
            req.push_str(&format!("Range: {r}\r\n"));
        }
        req.push_str("\r\n");
        stream.write_all(req.as_bytes()).map_err(io_error)?;
        let mut raw = Vec::new();
        let mut buf = [0u8; 16 * 1024];
        loop {
            match stream.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    raw.extend_from_slice(&buf[..n]);
                    if raw.len() > MAX_BODY {
                        return Err(Error::Unsupported(String::from(
                            "HTTP response larger than the store limit",
                        )));
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(io_error(e)),
            }
        }
        parse_response(&raw)
    }

    fn host_header(&self) -> String {
        if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        }
    }

    fn connect(&self) -> Result<TcpStream> {
        use std::net::ToSocketAddrs;
        let addr = (self.host.as_str(), self.port)
            .to_socket_addrs()
            .map_err(io_error)?
            .next()
            .ok_or_else(|| Error::Io(format!("cannot resolve host '{}'", self.host)))?;
        let stream = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(io_error)?;
        stream.set_read_timeout(Some(TIMEOUT)).map_err(io_error)?;
        stream.set_write_timeout(Some(TIMEOUT)).map_err(io_error)?;
        Ok(stream)
    }
}

/// Maps an I/O error to a transient or permanent store error.
fn io_error(e: io::Error) -> Error {
    match e.kind() {
        io::ErrorKind::TimedOut
        | io::ErrorKind::WouldBlock
        | io::ErrorKind::ConnectionReset
        | io::ErrorKind::ConnectionAborted
        | io::ErrorKind::BrokenPipe
        | io::ErrorKind::UnexpectedEof => Error::Transient(e.to_string()),
        _ => Error::Io(e.to_string()),
    }
}

/// Parses a complete HTTP/1.x response. Returns the status code and body.
fn parse_response(raw: &[u8]) -> Result<Response> {
    let bad = || Error::Io(String::from("malformed HTTP response"));
    let head_end = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(bad)?;
    let head = core::str::from_utf8(&raw[..head_end]).map_err(|_| bad())?;
    let mut lines = head.split("\r\n");
    let status_line = lines.next().ok_or_else(bad)?;
    let mut parts = status_line.splitn(3, ' ');
    let version = parts.next().ok_or_else(bad)?;
    if !version.starts_with("HTTP/1.") {
        return Err(bad());
    }
    let status: u16 = parts.next().and_then(|s| s.parse().ok()).ok_or_else(bad)?;

    let mut content_length: Option<usize> = None;
    let mut total: Option<u64> = None;
    let mut chunked = false;
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = Some(value.parse().map_err(|_| bad())?);
        } else if name.eq_ignore_ascii_case("content-range") {
            // `bytes a-b/total`, or `bytes */total` on 416.
            total = value.rsplit_once('/').and_then(|(_, t)| t.parse().ok());
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }

    let body = &raw[head_end + 4..];
    let body = if chunked {
        decode_chunked(body)?
    } else {
        match content_length {
            Some(n) if n <= body.len() => body[..n].to_vec(),
            Some(_) => return Err(Error::Io(String::from("truncated HTTP response body"))),
            None => body.to_vec(),
        }
    };
    Ok(Response {
        status,
        total,
        body,
    })
}

/// A parsed response: status, the total object length from `Content-Range`
/// (if sent), and the body.
struct Response {
    status: u16,
    total: Option<u64>,
    body: Vec<u8>,
}

/// Decodes a `chunked` transfer-coded body.
fn decode_chunked(mut data: &[u8]) -> Result<Vec<u8>> {
    let bad = || Error::Io(String::from("malformed chunked HTTP body"));
    let mut out = Vec::new();
    loop {
        let line_end = data.windows(2).position(|w| w == b"\r\n").ok_or_else(bad)?;
        let size_text = core::str::from_utf8(&data[..line_end]).map_err(|_| bad())?;
        let size_hex = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_hex, 16).map_err(|_| bad())?;
        data = &data[line_end + 2..];
        if size == 0 {
            return Ok(out);
        }
        if data.len() < size + 2 {
            return Err(bad());
        }
        if out.len() + size > MAX_BODY {
            return Err(Error::Unsupported(String::from(
                "HTTP response larger than the store limit",
            )));
        }
        out.extend_from_slice(&data[..size]);
        data = &data[size + 2..];
    }
}

impl Store for HttpStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let Response { status, body, .. } = self.request(key, None)?;
        match status {
            200 => Ok(Some(body)),
            404 | 410 => Ok(None),
            429 | 500 | 502 | 503 | 504 => {
                Err(Error::Transient(format!("HTTP {status} for '{key}'")))
            }
            other => Err(Error::Io(format!("HTTP {other} for '{key}'"))),
        }
    }

    fn get_range(&self, key: &str, start: u64, len: u64) -> Result<Option<Vec<u8>>> {
        if len == 0 {
            return self.get(key).map(|found| found.map(|_| Vec::new()));
        }
        let end = start
            .checked_add(len - 1)
            .ok_or_else(|| Error::InvalidSelection(String::from("byte range overflows")))?;
        let Response { status, body, .. } =
            self.request(key, Some(&format!("bytes={start}-{end}")))?;
        match status {
            // Partial content: the body is exactly the requested range.
            206 => {
                if body.len() as u64 > len {
                    return Err(Error::Io(String::from("server sent more than requested")));
                }
                Ok(Some(body))
            }
            // The server ignored the Range header and sent the whole object.
            200 => {
                let s = usize::try_from(start).unwrap_or(usize::MAX).min(body.len());
                let e = usize::try_from(end.saturating_add(1))
                    .unwrap_or(usize::MAX)
                    .min(body.len());
                Ok(Some(body[s..e].to_vec()))
            }
            404 | 410 => Ok(None),
            416 => Err(Error::InvalidSelection(format!(
                "byte range {start}+{len} not satisfiable for '{key}'"
            ))),
            429 | 500 | 502 | 503 | 504 => {
                Err(Error::Transient(format!("HTTP {status} for '{key}'")))
            }
            other => Err(Error::Io(format!("HTTP {other} for '{key}'"))),
        }
    }

    fn size(&self, key: &str) -> Result<Option<u64>> {
        // Ask for the first byte: the reply names the whole object's length.
        let Response {
            status,
            total,
            body,
        } = self.request(key, Some("bytes=0-0"))?;
        match status {
            206 => total
                .map(Some)
                .ok_or_else(|| Error::Io(format!("no Content-Range length for '{key}'"))),
            200 => Ok(Some(body.len() as u64)),
            // An empty object cannot satisfy a byte range.
            416 => Ok(Some(total.unwrap_or(0))),
            404 | 410 => Ok(None),
            429 | 500 | 502 | 503 | 504 => {
                Err(Error::Transient(format!("HTTP {status} for '{key}'")))
            }
            other => Err(Error::Io(format!("HTTP {other} for '{key}'"))),
        }
    }

    fn set(&self, _key: &str, _value: &[u8]) -> Result<()> {
        Err(Error::Unsupported(String::from("HTTP store is read-only")))
    }

    fn delete(&self, _key: &str) -> Result<()> {
        Err(Error::Unsupported(String::from("HTTP store is read-only")))
    }

    fn list(&self, _prefix: &str) -> Result<Vec<String>> {
        Err(Error::Unsupported(String::from(
            "plain HTTP cannot list keys",
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::thread;

    type Responder = Box<dyn Fn(&str) -> Vec<u8> + Send>;

    /// Serves `responses` in order, one per connection, and returns the base URL.
    /// Each response is a closure of the request text, returning the raw bytes.
    fn serve(responses: Vec<Responder>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            for respond in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let text = String::from_utf8_lossy(&req).into_owned();
                stream.write_all(&respond(&text)).unwrap();
            }
        });
        (format!("http://127.0.0.1:{port}/store/"), handle)
    }

    fn ok_body(body: &'static [u8]) -> Responder {
        Box::new(move |_| {
            let mut out =
                format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
            out.extend_from_slice(body);
            out
        })
    }

    #[test]
    fn url_parsing() {
        let s = HttpStore::new("http://example.org:8080/data").unwrap();
        assert_eq!(s.host, "example.org");
        assert_eq!(s.port, 8080);
        assert_eq!(s.target("a/b"), "/data/a/b");
        let d = HttpStore::new("http://h/").unwrap();
        assert_eq!((d.port, d.base_path.as_str()), (80, "/"));
        assert!(HttpStore::new("https://h/").is_err());
        assert!(HttpStore::new("ftp://h/").is_err());
        assert!(HttpStore::new("http:///x").is_err());
        assert!(HttpStore::new("http://h:notaport/").is_err());
    }

    #[test]
    fn get_with_content_length_and_missing_key() {
        let (url, server) = serve(vec![
            ok_body(b"hello"),
            Box::new(|_| b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()),
        ]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.get("x").unwrap(), Some(b"hello".to_vec()));
        assert_eq!(store.get("missing").unwrap(), None);
        server.join().unwrap();
    }

    #[test]
    fn get_chunked_body() {
        let (url, server) = serve(vec![Box::new(|_| {
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n"
                .to_vec()
        })]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.get("c").unwrap(), Some(b"Wikipedia".to_vec()));
        server.join().unwrap();
    }

    #[test]
    fn range_request_uses_partial_content() {
        let (url, server) = serve(vec![Box::new(|req| {
            assert!(req.contains("Range: bytes=2-4\r\n"), "request: {req}");
            b"HTTP/1.1 206 Partial Content\r\nContent-Length: 3\r\n\r\nCDE".to_vec()
        })]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.get_range("k", 2, 3).unwrap(), Some(b"CDE".to_vec()));
        server.join().unwrap();
    }

    #[test]
    fn range_falls_back_when_server_ignores_range() {
        let (url, server) = serve(vec![ok_body(b"ABCDEFGH")]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.get_range("k", 3, 2).unwrap(), Some(b"DE".to_vec()));
        server.join().unwrap();
    }

    #[test]
    fn transient_status_and_read_only_operations() {
        let (url, server) = serve(vec![Box::new(|_| {
            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\n\r\n".to_vec()
        })]);
        let store = HttpStore::new(&url).unwrap();
        assert!(matches!(store.get("k"), Err(Error::Transient(_))));
        assert!(matches!(store.set("k", b"x"), Err(Error::Unsupported(_))));
        assert!(matches!(store.delete("k"), Err(Error::Unsupported(_))));
        assert!(matches!(store.list(""), Err(Error::Unsupported(_))));
        server.join().unwrap();
    }

    #[test]
    fn size_reads_the_total_from_content_range() {
        let (url, server) = serve(vec![Box::new(|req| {
            assert!(req.contains("Range: bytes=0-0\r\n"), "request: {req}");
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/4096\r\nContent-Length: 1\r\n\r\nx"
                .to_vec()
        })]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.size("k").unwrap(), Some(4096));
        server.join().unwrap();
    }

    #[test]
    fn rejects_unsafe_keys_before_connecting() {
        let store = HttpStore::new("http://127.0.0.1:1/").unwrap();
        assert!(store.get("../etc").is_err());
    }
}
