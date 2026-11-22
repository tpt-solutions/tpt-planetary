//! A minimal blocking HTTP/1.1 client over `std::net::TcpStream`, shared by
//! the HTTP and S3 stores.
//!
//! Each exchange sends `Connection: close` and reads the response to the end
//! of the stream. Supported response framing: `Content-Length`,
//! `Transfer-Encoding: chunked`, and read-until-close. There is no TLS, no
//! redirect handling and no connection reuse.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use crate::error::{Error, Result};

/// Connect, read and write timeout for each exchange.
const TIMEOUT: Duration = Duration::from_secs(30);

/// Largest response body accepted, to bound memory use on hostile servers.
pub(crate) const MAX_BODY: usize = 1 << 31;

/// One request to send.
#[derive(Debug)]
pub(crate) struct Request<'a> {
    /// Host name or address (no scheme).
    pub host: &'a str,
    /// TCP port.
    pub port: u16,
    /// HTTP method, such as `GET`.
    pub method: &'a str,
    /// Request target: path and query, already percent-encoded.
    pub target: &'a str,
    /// Extra headers (name, value). `Host` and `Connection` are added here.
    pub headers: &'a [(String, String)],
    /// Request body; empty for bodiless methods.
    pub body: &'a [u8],
}

/// A parsed response.
#[derive(Debug)]
pub(crate) struct Response {
    /// Status code.
    pub status: u16,
    /// Total object length from `Content-Range`, if sent.
    pub total: Option<u64>,
    /// Decoded body.
    pub body: Vec<u8>,
}

/// Sends `request` and returns the parsed response.
pub(crate) fn exchange(request: &Request<'_>) -> Result<Response> {
    let addr = (request.host, request.port)
        .to_socket_addrs()
        .map_err(io_error)?
        .next()
        .ok_or_else(|| Error::Io(format!("cannot resolve host '{}'", request.host)))?;
    let mut stream = TcpStream::connect_timeout(&addr, TIMEOUT).map_err(io_error)?;
    stream.set_read_timeout(Some(TIMEOUT)).map_err(io_error)?;
    stream.set_write_timeout(Some(TIMEOUT)).map_err(io_error)?;

    let host_header = if request.port == 80 {
        request.host.to_string()
    } else {
        format!("{}:{}", request.host, request.port)
    };
    let mut head = format!(
        "{} {} HTTP/1.1\r\nHost: {host_header}\r\nConnection: close\r\nAccept-Encoding: identity\r\n",
        request.method, request.target
    );
    for (name, value) in request.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if !request.body.is_empty() || request.method == "PUT" || request.method == "POST" {
        head.push_str(&format!("Content-Length: {}\r\n", request.body.len()));
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes()).map_err(io_error)?;
    stream.write_all(request.body).map_err(io_error)?;
    stream.flush().map_err(io_error)?;

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

/// Maps an I/O error to a transient or permanent store error.
pub(crate) fn io_error(e: io::Error) -> Error {
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

/// Parses a complete HTTP/1.x response.
pub(crate) fn parse_response(raw: &[u8]) -> Result<Response> {
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
        let name = name.trim().to_ascii_lowercase();
        let value = value.trim();
        if name == "content-length" {
            content_length = Some(value.parse().map_err(|_| bad())?);
        } else if name == "content-range" {
            // `bytes a-b/total`, or `bytes */total` on 416.
            total = value.rsplit_once('/').and_then(|(_, t)| t.parse().ok());
        } else if name == "transfer-encoding" && value.to_ascii_lowercase().contains("chunked") {
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
