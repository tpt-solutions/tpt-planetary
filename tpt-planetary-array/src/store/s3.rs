//! A store backed by an S3 bucket (or any S3-compatible service reached over
//! plain HTTP, such as a local MinIO), with SigV4 request signing.
//!
//! Requests use path-style addressing: `http://endpoint/bucket/key`. Each
//! request is signed with SigV4 (a private module) when credentials are set.
//! Without credentials the store is anonymous and can read public buckets.
//!
//! Limits:
//! - `https://` endpoints are rejected. AWS itself requires TLS, and this crate
//!   has no TLS implementation, so the store reaches S3 only through an
//!   HTTP endpoint (for example a local S3-compatible server or a TLS-terminating
//!   proxy inside the trusted network).
//! - Multipart upload is not used; `set` sends one `PUT` with the whole value.
//! - Listing reads `ListObjectsV2` pages and follows continuation tokens.
//! - No credential chain beyond explicit values and the `AWS_*` environment
//!   variables (see [`Credentials::from_env`]). No instance metadata.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use std::time::{SystemTime, UNIX_EPOCH};

use super::http::{split_http_url, transient_status};
use super::sigv4::{Credentials, SigningRequest, amz_timestamp, sha256_hex, sign, uri_encode};
use super::transport::{Request, Response, exchange};
use super::{Store, validate_key};
use crate::error::{Error, Result};

impl Credentials {
    /// Reads `AWS_ACCESS_KEY_ID`, `AWS_SECRET_ACCESS_KEY` and the optional
    /// `AWS_SESSION_TOKEN`. Returns `None` if the first two are not set.
    pub fn from_env() -> Option<Self> {
        let access_key = std::env::var("AWS_ACCESS_KEY_ID").ok()?;
        let secret_key = std::env::var("AWS_SECRET_ACCESS_KEY").ok()?;
        let session_token = std::env::var("AWS_SESSION_TOKEN").ok();
        Some(Credentials {
            access_key,
            secret_key,
            session_token,
        })
    }
}

/// A store whose keys are objects in one S3 bucket.
#[derive(Debug, Clone)]
pub struct S3Store {
    host: String,
    port: u16,
    bucket: String,
    /// Key prefix inside the bucket, empty or ending in `/`.
    prefix: String,
    region: String,
    credentials: Option<Credentials>,
}

/// Checks the bucket name rules that matter for path-style URLs: 3 to 63
/// characters of lower-case letters, digits, `-` and `.`.
fn valid_bucket(name: &str) -> bool {
    (3..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.')
}

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Extracts every `<tag>text</tag>` value from an XML document, unescaping
/// the five predefined entities. This handles the flat `ListObjectsV2` reply;
/// it is not a general XML parser.
fn xml_values(xml: &str, tag: &str) -> Vec<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut out = Vec::new();
    let mut rest = xml;
    while let Some(i) = rest.find(&open) {
        let after = &rest[i + open.len()..];
        let Some(j) = after.find(&close) else { break };
        out.push(unescape_xml(&after[..j]));
        rest = &after[j + close.len()..];
    }
    out
}

