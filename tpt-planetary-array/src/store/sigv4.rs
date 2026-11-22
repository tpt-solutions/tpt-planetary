//! AWS Signature Version 4 for S3 requests, with the SHA-256 and HMAC
//! primitives it needs. Pure Rust, no dependencies.
//!
//! The signer produces the `Authorization` header for one request. It is
//! checked against the AWS documentation examples in the tests. Nothing here
//! performs I/O.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

/// SHA-256 round constants (first 32 bits of the fractional parts of the cube
/// roots of the first 64 primes).
const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 of `data`.
pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    // Padding: 0x80, zeros, then the bit length as a 64-bit big-endian value.
    let mut msg = data.to_vec();
    let bit_len = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());

    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks_exact(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut hh] = h;
        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ (!e & g);
            let t1 = hh
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let t2 = s0.wrapping_add(maj);
            hh = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (x, v) in h.iter_mut().zip([a, b, c, d, e, f, g, hh]) {
            *x = x.wrapping_add(v);
        }
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

/// HMAC-SHA256 of `message` under `key` (RFC 2104).
pub fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&sha256(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut inner: Vec<u8> = k.iter().map(|b| b ^ 0x36).collect();
    inner.extend_from_slice(message);
    let inner_hash = sha256(&inner);
    let mut outer: Vec<u8> = k.iter().map(|b| b ^ 0x5c).collect();
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

/// Lower-case hexadecimal encoding.
pub fn hex(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    s
}

/// SHA-256 of `data`, in hexadecimal (the payload hash of an S3 request).
pub fn sha256_hex(data: &[u8]) -> String {
    hex(&sha256(data))
}

/// Percent-encodes `s` for a SigV4 canonical URI or query component: the
/// unreserved characters `A-Z a-z 0-9 - _ . ~` are kept, `/` is kept when
/// `keep_slash` is set, and every other byte becomes `%XX` (upper case).
pub fn uri_encode(s: &str, keep_slash: bool) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        let keep = b.is_ascii_alphanumeric()
            || matches!(b, b'-' | b'_' | b'.' | b'~')
            || (keep_slash && b == b'/');
        if keep {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// Derives the SigV4 signing key: `HMAC` chained over date, region, service
/// and the literal `aws4_request`.
pub fn signing_key(secret: &str, date: &str, region: &str, service: &str) -> [u8; 32] {
    let k_date = hmac_sha256(format!("AWS4{secret}").as_bytes(), date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, service.as_bytes());
    hmac_sha256(&k_service, b"aws4_request")
}

/// Credentials for signing requests.
#[derive(Clone, PartialEq, Eq)]
pub struct Credentials {
    /// Access key ID.
    pub access_key: String,
    /// Secret access key.
    pub secret_key: String,
    /// Session token for temporary credentials, if any.
    pub session_token: Option<String>,
}

impl core::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        // Never print the secret.
        f.debug_struct("Credentials")
            .field("access_key", &self.access_key)
            .field("secret_key", &"<redacted>")
            .field(
                "session_token",
                &self.session_token.as_ref().map(|_| "<set>"),
            )
            .finish()
    }
}

/// The headers that a signed request must carry, computed from its contents.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Signed {
    /// `(name, value)` pairs to add to the request, lower-case names.
    pub headers: Vec<(String, String)>,
}

/// Inputs for one signature.
#[derive(Debug)]
pub struct SigningRequest<'a> {
    /// HTTP method.
    pub method: &'a str,
    /// Canonical URI: the percent-encoded path, starting with `/`.
    pub canonical_uri: &'a str,
    /// Canonical query string: sorted, percent-encoded `name=value` pairs
    /// joined by `&` (empty when there is no query).
    pub canonical_query: &'a str,
    /// Headers to sign, as `(lower-case name, trimmed value)`. `host`,
    /// `x-amz-content-sha256` and `x-amz-date` are added by [`sign`].
    pub headers: &'a [(String, String)],
    /// Hex SHA-256 of the request body (or `UNSIGNED-PAYLOAD`).
    pub payload_hash: &'a str,
    /// Host header value.
    pub host: &'a str,
    /// Request time in `YYYYMMDDTHHMMSSZ` form.
    pub amz_date: &'a str,
    /// Region, for example `us-east-1`.
    pub region: &'a str,
    /// Credentials.
    pub credentials: &'a Credentials,
}

