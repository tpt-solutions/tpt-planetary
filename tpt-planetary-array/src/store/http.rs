//! A read-only store over plain HTTP/1.1, with byte-range requests.
//!
//! [`HttpStore`] maps a key to `base + key` and issues `GET` requests with
//! optional `Range` headers through the shared client in `transport`.
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
use alloc::vec;
use alloc::vec::Vec;

use super::transport::{Request, Response, exchange};
use super::{Store, validate_key};
use crate::error::{Error, Result};

/// A read-only store over `http://` URLs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpStore {
    host: String,
    port: u16,
    /// Path prefix ending in `/`, for example `/data/store/`.
    base_path: String,
}

/// Splits an `http://host[:port]/path` URL into host, port and path.
pub(crate) fn split_http_url(url: &str) -> Result<(String, u16, String)> {
    if url.starts_with("https://") {
        return Err(Error::Unsupported(String::from(
            "https:// URLs need TLS, which this crate does not include",
        )));
    }
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| Error::InvalidMetadata(format!("URL '{url}' must start with http://")))?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    if authority.is_empty() || authority.contains('@') {
        return Err(Error::InvalidMetadata(format!(
            "invalid host in URL '{url}'"
        )));
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => {
            let port: u16 = p
                .parse()
                .map_err(|_| Error::InvalidMetadata(format!("invalid port in URL '{url}'")))?;
            (h, port)
        }
        None => (authority, 80),
    };
    if host.is_empty() {
        return Err(Error::InvalidMetadata(format!(
            "invalid host in URL '{url}'"
        )));
    }
    Ok((host.to_string(), port, path.to_string()))
}

impl HttpStore {
    /// Creates a store for `base_url`, such as `http://example.org:8080/data/`.
    /// Keys are appended to the path.
    pub fn new(base_url: &str) -> Result<Self> {
        let (host, port, mut base_path) = split_http_url(base_url)?;
        if !base_path.ends_with('/') {
            base_path.push('/');
        }
        Ok(Self {
            host,
            port,
            base_path,
        })
    }

    /// The request target (path and query) for `key`.
    fn target(&self, key: &str) -> String {
        format!("{}{}", self.base_path, key)
    }

    /// Sends a `GET` for `key`, with an optional `Range` header value.
    fn request(&self, key: &str, range: Option<&str>) -> Result<Response> {
        validate_key(key)?;
        let target = self.target(key);
        let headers: Vec<(String, String)> = match range {
            Some(r) => vec![(String::from("Range"), r.to_string())],
            None => Vec::new(),
        };
        exchange(&Request {
            host: &self.host,
            port: self.port,
            method: "GET",
            target: &target,
            headers: &headers,
            body: &[],
        })
    }
}

/// True for status codes that mean "try again later".
pub(crate) fn transient_status(status: u16) -> bool {
    matches!(status, 429 | 500 | 502 | 503 | 504)
}

impl Store for HttpStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        let Response { status, body, .. } = self.request(key, None)?;
        match status {
            200 => Ok(Some(body)),
            404 | 410 => Ok(None),
            s if transient_status(s) => Err(Error::Transient(format!("HTTP {s} for '{key}'"))),
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
            s if transient_status(s) => Err(Error::Transient(format!("HTTP {s} for '{key}'"))),
            other => Err(Error::Io(format!("HTTP {other} for '{key}'"))),
        }
    }

    fn size(&self, key: &str) -> Result<Option<u64>> {
        // Ask for the first byte: the reply names the whole object's length.
        let Response {
            status,
            total,
            body,
            ..
        } = self.request(key, Some("bytes=0-0"))?;
        match status {
            206 => total
                .map(Some)
                .ok_or_else(|| Error::Io(format!("no Content-Range length for '{key}'"))),
            200 => Ok(Some(body.len() as u64)),
            // An empty object cannot satisfy a byte range.
            416 => Ok(Some(total.unwrap_or(0))),
            404 | 410 => Ok(None),
            s if transient_status(s) => Err(Error::Transient(format!("HTTP {s} for '{key}'"))),
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
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::thread;

    type Responder = alloc::boxed::Box<dyn Fn(&str) -> Vec<u8> + Send>;

    /// Serves `responses` in order, one per connection, and returns the base URL.
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
        alloc::boxed::Box::new(move |_| {
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
            alloc::boxed::Box::new(|_| {
                b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n".to_vec()
            }),
        ]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.get("x").unwrap(), Some(b"hello".to_vec()));
        assert_eq!(store.get("missing").unwrap(), None);
        server.join().unwrap();
    }

    #[test]
    fn get_chunked_body() {
        let (url, server) = serve(vec![alloc::boxed::Box::new(|_| {
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5;ext=1\r\npedia\r\n0\r\n\r\n"
                .to_vec()
        })]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.get("c").unwrap(), Some(b"Wikipedia".to_vec()));
        server.join().unwrap();
    }

    #[test]
    fn range_request_uses_partial_content() {
        let (url, server) = serve(vec![alloc::boxed::Box::new(|req| {
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
    fn size_reads_the_total_from_content_range() {
        let (url, server) = serve(vec![alloc::boxed::Box::new(|req| {
            assert!(req.contains("Range: bytes=0-0\r\n"), "request: {req}");
            b"HTTP/1.1 206 Partial Content\r\nContent-Range: bytes 0-0/4096\r\nContent-Length: 1\r\n\r\nx"
                .to_vec()
        })]);
        let store = HttpStore::new(&url).unwrap();
        assert_eq!(store.size("k").unwrap(), Some(4096));
        server.join().unwrap();
    }

    #[test]
    fn transient_status_and_read_only_operations() {
        let (url, server) = serve(vec![alloc::boxed::Box::new(|_| {
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
    fn rejects_unsafe_keys_before_connecting() {
        let store = HttpStore::new("http://127.0.0.1:1/").unwrap();
        assert!(store.get("../etc").is_err());
    }
}