fn unescape_xml(s: &str) -> String {
    s.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

impl S3Store {
    /// Creates a store for `bucket` at `endpoint` (for example
    /// `http://127.0.0.1:9000`), in `region`. The store is anonymous until
    /// [`S3Store::with_credentials`] is called.
    pub fn new(endpoint: &str, bucket: &str, region: &str) -> Result<Self> {
        if !valid_bucket(bucket) {
            return Err(Error::InvalidMetadata(format!(
                "invalid bucket name '{bucket}'"
            )));
        }
        if region.is_empty() {
            return Err(Error::InvalidMetadata("S3 region must not be empty".into()));
        }
        let (host, port, path) = split_http_url(endpoint)?;
        if path != "/" {
            return Err(Error::InvalidMetadata(
                "S3 endpoint must not have a path".into(),
            ));
        }
        Ok(Self {
            host,
            port,
            bucket: bucket.to_string(),
            prefix: String::new(),
            region: region.to_string(),
            credentials: None,
        })
    }

    /// Sets the key prefix inside the bucket. The prefix is joined with `/`.
    pub fn with_prefix(mut self, prefix: &str) -> Self {
        let trimmed = prefix.trim_matches('/');
        self.prefix = if trimmed.is_empty() {
            String::new()
        } else {
            format!("{trimmed}/")
        };
        self
    }

    /// Signs every request with `credentials`.
    pub fn with_credentials(mut self, credentials: Credentials) -> Self {
        self.credentials = Some(credentials);
        self
    }

    /// The bucket name.
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    fn full_key(&self, key: &str) -> String {
        format!("{}{}", self.prefix, key)
    }

    /// Sends one request. `path` is the unencoded object path (starting with
    /// `/`, with the bucket), `query` the unencoded query pairs.
    fn send(
        &self,
        method: &str,
        path: &str,
        query: &[(String, String)],
        extra: &[(String, String)],
        body: &[u8],
    ) -> Result<Response> {
        let canonical_uri = path
            .split('/')
            .map(|seg| uri_encode(seg, false))
            .collect::<Vec<_>>()
            .join("/");
        let mut encoded_query: Vec<(String, String)> = query
            .iter()
            .map(|(k, v)| (uri_encode(k, false), uri_encode(v, false)))
            .collect();
        encoded_query.sort();
        let canonical_query = encoded_query
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>()
            .join("&");
        let target = if canonical_query.is_empty() {
            canonical_uri.clone()
        } else {
            format!("{canonical_uri}?{canonical_query}")
        };

        let host_header = if self.port == 80 {
            self.host.clone()
        } else {
            format!("{}:{}", self.host, self.port)
        };
        let payload_hash = sha256_hex(body);
        let mut headers: Vec<(String, String)> = extra.to_vec();
        match &self.credentials {
            Some(creds) => {
                let (amz_date, _) = amz_timestamp(now_unix());
                let signed = sign(
                    &SigningRequest {
                        method,
                        canonical_uri: &canonical_uri,
                        canonical_query: &canonical_query,
                        headers: &headers,
                        payload_hash: &payload_hash,
                        host: &host_header,
                        amz_date: &amz_date,
                        region: &self.region,
                        credentials: creds,
                    },
                    "s3",
                );
                headers.extend(signed.headers);
            }
            None => headers.push(("x-amz-content-sha256".into(), payload_hash)),
        }
        exchange(&Request {
            host: &self.host,
            port: self.port,
            method,
            target: &target,
            headers: &headers,
            body,
        })
    }

    fn object_path(&self, key: &str) -> String {
        format!("/{}/{}", self.bucket, self.full_key(key))
    }

    fn status_error(status: u16, body: &[u8], what: &str) -> Error {
        let detail = core::str::from_utf8(body)
            .ok()
            .and_then(|b| xml_values(b, "Code").into_iter().next())
            .unwrap_or_default();
        let msg = format!("S3 HTTP {status} for {what}: {detail}");
        if transient_status(status) {
            Error::Transient(msg)
        } else {
            Error::Io(msg)
        }
    }
}

impl Store for S3Store {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        let r = self.send("GET", &self.object_path(key), &[], &[], &[])?;
        match r.status {
            200 => Ok(Some(r.body)),
            404 => Ok(None),
            s => Err(Self::status_error(s, &r.body, key)),
        }
    }

    fn get_range(&self, key: &str, start: u64, len: u64) -> Result<Option<Vec<u8>>> {
        validate_key(key)?;
        if len == 0 {
            return self.get(key).map(|found| found.map(|_| Vec::new()));
        }
        let end = start
            .checked_add(len - 1)
            .ok_or_else(|| Error::InvalidSelection("byte range overflows".into()))?;
        let range = vec![("Range".to_string(), format!("bytes={start}-{end}"))];
        let r = self.send("GET", &self.object_path(key), &[], &range, &[])?;
        match r.status {
            206 => Ok(Some(r.body)),
            // The service ignored the range and sent the whole object.
            200 => {
                let s = usize::try_from(start)
                    .unwrap_or(usize::MAX)
                    .min(r.body.len());
                let e = usize::try_from(end.saturating_add(1))
                    .unwrap_or(usize::MAX)
                    .min(r.body.len());
                Ok(Some(r.body[s..e].to_vec()))
            }
            404 => Ok(None),
            416 => Err(Error::InvalidSelection(format!(
                "byte range {start}+{len} not satisfiable for '{key}'"
            ))),
            s => Err(Self::status_error(s, &r.body, key)),
        }
    }

    fn size(&self, key: &str) -> Result<Option<u64>> {
        validate_key(key)?;
        let range = vec![("Range".to_string(), "bytes=0-0".to_string())];
        let r = self.send("GET", &self.object_path(key), &[], &range, &[])?;
        match r.status {
            206 => r
                .total
                .map(Some)
                .ok_or_else(|| Error::Io(format!("no Content-Range length for '{key}'"))),
            200 => Ok(Some(r.body.len() as u64)),
            416 => Ok(Some(r.total.unwrap_or(0))),
            404 => Ok(None),
            s => Err(Self::status_error(s, &r.body, key)),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        validate_key(key)?;
        let r = self.send("PUT", &self.object_path(key), &[], &[], value)?;
        match r.status {
            200 => Ok(()),
            s => Err(Self::status_error(s, &r.body, key)),
        }
    }

    fn delete(&self, key: &str) -> Result<()> {
        validate_key(key)?;
        let r = self.send("DELETE", &self.object_path(key), &[], &[], &[])?;
        match r.status {
            200 | 204 | 404 => Ok(()),
            s => Err(Self::status_error(s, &r.body, key)),
        }
    }

    fn list(&self, prefix: &str) -> Result<Vec<String>> {
        let full_prefix = self.full_key(prefix);
        let bucket_path = format!("/{}", self.bucket);
        let mut keys: Vec<String> = Vec::new();
        let mut token: Option<String> = None;
        loop {
            let mut query = vec![
                ("list-type".to_string(), "2".to_string()),
                ("prefix".to_string(), full_prefix.clone()),
            ];
            if let Some(t) = &token {
                query.push(("continuation-token".to_string(), t.clone()));
            }
            let r = self.send("GET", &bucket_path, &query, &[], &[])?;
            if r.status != 200 {
                return Err(Self::status_error(r.status, &r.body, &self.bucket));
            }
            let xml = core::str::from_utf8(&r.body)
                .map_err(|_| Error::Io("S3 listing is not UTF-8".into()))?;
            for full in xml_values(xml, "Key") {
                if let Some(rest) = full.strip_prefix(&self.prefix) {
                    keys.push(rest.to_string());
                }
            }
            let truncated = xml_values(xml, "IsTruncated")
                .first()
                .is_some_and(|v| v == "true");
            if !truncated {
                break;
            }
            token = xml_values(xml, "NextContinuationToken").into_iter().next();
            if token.is_none() {
                return Err(Error::Io("truncated S3 listing without a token".into()));
            }
        }
        keys.sort();
        keys.dedup();
        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};
    use std::thread;

    /// Answers one request, given the request text, with raw response bytes.
    type Responder = Box<dyn Fn(&str) -> Vec<u8> + Send>;

    /// A fake S3 server: each entry answers one request, given the request
    /// text. Returns the base URL and a log of request lines.
    fn fake_s3(
        responses: Vec<Responder>,
    ) -> (String, Arc<Mutex<Vec<String>>>, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log2 = log.clone();
        let handle = thread::spawn(move || {
            for respond in responses {
                let (mut stream, _) = listener.accept().unwrap();
                let mut req = Vec::new();
                let mut buf = [0u8; 4096];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                let head_end = req.windows(4).position(|w| w == b"\r\n\r\n").unwrap() + 4;
                let head = String::from_utf8_lossy(&req[..head_end]).into_owned();
                // Read the body if Content-Length asks for one.
                let want: usize = head
                    .lines()
                    .find_map(|l| l.strip_prefix("Content-Length: "))
                    .and_then(|v| v.trim().parse().ok())
                    .unwrap_or(0);
                while req.len() < head_end + want {
                    let n = stream.read(&mut buf).unwrap();
                    if n == 0 {
                        break;
                    }
                    req.extend_from_slice(&buf[..n]);
                }
                // Header names are compared case-insensitively: keep the request
                // line as sent, lower-case the rest.
                let mut lines = head.split("\r\n");
                let first = lines.next().unwrap_or_default().to_string();
                let logged: Vec<String> = std::iter::once(first)
                    .chain(lines.map(|l| l.to_ascii_lowercase()))
                    .collect();
                log2.lock().unwrap().push(logged.join("\r\n"));
                let text = format!("{head}{}", String::from_utf8_lossy(&req[head_end..]));
                stream.write_all(&respond(&text)).unwrap();
            }
        });
        (format!("http://127.0.0.1:{port}"), log, handle)
    }

    fn reply(status: &str, extra: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    fn creds() -> Credentials {
        Credentials {
            access_key: "AKIDEXAMPLE".into(),
            secret_key: "secret".into(),
            session_token: Some("tok".into()),
        }
    }

    #[test]
    fn constructor_validation() {
        assert!(S3Store::new("http://h:9000", "ab", "us-east-1").is_err());
        assert!(S3Store::new("http://h:9000", "Bad_Name", "us-east-1").is_err());
        assert!(S3Store::new("http://h:9000", "data-bucket", "").is_err());
        assert!(S3Store::new("https://s3.example", "data-bucket", "us-east-1").is_err());
        assert!(S3Store::new("http://h:9000/x", "data-bucket", "us-east-1").is_err());
        assert!(S3Store::new("http://h:9000", "data-bucket", "us-east-1").is_ok());
    }

    #[test]
    fn get_is_signed_and_maps_missing_keys() {
        let (url, log, server) = fake_s3(vec![
            Box::new(|_| reply("200 OK", "", "hello")),
            Box::new(|_| reply("404 Not Found", "", "<Error><Code>NoSuchKey</Code></Error>")),
        ]);
        let store = S3Store::new(&url, "data-bucket", "us-east-1")
            .unwrap()
            .with_prefix("/store/")
            .with_credentials(creds());
        assert_eq!(store.get("a/b").unwrap(), Some(b"hello".to_vec()));
        assert_eq!(store.get("missing").unwrap(), None);
        server.join().unwrap();
        let heads = log.lock().unwrap();
        assert!(
            heads[0].starts_with("GET /data-bucket/store/a/b HTTP/1.1"),
            "{}",
            heads[0]
        );
        assert!(heads[0].contains("authorization: aws4-hmac-sha256 credential=akidexample/"));
        assert!(heads[0].contains("x-amz-security-token: tok"));
        assert!(heads[0].contains("x-amz-content-sha256: e3b0c442"));
    }

    #[test]
    fn range_reads_and_size_use_partial_content() {
        let (url, log, server) = fake_s3(vec![
            Box::new(|_| reply("206 Partial Content", "", "CDE")),
            Box::new(|_| {
                reply(
                    "206 Partial Content",
                    "Content-Range: bytes 0-0/4096\r\n",
                    "x",
                )
            }),
        ]);
        let store = S3Store::new(&url, "data-bucket", "us-east-1").unwrap();
        assert_eq!(store.get_range("k", 2, 3).unwrap(), Some(b"CDE".to_vec()));
        assert_eq!(store.size("k").unwrap(), Some(4096));
        server.join().unwrap();
        let heads = log.lock().unwrap();
        assert!(heads[0].contains("range: bytes=2-4"));
        assert!(heads[1].contains("range: bytes=0-0"));
    }

    #[test]
    fn put_and_delete() {
        let (url, log, server) = fake_s3(vec![
            Box::new(|req| {
                assert!(req.contains("hello"), "body missing: {req}");
                reply("200 OK", "", "")
            }),
            Box::new(|_| reply("204 No Content", "", "")),
        ]);
        let store = S3Store::new(&url, "data-bucket", "us-east-1")
            .unwrap()
            .with_credentials(creds());
        store.set("k", b"hello").unwrap();
        store.delete("k").unwrap();
        server.join().unwrap();
        let heads = log.lock().unwrap();
        assert!(heads[0].starts_with("PUT /data-bucket/k HTTP/1.1"));
        assert!(heads[0].contains("content-length: 5"));
        assert!(heads[1].starts_with("DELETE /data-bucket/k"));
    }

    #[test]
    fn list_follows_continuation_tokens_and_strips_prefix() {
        let page1 = "<ListBucketResult><IsTruncated>true</IsTruncated>\
            <NextContinuationToken>t+1</NextContinuationToken>\
            <Contents><Key>p/a</Key></Contents>\
            <Contents><Key>p/b&amp;c</Key></Contents></ListBucketResult>";
        let page2 = "<ListBucketResult><IsTruncated>false</IsTruncated>\
            <Contents><Key>p/z</Key></Contents></ListBucketResult>";
        let (url, log, server) = fake_s3(vec![
            Box::new(move |_| reply("200 OK", "", page1)),
            Box::new(move |_| reply("200 OK", "", page2)),
        ]);
        let store = S3Store::new(&url, "data-bucket", "us-east-1")
            .unwrap()
            .with_prefix("p");
        assert_eq!(store.list("").unwrap(), vec!["a", "b&c", "z"]);
        server.join().unwrap();
        let heads = log.lock().unwrap();
        assert!(heads[0].contains("list-type=2"));
        assert!(heads[0].contains("prefix=p%2F"));
        assert!(heads[1].contains("continuation-token=t%2B1"));
    }

    #[test]
    fn server_errors_are_classified() {
        let (url, _log, server) = fake_s3(vec![
            Box::new(|_| reply("503 Slow Down", "", "<Error><Code>SlowDown</Code></Error>")),
            Box::new(|_| {
                reply(
                    "403 Forbidden",
                    "",
                    "<Error><Code>AccessDenied</Code></Error>",
                )
            }),
        ]);
        let store = S3Store::new(&url, "data-bucket", "us-east-1").unwrap();
        assert!(matches!(store.get("k"), Err(Error::Transient(_))));
        let err = store.get("k").unwrap_err();
        assert!(
            matches!(err, Error::Io(ref m) if m.contains("AccessDenied")),
            "{err:?}"
        );
        server.join().unwrap();
    }

    #[test]
    fn xml_helpers() {
        assert_eq!(
            xml_values("<a>1</a><b>x</b><a>2&lt;</a>", "a"),
            vec!["1", "2<"]
        );
        assert!(xml_values("<a>unclosed", "a").is_empty());
    }
}