/// Computes the SigV4 headers for a request. `service` is `s3` for S3.
pub fn sign(req: &SigningRequest<'_>, service: &str) -> Signed {
    let date = &req.amz_date[..8];
    let mut signed: Vec<(String, String)> = req
        .headers
        .iter()
        .map(|(n, v)| (n.to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    signed.push(("host".into(), req.host.into()));
    signed.push(("x-amz-content-sha256".into(), req.payload_hash.into()));
    signed.push(("x-amz-date".into(), req.amz_date.into()));
    if let Some(token) = &req.credentials.session_token {
        signed.push(("x-amz-security-token".into(), token.clone()));
    }
    signed.sort();

    let canonical_headers: String = signed.iter().map(|(n, v)| format!("{n}:{v}\n")).collect();
    let signed_names: Vec<&str> = signed.iter().map(|(n, _)| n.as_str()).collect();
    let signed_list = signed_names.join(";");

    let canonical_request = format!(
        "{}\n{}\n{}\n{}\n{}\n{}",
        req.method,
        req.canonical_uri,
        req.canonical_query,
        canonical_headers,
        signed_list,
        req.payload_hash
    );
    let scope = format!("{date}/{}/{service}/aws4_request", req.region);
    let string_to_sign = format!(
        "AWS4-HMAC-SHA256\n{}\n{scope}\n{}",
        req.amz_date,
        sha256_hex(canonical_request.as_bytes())
    );
    let key = signing_key(&req.credentials.secret_key, date, req.region, service);
    let signature = hex(&hmac_sha256(&key, string_to_sign.as_bytes()));
    let authorization = format!(
        "AWS4-HMAC-SHA256 Credential={}/{scope}, SignedHeaders={signed_list}, Signature={signature}",
        req.credentials.access_key
    );

    let mut headers = vec![
        (
            "x-amz-content-sha256".to_string(),
            req.payload_hash.to_string(),
        ),
        ("x-amz-date".to_string(), req.amz_date.to_string()),
    ];
    if let Some(token) = &req.credentials.session_token {
        headers.push(("x-amz-security-token".to_string(), token.clone()));
    }
    headers.push(("authorization".to_string(), authorization));
    Signed { headers }
}

/// Formats Unix seconds as `(YYYYMMDDTHHMMSSZ, YYYYMMDD)`.
pub fn amz_timestamp(unix_secs: i64) -> (String, String) {
    let days = unix_secs.div_euclid(86_400);
    let secs = unix_secs.rem_euclid(86_400);
    // Civil date from a day count (proleptic Gregorian, Howard Hinnant's method).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    let date = format!("{y:04}{m:02}{d:02}");
    let time = format!("{:02}{:02}{:02}", secs / 3600, (secs / 60) % 60, secs % 60);
    (format!("{date}T{time}Z"), date)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_known_answers() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        // Two blocks (the 448-bit padding boundary).
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn hmac_rfc4231_case_1_and_long_key() {
        let key = [0x0bu8; 20];
        assert_eq!(
            hex(&hmac_sha256(&key, b"Hi There")),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
        // RFC 4231 case 6: a key longer than the block size is hashed first.
        let long_key = [0xaau8; 131];
        assert_eq!(
            hex(&hmac_sha256(
                &long_key,
                b"Test Using Larger Than Block-Size Key - Hash Key First"
            )),
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn signing_key_matches_aws_documentation() {
        // AWS SigV4 documentation example: IAM, 2015-08-30, us-east-1.
        let key = signing_key(
            "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY",
            "20150830",
            "us-east-1",
            "iam",
        );
        assert_eq!(
            hex(&key),
            "c4afb1cc5771d871763a393e44b703571b55cc28424d1a5e86da6ed3c154a4b9"
        );
    }

    #[test]
    fn get_object_signature_matches_aws_documentation() {
        // AWS S3 documentation example "GET Object" with a Range header.
        let creds = Credentials {
            access_key: "AKIAIOSFODNN7EXAMPLE".into(),
            secret_key: "wJalrXUtnFEMI/K7MDENG/bPxRfiCYEXAMPLEKEY".into(),
            session_token: None,
        };
        let headers = vec![("Range".to_string(), "bytes=0-9".to_string())];
        let signed = sign(
            &SigningRequest {
                method: "GET",
                canonical_uri: "/test.txt",
                canonical_query: "",
                headers: &headers,
                payload_hash: &sha256_hex(b""),
                host: "examplebucket.s3.amazonaws.com",
                amz_date: "20130524T000000Z",
                region: "us-east-1",
                credentials: &creds,
            },
            "s3",
        );
        let auth = signed
            .headers
            .iter()
            .find(|(n, _)| n == "authorization")
            .map(|(_, v)| v.as_str())
            .unwrap();
        assert!(auth.ends_with(
            "Signature=f0e8bdb87c964420e857bd35b5d6ed310bd44f0170aba48dd91039c6036bdb41"
        ));
        assert!(auth.contains("SignedHeaders=host;range;x-amz-content-sha256;x-amz-date"));
        assert!(
            auth.contains("Credential=AKIAIOSFODNN7EXAMPLE/20130524/us-east-1/s3/aws4_request")
        );
    }

    #[test]
    fn uri_encoding_keeps_unreserved_and_slash_when_asked() {
        assert_eq!(uri_encode("a b/c~d_e.f-g", false), "a%20b%2Fc~d_e.f-g");
        assert_eq!(uri_encode("a b/c", true), "a%20b/c");
        assert_eq!(uri_encode("é", false), "%C3%A9");
    }

    #[test]
    fn timestamps_from_unix_seconds() {
        assert_eq!(
            amz_timestamp(0),
            ("19700101T000000Z".into(), "19700101".into())
        );
        // 2013-05-24T00:00:00Z.
        assert_eq!(
            amz_timestamp(1_369_353_600),
            ("20130524T000000Z".into(), "20130524".into())
        );
        // A leap day: 2024-02-29T12:34:56Z.
        assert_eq!(
            amz_timestamp(1_709_210_096),
            ("20240229T123456Z".into(), "20240229".into())
        );
    }

    #[test]
    fn debug_hides_the_secret() {
        let c = Credentials {
            access_key: "AK".into(),
            secret_key: "super-secret".into(),
            session_token: None,
        };
        assert!(!format!("{c:?}").contains("super-secret"));
    }
}
