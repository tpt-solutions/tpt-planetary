//! FITS image I/O: a pure-Rust reader and writer for primary HDUs and
//! `IMAGE` extensions, following NOST 100-2.0 (Pence et al. 2010).
//!
//! The reader works on byte slices, performs no allocation and does not need
//! `std`, so it builds for `no_std` targets. Owned pixel arrays
//! ([`ImageData`]), the writer, and [`read_file`] need the `std` feature.
//!
//! Supported:
//! - `BITPIX` 8, 16, 32, 64, -32 and -64, with `NAXIS` and `NAXISn`.
//! - `BSCALE` and `BZERO`: physical value = `BZERO + BSCALE * stored`. The
//!   conventional unsigned offsets (2^15, 2^31, 2^63, and -128 for 8-bit) are
//!   mapped to unsigned or signed native types by [`Hdu::to_native`].
//! - The primary HDU and `XTENSION= 'IMAGE'` extensions, in 2880-byte blocks.
//! - Fixed-format and free-format cards, with `''` escapes in strings and
//!   Fortran `D` exponents in numbers.
//!
//! Not supported, and rejected with an error: binary and ASCII tables, tile or
//! other compression, random groups, and `CONTINUE` long-string cards. A
//! `HIERARCH` card is returned as a commentary card and is not interpreted.
//! A final data block that lacks its zero padding is accepted.
//!
//! Pixel order follows FITS: axis 1 varies fastest, so the flat index of a
//! pixel is `x + NAXIS1 * (y + NAXIS2 * z)` for 0-based `x`, `y`, `z`.
//!
//! # Example
//!
//! ```
//! use tpt_planetary_astro::fits::{self, Bitpix, Scaling, Value, WriteCard};
//!
//! let header = [WriteCard {
//!     keyword: "OBJECT",
//!     value: Some(Value::Text("M31")),
//!     comment: "target",
//! }];
//! let data = [1.0, 2.5, -3.0, 4.0];
//! let bytes = fits::write_primary_f64(&header, Bitpix::F64, Scaling::NONE, &[2, 2], &data)?;
//!
//! let hdu = fits::primary(&bytes)?;
//! assert_eq!(hdu.header().get("OBJECT"), Some(Value::Text("M31")));
//! assert_eq!(hdu.values().collect::<Vec<f64>>(), data);
//! # Ok::<(), fits::FitsError>(())
//! ```

use core::fmt;

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// Size of a FITS logical block in bytes.
const BLOCK: usize = 2880;
/// Size of a FITS card in bytes.
const CARD: usize = 80;
/// Largest `NAXIS` the standard allows.
const MAX_AXES: usize = 999;

/// Errors from parsing or writing FITS data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FitsError {
    /// The input ends before the structure it declares.
    Truncated(&'static str),
    /// A card has a non-printable byte or a malformed keyword or value.
    InvalidCard {
        /// Byte offset of the card from the start of the input.
        offset: usize,
    },
    /// A required keyword is missing, or has a value that cannot be used.
    InvalidHeader(&'static str),
    /// Valid FITS that this module does not read or write.
    Unsupported(&'static str),
    /// The output buffer does not match the number of pixels.
    BufferMismatch {
        /// Number of pixels in the HDU.
        expected: usize,
        /// Length of the buffer supplied.
        actual: usize,
    },
    /// A value cannot be stored in the requested `BITPIX`: it is out of range,
    /// fractional without scaling, or not finite.
    NotRepresentable,
    /// The writer rejected a keyword: it is malformed, or the writer generates it.
    InvalidKeyword,
    /// The writer rejected card text: it is not printable ASCII, or a string
    /// does not fit on one card.
    InvalidCardText,
    /// The shape does not match the number of data elements.
    ShapeMismatch,
    /// A file I/O error from [`read_file`].
    #[cfg(feature = "std")]
    Io(std::io::ErrorKind),
}

impl fmt::Display for FitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated(what) => write!(f, "truncated FITS data: {what}"),
            Self::InvalidCard { offset } => write!(f, "invalid FITS card at byte {offset}"),
            Self::InvalidHeader(what) => write!(f, "invalid FITS header: {what}"),
            Self::Unsupported(what) => write!(f, "unsupported FITS feature: {what}"),
            Self::BufferMismatch { expected, actual } => {
                write!(f, "buffer has {actual} elements, HDU has {expected}")
            }
            Self::NotRepresentable => write!(f, "value not representable in the requested BITPIX"),
            Self::InvalidKeyword => write!(f, "invalid or reserved FITS keyword"),
            Self::InvalidCardText => write!(f, "card text is not printable ASCII or does not fit"),
            Self::ShapeMismatch => write!(f, "shape does not match the number of data elements"),
            #[cfg(feature = "std")]
            Self::Io(kind) => write!(f, "I/O error: {kind:?}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for FitsError {}

/// Result type for this module.
pub type Result<T> = core::result::Result<T, FitsError>;

/// A parsed header-card value.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value<'a> {
    /// Logical `T` or `F`.
    Logical(bool),
    /// Integer with no `.`, `E` or `D`.
    Integer(i64),
    /// Real number. `D` exponents are read as `E`.
    Float(f64),
    /// Character string. The text is as written between the quotes, with
    /// trailing spaces removed and each `''` escape left as two quotes.
    Text(&'a str),
}

impl Value<'_> {
    /// Returns the value as an integer, or `None` for other types.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        match *self {
            Self::Integer(i) => Some(i),
            _ => None,
        }
    }

    /// Returns the value as a float. Integers are converted.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match *self {
            Self::Integer(i) => Some(i as f64),
            Self::Float(f) => Some(f),
            _ => None,
        }
    }
}

/// One parsed 80-character header card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Card<'a> {
    /// The keyword with trailing spaces removed. Empty for a blank card.
    pub keyword: &'a str,
    /// The parsed value. `None` for commentary cards (`COMMENT`, `HISTORY`,
    /// blank) and for undefined values.
    pub value: Option<Value<'a>>,
    /// The text after `/`, or the text of a commentary card.
    pub comment: &'a str,
    /// The 80 characters of the card as written in the file.
    pub raw: &'a str,
}

/// A card to write, given to the writer as part of the header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WriteCard<'a> {
    /// Keyword: up to 8 characters from `A-Z`, `0-9`, `-` and `_`.
    pub keyword: &'a str,
    /// Value, or `None` for a commentary card whose text is in `comment`.
    pub value: Option<Value<'a>>,
    /// Comment after `/`. Truncated to fit on the card.
    pub comment: &'a str,
}

/// The FITS `BITPIX` data types supported by this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(i8)]
pub enum Bitpix {
    /// 8-bit integer (`BITPIX = 8`).
    U8 = 8,
    /// 16-bit two's-complement integer (`BITPIX = 16`).
    I16 = 16,
    /// 32-bit two's-complement integer (`BITPIX = 32`).
    I32 = 32,
    /// 64-bit two's-complement integer (`BITPIX = 64`).
    I64 = 64,
    /// IEEE 754 single precision (`BITPIX = -32`).
    F32 = -32,
    /// IEEE 754 double precision (`BITPIX = -64`).
    F64 = -64,
}

impl Bitpix {
    /// Returns the `BITPIX` keyword value.
    #[must_use]
    pub fn code(self) -> i64 {
        i64::from(self as i8)
    }

    /// Returns the type for a `BITPIX` keyword value, if supported.
    #[must_use]
    pub fn from_code(code: i64) -> Option<Self> {
        match code {
            8 => Some(Self::U8),
            16 => Some(Self::I16),
            32 => Some(Self::I32),
            64 => Some(Self::I64),
            -32 => Some(Self::F32),
            -64 => Some(Self::F64),
            _ => None,
        }
    }

    /// Returns the size of one stored element in bytes.
    #[must_use]
    pub fn bytes(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::I16 => 2,
            Self::I32 | Self::F32 => 4,
            Self::I64 | Self::F64 => 8,
        }
    }
}

/// Linear scaling from stored values to physical values:
/// `physical = bzero + bscale * stored`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scaling {
    /// The `BSCALE` factor. Must be finite and non-zero.
    pub bscale: f64,
    /// The `BZERO` offset. Must be finite.
    pub bzero: f64,
}

impl Scaling {
    /// No scaling: `BSCALE = 1` and `BZERO = 0`.
    pub const NONE: Self = Self {
        bscale: 1.0,
        bzero: 0.0,
    };
}

/// A header with its byte range, cards in file order up to `END`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header<'a> {
    bytes: &'a [u8],
}

impl<'a> Header<'a> {
    /// Parses 2880-byte blocks until the block that holds `END`. Every card is
    /// validated here, so later iteration cannot fail.
    fn parse(bytes: &'a [u8], offset: usize) -> Result<(Self, usize)> {
        let mut pos = 0;
        while pos + BLOCK <= bytes.len() {
            let block = &bytes[pos..pos + BLOCK];
            let mut found = false;
            for (i, raw) in block.chunks_exact(CARD).enumerate() {
                let card = parse_card(raw).ok_or(FitsError::InvalidCard {
                    offset: offset + pos + i * CARD,
                })?;
                if card.keyword == "CONTINUE" {
                    return Err(FitsError::Unsupported("CONTINUE long-string cards"));
                }
                if card.keyword == "END" && card.value.is_none() && card.comment.is_empty() {
                    found = true;
                    break;
                }
            }
            pos += BLOCK;
            if found {
                return Ok((
                    Self {
                        bytes: &bytes[..pos],
                    },
                    pos,
                ));
            }
        }
        if bytes.len() < BLOCK {
            Err(FitsError::Truncated(
                "header is shorter than one 2880-byte block",
            ))
        } else {
            Err(FitsError::Truncated("no END card before end of input"))
        }
    }

    /// Iterates over the cards in file order, up to but not including `END`.
    pub fn cards(&self) -> impl Iterator<Item = Card<'a>> + 'a {
        let bytes: &'a [u8] = self.bytes;
        bytes
            .chunks_exact(CARD)
            .map_while(|raw| parse_card(raw).filter(|c| c.keyword != "END"))
    }

    /// Returns the first card with `keyword`, if any.
    #[must_use]
    pub fn card(&self, keyword: &str) -> Option<Card<'a>> {
        self.cards().find(|c| c.keyword == keyword)
    }

    /// Returns the value of the first card with `keyword`, if it has one.
    #[must_use]
    pub fn get(&self, keyword: &str) -> Option<Value<'a>> {
        self.card(keyword).and_then(|c| c.value)
    }

    /// Returns the header bytes as text: 80-character cards, `END` and the
    /// space padding. Suitable for [`crate::wcs::Wcs::from_header`].
    #[must_use]
    pub fn as_text(&self) -> &'a str {
        core::str::from_utf8(self.bytes).unwrap_or("")
    }

    /// Returns the header length in bytes, a multiple of 2880.
    #[must_use]
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
}

/// Parses one 80-byte card. Returns `None` if it is malformed.
fn parse_card(raw: &[u8]) -> Option<Card<'_>> {
    if raw.len() != CARD || !raw.iter().all(|b| (0x20..=0x7E).contains(b)) {
        return None;
    }
    let text = core::str::from_utf8(raw).ok()?;
    let keyword = text.get(..8)?.trim_end_matches(' ');
    if !keyword
        .bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return None;
    }
    let rest = text.get(8..)?;
    if raw[8] != b'=' || raw[9] != b' ' {
        // Commentary card, blank card, END, or a card without a value field.
        return Some(Card {
            keyword,
            value: None,
            comment: rest.trim_matches(' '),
            raw: text,
        });
    }

    let field = text.get(10..)?.trim_start_matches(' ');
    if let Some(quoted) = field.strip_prefix('\'') {
        let bytes = quoted.as_bytes();
        let mut i = 0;
        let close = loop {
            match bytes.get(i) {
                None => return None,
                Some(b'\'') if bytes.get(i + 1) == Some(&b'\'') => i += 2,
                Some(b'\'') => break i,
                Some(_) => i += 1,
            }
        };
        let body = quoted.get(..close)?.trim_end_matches(' ');
        let tail = quoted.get(close + 1..)?;
        return Some(Card {
            keyword,
            value: Some(Value::Text(body)),
            comment: slash_comment(tail),
            raw: text,
        });
    }

    let (token, comment) = field.split_once('/').unwrap_or((field, ""));
    let token = token.trim_end_matches(' ');
    let value = if token.is_empty() {
        None
    } else {
        Some(parse_scalar(token)?)
    };
    Some(Card {
        keyword,
        value,
        comment: comment.trim(),
        raw: text,
    })
}

/// Returns the text after the first `/`, trimmed, or `""`.
fn slash_comment(tail: &str) -> &str {
    tail.split_once('/').map_or("", |(_, c)| c.trim())
}

/// Parses a logical, integer or real token (no string, no comment).
fn parse_scalar(token: &str) -> Option<Value<'_>> {
    match token {
        "T" => return Some(Value::Logical(true)),
        "F" => return Some(Value::Logical(false)),
        _ => {}
    }
    let first = *token.as_bytes().first()?;
    if !(first.is_ascii_digit() || matches!(first, b'+' | b'-' | b'.')) {
        return None;
    }
    if let Ok(i) = token.parse::<i64>() {
        return Some(Value::Integer(i));
    }
    // Rust's float parser does not accept `D` exponents, so copy the token
    // into a fixed buffer and replace them.
    let mut buf = [0u8; 48];
    let dst = buf.get_mut(..token.len())?;
    for (d, s) in dst.iter_mut().zip(token.bytes()) {
        *d = if s == b'D' || s == b'd' { b'E' } else { s };
    }
    let f: f64 = core::str::from_utf8(dst).ok()?.parse().ok()?;
    f.is_finite().then_some(Value::Float(f))
}

/// One HDU: its header, and the raw big-endian data bytes (no padding).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hdu<'a> {
    header: Header<'a>,
    data: &'a [u8],
    bitpix: Bitpix,
    naxis: usize,
    pixel_count: usize,
    bscale: f64,
    bzero: f64,
    table: bool,
}

impl<'a> Hdu<'a> {
    /// Returns the header.
    #[must_use]
    pub fn header(&self) -> Header<'a> {
        self.header
    }

    /// True if this extension is a binary table (`XTENSION = 'BINTABLE'`).
    #[must_use]
    pub fn is_table(&self) -> bool {
        self.table
    }

    /// Returns the data type.
    #[must_use]
    pub fn bitpix(&self) -> Bitpix {
        self.bitpix
    }

    /// Returns `NAXIS`, the number of axes.
    #[must_use]
    pub fn naxis(&self) -> usize {
        self.naxis
    }

    /// Returns `NAXISn` for 1-based `n`, or 0 if `n` is out of range.
    #[must_use]
    pub fn axis_len(&self, n: usize) -> usize {
        if n == 0 || n > self.naxis {
            return 0;
        }
        self.header
            .get(&format_axis(n))
            .and_then(|v| v.as_i64())
            .and_then(|v| usize::try_from(v).ok())
            .unwrap_or(0)
    }

    /// Returns the number of pixels: the product of all `NAXISn`.
    #[must_use]
    pub fn pixel_count(&self) -> usize {
        self.pixel_count
    }

    /// Returns `BSCALE` (1.0 if absent).
    #[must_use]
    pub fn bscale(&self) -> f64 {
        self.bscale
    }

    /// Returns `BZERO` (0.0 if absent).
    #[must_use]
    pub fn bzero(&self) -> f64 {
        self.bzero
    }

    /// Returns the raw big-endian data bytes, without block padding.
    #[must_use]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Iterates over the physical pixel values in FITS order, as
    /// `bzero + bscale * stored`. Float NaN values are passed through.
    ///
    /// Scaling is done in `f64`, so large 64-bit integers can lose precision.
    pub fn values(&self) -> impl ExactSizeIterator<Item = f64> + 'a {
        let (bitpix, bscale, bzero) = (self.bitpix, self.bscale, self.bzero);
        // The conventional unsigned 64-bit encoding (BZERO = 2^63) is mapped
        // by flipping the sign bit, so that the one rounding is the conversion
        // to f64. Going through a signed f64 first loses small values.
        let unsigned64 =
            bitpix == Bitpix::I64 && bscale == 1.0 && bzero == 9_223_372_036_854_775_808.0;
        self.data.chunks_exact(bitpix.bytes()).map(move |c| {
            if unsigned64 {
                return c
                    .first_chunk::<8>()
                    .map_or(f64::NAN, |a| (u64::from_be_bytes(*a) ^ (1 << 63)) as f64);
            }
            bzero + bscale * decode(bitpix, c)
        })
    }

    /// Writes the physical pixel values into `out`.
    ///
    /// # Errors
    ///
    /// Returns [`FitsError::BufferMismatch`] if `out.len()` is not
    /// [`pixel_count`](Self::pixel_count).
    pub fn read_into(&self, out: &mut [f64]) -> Result<()> {
        if out.len() != self.pixel_count {
            return Err(FitsError::BufferMismatch {
                expected: self.pixel_count,
                actual: out.len(),
            });
        }
        for (o, v) in out.iter_mut().zip(self.values()) {
            *o = v;
        }
        Ok(())
    }

    /// Returns the stored (unscaled) values in their native type.
    ///
    /// The native type is chosen from `BITPIX` and the conventional offset:
    /// `BITPIX 8` with `BZERO 0` gives `U8`, `BITPIX 8` with `BZERO -128`
    /// gives `I8`, `BITPIX 16` with `BZERO 32768` gives `U16`, and likewise
    /// for 32 and 64 bits with `2^31` and `2^63`. Float data with `BZERO 0`
    /// gives `F32` or `F64`. Other combinations need [`values`](Self::values).
    ///
    /// # Errors
    ///
    /// Returns [`FitsError::Unsupported`] if `BSCALE` is not 1, or `BZERO`
    /// is not one of the conventional values above.
    #[cfg(feature = "std")]
    pub fn to_native(&self) -> Result<ImageData> {
        if self.bscale != 1.0 {
            return Err(FitsError::Unsupported(
                "BSCALE != 1; use values() for scaled data",
            ));
        }
        let d = self.data;
        let bz = self.bzero;
        Ok(match (self.bitpix, bz) {
            (Bitpix::U8, 0.0) => ImageData::U8(d.to_vec()),
            (Bitpix::U8, -128.0) => ImageData::I8(d.iter().map(|&b| (b ^ 0x80) as i8).collect()),
            (Bitpix::I16, 0.0) => ImageData::I16(collect_be(d, i16::from_be_bytes)),
            (Bitpix::I16, 32768.0) => {
                ImageData::U16(collect_be(d, |a| u16::from_be_bytes(a) ^ 0x8000))
            }
            (Bitpix::I32, 0.0) => ImageData::I32(collect_be(d, i32::from_be_bytes)),
            (Bitpix::I32, 2_147_483_648.0) => {
                ImageData::U32(collect_be(d, |a| u32::from_be_bytes(a) ^ 0x8000_0000))
            }
            (Bitpix::I64, 0.0) => ImageData::I64(collect_be(d, i64::from_be_bytes)),
            (Bitpix::I64, 9_223_372_036_854_775_808.0) => {
                ImageData::U64(collect_be(d, |a| u64::from_be_bytes(a) ^ (1 << 63)))
            }
            (Bitpix::F32, 0.0) => ImageData::F32(collect_be(d, f32::from_be_bytes)),
            (Bitpix::F64, 0.0) => ImageData::F64(collect_be(d, f64::from_be_bytes)),
            _ => {
                return Err(FitsError::Unsupported(
                    "non-conventional BZERO; use values() for scaled data",
                ));
            }
        })
    }
}

/// Formats `NAXISn` without allocating a `String` when used as a key.
fn format_axis(n: usize) -> AxisKey {
    AxisKey::new(n)
}

/// A `NAXISn` keyword held in a fixed buffer, so header lookup needs no heap.
struct AxisKey {
    buf: [u8; 10],
    len: usize,
}

impl AxisKey {
    fn new(n: usize) -> Self {
        let mut buf = [0u8; 10];
        buf[..5].copy_from_slice(b"NAXIS");
        let mut digits = [0u8; 20];
        let mut k = n;
        let mut len = 0;
        loop {
            digits[len] = b'0' + (k % 10) as u8;
            len += 1;
            k /= 10;
            if k == 0 {
                break;
            }
        }
        for (dst, &d) in buf[5..5 + len].iter_mut().zip(digits[..len].iter().rev()) {
            *dst = d;
        }
        Self { buf, len: 5 + len }
    }
}

impl core::ops::Deref for AxisKey {
    type Target = str;
    fn deref(&self) -> &str {
        core::str::from_utf8(&self.buf[..self.len]).unwrap_or("")
    }
}

/// Decodes one stored element as `f64`, without scaling.
fn decode(bitpix: Bitpix, c: &[u8]) -> f64 {
    match bitpix {
        Bitpix::U8 => c.first().map_or(f64::NAN, |&b| f64::from(b)),
        Bitpix::I16 => c
            .first_chunk::<2>()
            .map_or(f64::NAN, |a| f64::from(i16::from_be_bytes(*a))),
        Bitpix::I32 => c
            .first_chunk::<4>()
            .map_or(f64::NAN, |a| f64::from(i32::from_be_bytes(*a))),
        Bitpix::I64 => c
            .first_chunk::<8>()
            .map_or(f64::NAN, |a| i64::from_be_bytes(*a) as f64),
        Bitpix::F32 => c
            .first_chunk::<4>()
            .map_or(f64::NAN, |a| f64::from(f32::from_be_bytes(*a))),
        Bitpix::F64 => c
            .first_chunk::<8>()
            .map_or(f64::NAN, |a| f64::from_be_bytes(*a)),
    }
}

/// Decodes every `N`-byte chunk of `data` with `f`.
#[cfg(feature = "std")]
fn collect_be<T, const N: usize>(data: &[u8], f: impl Fn([u8; N]) -> T) -> Vec<T> {
    data.chunks_exact(N)
        .filter_map(|c| c.first_chunk::<N>().map(|a| f(*a)))
        .collect()
}

/// Owned pixel data in its stored native type. See [`Hdu::to_native`].
#[cfg(feature = "std")]
#[derive(Debug, Clone, PartialEq)]
pub enum ImageData {
    /// Unsigned 8-bit (`BITPIX 8`, `BZERO 0`).
    U8(Vec<u8>),
    /// Signed 8-bit (`BITPIX 8`, `BZERO -128`).
    I8(Vec<i8>),
    /// Signed 16-bit (`BITPIX 16`, `BZERO 0`).
    I16(Vec<i16>),
    /// Unsigned 16-bit (`BITPIX 16`, `BZERO 32768`).
    U16(Vec<u16>),
    /// Signed 32-bit (`BITPIX 32`, `BZERO 0`).
    I32(Vec<i32>),
    /// Unsigned 32-bit (`BITPIX 32`, `BZERO 2^31`).
    U32(Vec<u32>),
    /// Signed 64-bit (`BITPIX 64`, `BZERO 0`).
    I64(Vec<i64>),
    /// Unsigned 64-bit (`BITPIX 64`, `BZERO 2^63`).
    U64(Vec<u64>),
    /// Single precision (`BITPIX -32`).
    F32(Vec<f32>),
    /// Double precision (`BITPIX -64`).
    F64(Vec<f64>),
}

/// Parses one HDU from the start of `bytes`. Returns it with the number of
/// bytes it occupies, including block padding.
fn parse_hdu(bytes: &[u8], offset: usize, first: bool) -> Result<(Hdu<'_>, usize)> {
    let (header, header_len) = Header::parse(bytes, offset)?;
    let first_card = header
        .cards()
        .next()
        .ok_or(FitsError::InvalidHeader("empty header"))?;
    if first {
        if first_card.keyword != "SIMPLE" {
            return Err(FitsError::InvalidHeader("first card is not SIMPLE"));
        }
        if first_card.value != Some(Value::Logical(true)) {
            return Err(FitsError::Unsupported("SIMPLE is not T"));
        }
        if header.get("GROUPS") == Some(Value::Logical(true)) {
            return Err(FitsError::Unsupported("random groups (GROUPS = T)"));
        }
    } else {
        if first_card.keyword != "XTENSION" {
            return Err(FitsError::InvalidHeader(
                "extension does not start with XTENSION",
            ));
        }
        match first_card.value {
            Some(Value::Text(t)) if t.trim_end() == "IMAGE" || t.trim_end() == "BINTABLE" => {}
            Some(Value::Text(_)) => {
                return Err(FitsError::Unsupported(
                    "XTENSION other than IMAGE and BINTABLE (ASCII tables are not supported)",
                ));
            }
            _ => return Err(FitsError::InvalidHeader("XTENSION is not a string")),
        }
    }
    let table = !first
        && matches!(
            header.get("XTENSION"),
            Some(Value::Text(t)) if t.trim_end() == "BINTABLE"
        );

    let bitpix = header
        .get("BITPIX")
        .and_then(|v| v.as_i64())
        .and_then(Bitpix::from_code)
        .ok_or(FitsError::InvalidHeader(
            "BITPIX missing or not 8, 16, 32, 64, -32, -64",
        ))?;
    let naxis = header
        .get("NAXIS")
        .and_then(|v| v.as_i64())
        .and_then(|v| usize::try_from(v).ok())
        .filter(|&n| n <= MAX_AXES)
        .ok_or(FitsError::InvalidHeader("NAXIS missing or out of range"))?;

    let mut pixel_count: usize = if naxis == 0 { 0 } else { 1 };
    for n in 1..=naxis {
        let len = header
            .get(&format_axis(n))
            .and_then(|v| v.as_i64())
            .and_then(|v| usize::try_from(v).ok())
            .ok_or(FitsError::InvalidHeader("NAXISn missing or negative"))?;
        pixel_count = pixel_count
            .checked_mul(len)
            .ok_or(FitsError::InvalidHeader("pixel count overflows"))?;
    }
    let pcount = optional_count(&header, "PCOUNT", 0)?;
    let gcount = optional_count(&header, "GCOUNT", 1)?;
    let data_len = if naxis == 0 {
        0
    } else {
        bitpix
            .bytes()
            .checked_mul(gcount)
            .and_then(|v| v.checked_mul(pcount.checked_add(pixel_count)?))
            .ok_or(FitsError::InvalidHeader("data size overflows"))?
    };

    let bscale = optional_f64(&header, "BSCALE", 1.0)?;
    let bzero = optional_f64(&header, "BZERO", 0.0)?;

    let data_end = header_len
        .checked_add(data_len)
        .ok_or(FitsError::InvalidHeader("data size overflows"))?;
    let data = bytes
        .get(header_len..data_end)
        .ok_or(FitsError::Truncated("data is shorter than NAXIS implies"))?;
    // A final block without its zero padding is accepted.
    let padded = data_end.div_ceil(BLOCK) * BLOCK;
    let used = padded.min(bytes.len());

    Ok((
        Hdu {
            header,
            data,
            bitpix,
            naxis,
            pixel_count,
            bscale,
            bzero,
            table,
        },
        used,
    ))
}

fn optional_count(header: &Header<'_>, keyword: &str, default: usize) -> Result<usize> {
    match header.get(keyword) {
        None => Ok(default),
        Some(v) => {
            v.as_i64()
                .and_then(|i| usize::try_from(i).ok())
                .ok_or(FitsError::InvalidHeader(
                    "PCOUNT/GCOUNT is not a non-negative integer",
                ))
        }
    }
}

fn optional_f64(header: &Header<'_>, keyword: &str, default: f64) -> Result<f64> {
    match header.get(keyword) {
        None => Ok(default),
        Some(v) => v
            .as_f64()
            .filter(|f| f.is_finite())
            .ok_or(FitsError::InvalidHeader(
                "BSCALE/BZERO is not a finite number",
            )),
    }
}

/// Iterator over the HDUs of a FITS file. See [`hdus`].
#[derive(Debug, Clone)]
pub struct Hdus<'a> {
    rest: &'a [u8],
    offset: usize,
    first: bool,
    done: bool,
}

impl<'a> Iterator for Hdus<'a> {
    type Item = Result<Hdu<'a>>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done || self.rest.is_empty() {
            return None;
        }
        match parse_hdu(self.rest, self.offset, self.first) {
            Ok((hdu, used)) => {
                self.rest = self.rest.get(used..).unwrap_or(&[]);
                self.offset += used;
                self.first = false;
                Some(Ok(hdu))
            }
            Err(e) => {
                // Stop after an error: the position of the next HDU is unknown.
                self.done = true;
                Some(Err(e))
            }
        }
    }
}

/// Iterates over every HDU in `bytes`: the primary HDU, then each extension.
///
/// ASCII table extensions yield an [`FitsError::Unsupported`] error and end the
/// iteration. Binary tables are yielded as HDUs; read them with [`Hdu::table`].
/// The primary HDU must be first.
#[must_use]
pub fn hdus(bytes: &[u8]) -> Hdus<'_> {
    Hdus {
        rest: bytes,
        offset: 0,
        first: true,
        done: false,
    }
}

/// Parses the primary HDU at the start of `bytes`.
///
/// # Errors
///
/// Returns an error if the header or data is malformed, truncated, or not a
/// supported image.
pub fn primary(bytes: &[u8]) -> Result<Hdu<'_>> {
    parse_hdu(bytes, 0, true).map(|(hdu, _)| hdu)
}

/// Reads a whole file into memory, for use with [`primary`] or [`hdus`].
///
/// # Errors
///
/// Returns [`FitsError::Io`] if the file cannot be read.
#[cfg(feature = "std")]
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<Vec<u8>> {
    std::fs::read(path).map_err(|e| FitsError::Io(e.kind()))
}

// ---------------------------------------------------------------------------
// Writer
// ---------------------------------------------------------------------------

/// Native pixel slices accepted by [`write_primary`].
///
/// Unsigned types are written with the conventional offset (`BZERO` 2^15,
/// 2^31 or 2^63), and `I8` with `BZERO -128`, so they read back as the same
/// native type.
#[cfg(feature = "std")]
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Pixels<'a> {
    /// Unsigned 8-bit, written as `BITPIX 8`.
    U8(&'a [u8]),
    /// Signed 8-bit, written as `BITPIX 8` with `BZERO -128`.
    I8(&'a [i8]),
    /// Signed 16-bit, written as `BITPIX 16`.
    I16(&'a [i16]),
    /// Unsigned 16-bit, written as `BITPIX 16` with `BZERO 32768`.
    U16(&'a [u16]),
    /// Signed 32-bit, written as `BITPIX 32`.
    I32(&'a [i32]),
    /// Unsigned 32-bit, written as `BITPIX 32` with `BZERO 2^31`.
    U32(&'a [u32]),
    /// Signed 64-bit, written as `BITPIX 64`.
    I64(&'a [i64]),
    /// Unsigned 64-bit, written as `BITPIX 64` with `BZERO 2^63`.
    U64(&'a [u64]),
    /// Single precision, written as `BITPIX -32`.
    F32(&'a [f32]),
    /// Double precision, written as `BITPIX -64`.
    F64(&'a [f64]),
}

#[cfg(feature = "std")]
impl Pixels<'_> {
    /// Returns the number of pixels.
    #[must_use]
    pub fn len(&self) -> usize {
        match self {
            Self::U8(s) => s.len(),
            Self::I8(s) => s.len(),
            Self::I16(s) => s.len(),
            Self::U16(s) => s.len(),
            Self::I32(s) => s.len(),
            Self::U32(s) => s.len(),
            Self::I64(s) => s.len(),
            Self::U64(s) => s.len(),
            Self::F32(s) => s.len(),
            Self::F64(s) => s.len(),
        }
    }

    /// Returns `true` if there are no pixels.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(feature = "std")]
impl ImageData {
    /// Borrows the data as [`Pixels`], for writing it back out.
    #[must_use]
    pub fn as_pixels(&self) -> Pixels<'_> {
        match self {
            Self::U8(v) => Pixels::U8(v),
            Self::I8(v) => Pixels::I8(v),
            Self::I16(v) => Pixels::I16(v),
            Self::U16(v) => Pixels::U16(v),
            Self::I32(v) => Pixels::I32(v),
            Self::U32(v) => Pixels::U32(v),
            Self::I64(v) => Pixels::I64(v),
            Self::U64(v) => Pixels::U64(v),
            Self::F32(v) => Pixels::F32(v),
            Self::F64(v) => Pixels::F64(v),
        }
    }
}

/// Writes a primary HDU from native pixels. The `BITPIX` and offset come
/// from the variant of `data`.
///
/// `header` holds the caller's cards. The writer generates `SIMPLE`,
/// `BITPIX`, `NAXIS`, `NAXISn`, `BSCALE`/`BZERO` when needed, and `EXTEND`,
/// and adds `END`. Those keywords are rejected in `header`.
///
/// # Errors
///
/// Returns an error if `dims` does not match the data length, a keyword is
/// invalid or reserved, or a card cannot be formatted in 80 columns.
#[cfg(feature = "std")]
pub fn write_primary(
    header: &[WriteCard<'_>],
    data: Pixels<'_>,
    dims: &[usize],
) -> Result<Vec<u8>> {
    let (bitpix, scaling, bytes) = encode_native(data);
    assemble(header, bitpix, scaling, dims, bytes)
}

/// Writes a primary HDU from `f64` values, stored as `bitpix`.
///
/// For integer `bitpix`, the stored value is `round((v - bzero) / bscale)`.
/// With [`Scaling::NONE`], every value must already be an integer in range,
/// so nothing is rounded silently. With other scaling the values are rounded
/// to the nearest stored value. Float `bitpix` does not accept scaling.
///
/// # Errors
///
/// Returns [`FitsError::NotRepresentable`] for a value that does not fit,
/// [`FitsError::Unsupported`] for scaling on float data, and the same header
/// and shape errors as [`write_primary`].
#[cfg(feature = "std")]
pub fn write_primary_f64(
    header: &[WriteCard<'_>],
    bitpix: Bitpix,
    scaling: Scaling,
    dims: &[usize],
    data: &[f64],
) -> Result<Vec<u8>> {
    if matches!(bitpix, Bitpix::F32 | Bitpix::F64) && scaling != Scaling::NONE {
        return Err(FitsError::Unsupported(
            "scaling applies only to integer BITPIX",
        ));
    }
    let mut bytes = Vec::with_capacity(data.len() * bitpix.bytes());
    for &v in data {
        push_f64(&mut bytes, bitpix, scaling, v)?;
    }
    assemble(header, bitpix, scaling, dims, bytes)
}

#[cfg(feature = "std")]
fn encode_native(data: Pixels<'_>) -> (Bitpix, Scaling, Vec<u8>) {
    let unsigned = |bzero: f64| Scaling { bscale: 1.0, bzero };
    match data {
        Pixels::U8(s) => (Bitpix::U8, Scaling::NONE, s.to_vec()),
        Pixels::I8(s) => (
            Bitpix::U8,
            unsigned(-128.0),
            s.iter().map(|&v| (v as u8) ^ 0x80).collect(),
        ),
        Pixels::I16(s) => (
            Bitpix::I16,
            Scaling::NONE,
            s.iter().flat_map(|v| v.to_be_bytes()).collect(),
        ),
        Pixels::U16(s) => (
            Bitpix::I16,
            unsigned(32768.0),
            s.iter().flat_map(|&v| (v ^ 0x8000).to_be_bytes()).collect(),
        ),
        Pixels::I32(s) => (
            Bitpix::I32,
            Scaling::NONE,
            s.iter().flat_map(|v| v.to_be_bytes()).collect(),
        ),
        Pixels::U32(s) => (
            Bitpix::I32,
            unsigned(2_147_483_648.0),
            s.iter()
                .flat_map(|&v| (v ^ 0x8000_0000).to_be_bytes())
                .collect(),
        ),
        Pixels::I64(s) => (
            Bitpix::I64,
            Scaling::NONE,
            s.iter().flat_map(|v| v.to_be_bytes()).collect(),
        ),
        Pixels::U64(s) => (
            Bitpix::I64,
            unsigned(9_223_372_036_854_775_808.0),
            s.iter()
                .flat_map(|&v| (v ^ (1 << 63)).to_be_bytes())
                .collect(),
        ),
        Pixels::F32(s) => (
            Bitpix::F32,
            Scaling::NONE,
            s.iter().flat_map(|v| v.to_be_bytes()).collect(),
        ),
        Pixels::F64(s) => (
            Bitpix::F64,
            Scaling::NONE,
            s.iter().flat_map(|v| v.to_be_bytes()).collect(),
        ),
    }
}

/// Appends one `f64` as a stored element of type `bitpix`.
#[cfg(feature = "std")]
fn push_f64(out: &mut Vec<u8>, bitpix: Bitpix, sc: Scaling, v: f64) -> Result<()> {
    match bitpix {
        Bitpix::F32 => out.extend_from_slice(&(v as f32).to_be_bytes()),
        Bitpix::F64 => out.extend_from_slice(&v.to_be_bytes()),
        Bitpix::U8 => out.push(stored_int(bitpix, sc, v)? as u8),
        Bitpix::I16 => out.extend_from_slice(&(stored_int(bitpix, sc, v)? as i16).to_be_bytes()),
        Bitpix::I32 => out.extend_from_slice(&(stored_int(bitpix, sc, v)? as i32).to_be_bytes()),
        Bitpix::I64 => out.extend_from_slice(&(stored_int(bitpix, sc, v)? as i64).to_be_bytes()),
    }
    Ok(())
}

/// Returns the stored integer for `v` as an `f64` already in range.
#[cfg(feature = "std")]
fn stored_int(bitpix: Bitpix, sc: Scaling, v: f64) -> Result<f64> {
    if !v.is_finite() {
        return Err(FitsError::NotRepresentable);
    }
    if sc == Scaling::NONE && v.fract() != 0.0 {
        return Err(FitsError::NotRepresentable);
    }
    let r = ((v - sc.bzero) / sc.bscale).round();
    // Half-open range [lo, hi) that holds every value of the type.
    let (lo, hi) = match bitpix {
        Bitpix::U8 => (0.0, 256.0),
        Bitpix::I16 => (-32768.0, 32768.0),
        Bitpix::I32 => (-2_147_483_648.0, 2_147_483_648.0),
        Bitpix::I64 => (-9_223_372_036_854_775_808.0, 9_223_372_036_854_775_808.0),
        Bitpix::F32 | Bitpix::F64 => return Ok(r),
    };
    if r >= lo && r < hi {
        Ok(r)
    } else {
        Err(FitsError::NotRepresentable)
    }
}

/// Builds header and data bytes and checks the shape against `data`.
#[cfg(feature = "std")]
fn assemble(
    header: &[WriteCard<'_>],
    bitpix: Bitpix,
    sc: Scaling,
    dims: &[usize],
    data: Vec<u8>,
) -> Result<Vec<u8>> {
    if dims.len() > MAX_AXES {
        return Err(FitsError::ShapeMismatch);
    }
    if !sc.bscale.is_finite() || sc.bscale == 0.0 || !sc.bzero.is_finite() {
        return Err(FitsError::Unsupported(
            "BSCALE must be finite and non-zero, BZERO finite",
        ));
    }
    let count = data.len() / bitpix.bytes();
    let expected = if dims.is_empty() {
        0
    } else {
        dims.iter()
            .try_fold(1usize, |acc, &d| acc.checked_mul(d))
            .ok_or(FitsError::ShapeMismatch)?
    };
    if count != expected {
        return Err(FitsError::ShapeMismatch);
    }

    let mut cards = Vec::with_capacity(CARD * (8 + dims.len() + header.len()));
    push_card(
        &mut cards,
        &logical("SIMPLE", true, "conforms to FITS standard"),
    )?;
    push_card(
        &mut cards,
        &WriteCard {
            keyword: "BITPIX",
            value: Some(Value::Integer(bitpix.code())),
            comment: "array data type",
        },
    )?;
    push_card(
        &mut cards,
        &WriteCard {
            keyword: "NAXIS",
            value: Some(Value::Integer(dims.len() as i64)),
            comment: "number of array dimensions",
        },
    )?;
    for (i, &d) in dims.iter().enumerate() {
        let kw = format_axis(i + 1);
        push_card(
            &mut cards,
            &WriteCard {
                keyword: &kw,
                value: Some(Value::Integer(d as i64)),
                comment: "",
            },
        )?;
    }
    if sc != Scaling::NONE {
        push_card(
            &mut cards,
            &WriteCard {
                keyword: "BSCALE",
                value: Some(Value::Float(sc.bscale)),
                comment: "",
            },
        )?;
        push_card(
            &mut cards,
            &WriteCard {
                keyword: "BZERO",
                value: Some(Value::Float(sc.bzero)),
                comment: "",
            },
        )?;
    }
    push_card(
        &mut cards,
        &logical("EXTEND", true, "file may contain extensions"),
    )?;
    for card in header {
        if is_reserved(card.keyword) {
            return Err(FitsError::InvalidKeyword);
        }
        push_card(&mut cards, card)?;
    }
    let mut end = [b' '; CARD];
    end[..3].copy_from_slice(b"END");
    cards.extend_from_slice(&end);
    cards.resize(cards.len().div_ceil(BLOCK) * BLOCK, b' ');

    let padded = data.len().div_ceil(BLOCK) * BLOCK;
    let mut out = cards;
    out.extend_from_slice(&data);
    out.resize(out.len() + (padded - data.len()), 0);
    Ok(out)
}

#[cfg(feature = "std")]
fn logical(keyword: &'static str, value: bool, comment: &'static str) -> WriteCard<'static> {
    WriteCard {
        keyword,
        value: Some(Value::Logical(value)),
        comment,
    }
}

/// Keywords the writer generates itself. Callers may not supply them.
#[cfg(feature = "std")]
fn is_reserved(keyword: &str) -> bool {
    matches!(
        keyword,
        "SIMPLE"
            | "BITPIX"
            | "EXTEND"
            | "BSCALE"
            | "BZERO"
            | "END"
            | "GROUPS"
            | "PCOUNT"
            | "GCOUNT"
            | "XTENSION"
    ) || keyword.starts_with("NAXIS")
}

/// Appends one formatted 80-byte card to `out`.
#[cfg(feature = "std")]
fn push_card(out: &mut Vec<u8>, card: &WriteCard<'_>) -> Result<()> {
    if card.keyword.len() > 8
        || !card
            .keyword
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
    {
        return Err(FitsError::InvalidKeyword);
    }
    let (mut line, sep) = match card.value {
        None => (format!("{:<8}", card.keyword), ""),
        Some(v) => {
            let field = format_value(v)?;
            let sep = if card.comment.is_empty() { "" } else { " / " };
            (format!("{:<8}= {field}", card.keyword), sep)
        }
    };
    if !line.bytes().all(|b| (0x20..=0x7E).contains(&b)) || line.len() > CARD {
        return Err(FitsError::InvalidCardText);
    }
    if !card.comment.is_empty() {
        if !card.comment.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
            return Err(FitsError::InvalidCardText);
        }
        // Keep the separator only if some comment fits after it.
        let room = CARD.saturating_sub(line.len() + sep.len());
        if room > 0 {
            let comment = card
                .comment
                .get(..room.min(card.comment.len()))
                .unwrap_or("");
            line.push_str(sep);
            line.push_str(comment);
        }
    }
    line.push_str(&" ".repeat(CARD - line.len()));
    out.extend_from_slice(line.as_bytes());
    Ok(())
}

/// Formats a value field: right-justified to column 30 for logicals and
/// numbers, quoted and left-justified from column 11 for strings.
#[cfg(feature = "std")]
fn format_value(v: Value<'_>) -> Result<String> {
    Ok(match v {
        Value::Logical(b) => format!("{:>20}", if b { "T" } else { "F" }),
        Value::Integer(i) => format!("{i:>20}"),
        Value::Float(f) => format!("{:>20}", format_float(f)?),
        Value::Text(t) => {
            if !t.bytes().all(|b| (0x20..=0x7E).contains(&b)) {
                return Err(FitsError::InvalidCardText);
            }
            let escaped = t.replace('\'', "''");
            if escaped.len() > 68 {
                return Err(FitsError::InvalidCardText);
            }
            format!("'{escaped:<8}'")
        }
    })
}

/// Formats a finite float in FITS `E` notation, with a sign and two or more
/// exponent digits.
#[cfg(feature = "std")]
fn format_float(f: f64) -> Result<String> {
    if !f.is_finite() {
        return Err(FitsError::NotRepresentable);
    }
    let s = format!("{f:E}");
    let (mant, exp) = s.split_once('E').ok_or(FitsError::NotRepresentable)?;
    let exp: i32 = exp.parse().map_err(|_| FitsError::NotRepresentable)?;
    let mant = if mant.contains('.') {
        mant.to_string()
    } else {
        format!("{mant}.0")
    };
    let sign = if exp < 0 { '-' } else { '+' };
    Ok(format!("{mant}E{sign}{:02}", exp.abs()))
}

/// The storage type of one column of a binary table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// `L`: logical, one byte per element (`T` or `F`).
    Logical,
    /// `B`: unsigned byte.
    Byte,
    /// `I`: 16-bit signed integer.
    Short,
    /// `J`: 32-bit signed integer.
    Int,
    /// `K`: 64-bit signed integer.
    Long,
    /// `E`: 32-bit float.
    Float,
    /// `D`: 64-bit float.
    Double,
    /// `A`: character string, `repeat` bytes per row.
    Char,
    /// A form this module does not read (bit, complex, variable-length, ...).
    Other(char),
}

/// One column of a binary table, with the layout and scaling from its header.
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Column name (`TTYPEn`), or empty if the header has none.
    pub name: String,
    /// Storage type.
    pub kind: ColumnKind,
    /// Number of elements per row (for `A`, the number of bytes per row).
    pub repeat: usize,
    /// Byte offset of the column within a row.
    offset: usize,
    /// Physical value = `zero + scale * stored`.
    scale: f64,
    zero: f64,
    /// Integer value that stands for an undefined element (`TNULLn`).
    null: Option<i64>,
}

/// A binary table extension: rows of fixed-width columns.
#[derive(Debug, Clone)]
pub struct Table<'a> {
    data: &'a [u8],
    rows: usize,
    row_bytes: usize,
    columns: Vec<Column>,
}

/// Parses a `TFORMn` value into `(repeat, type code)`. The repeat count
/// defaults to 1; a value such as `'10A'` gives `(10, 'A')`.
fn parse_tform(text: &str) -> Option<(usize, char)> {
    let t = text.trim();
    let digits = t.bytes().take_while(u8::is_ascii_digit).count();
    let repeat = if digits == 0 {
        1
    } else {
        t[..digits].parse().ok()?
    };
    let code = t[digits..].chars().next()?;
    Some((repeat, code))
}

/// Column kind and bytes per element of a `TFORM` type code.
fn element_kind(code: char) -> (ColumnKind, usize) {
    match code {
        'L' => (ColumnKind::Logical, 1),
        'B' => (ColumnKind::Byte, 1),
        'I' => (ColumnKind::Short, 2),
        'J' => (ColumnKind::Int, 4),
        'K' => (ColumnKind::Long, 8),
        'E' => (ColumnKind::Float, 4),
        'D' => (ColumnKind::Double, 8),
        'A' => (ColumnKind::Char, 1),
        other => (ColumnKind::Other(other), 0),
    }
}

fn text_value(v: Value<'_>) -> Option<&str> {
    match v {
        Value::Text(t) => Some(t),
        _ => None,
    }
}

impl<'a> Hdu<'a> {
    /// Reads this binary table extension.
    ///
    /// Fails with [`FitsError::InvalidHeader`] if the HDU is not a table or a
    /// column description is inconsistent with `NAXIS1`, and with
    /// [`FitsError::Unsupported`] for column forms other than the fixed-width
    /// ones (`L`, `B`, `I`, `J`, `K`, `E`, `D`, `A`).
    pub fn table(&self) -> Result<Table<'a>> {
        if !self.table {
            return Err(FitsError::InvalidHeader("the HDU is not a binary table"));
        }
        let header = self.header;
        let row_bytes = header
            .get("NAXIS1")
            .and_then(|v| v.as_i64())
            .and_then(|v| usize::try_from(v).ok())
            .ok_or(FitsError::InvalidHeader("NAXIS1 (row width) missing"))?;
        let rows = header
            .get("NAXIS2")
            .and_then(|v| v.as_i64())
            .and_then(|v| usize::try_from(v).ok())
            .ok_or(FitsError::InvalidHeader("NAXIS2 (row count) missing"))?;
        let fields = header
            .get("TFIELDS")
            .and_then(|v| v.as_i64())
            .and_then(|v| usize::try_from(v).ok())
            .ok_or(FitsError::InvalidHeader("TFIELDS missing"))?;
        let table_len = rows
            .checked_mul(row_bytes)
            .ok_or(FitsError::InvalidHeader("table size overflows"))?;
        if table_len > self.data.len() {
            return Err(FitsError::Truncated(
                "table data is shorter than NAXIS1 x NAXIS2",
            ));
        }

        let mut columns = Vec::with_capacity(fields);
        let mut offset = 0usize;
        for n in 1..=fields {
            let form = header
                .get(&format!("TFORM{n}"))
                .and_then(|v| text_value(v))
                .ok_or(FitsError::InvalidHeader("TFORMn missing or not a string"))?;
            let (repeat, code) =
                parse_tform(form).ok_or(FitsError::InvalidHeader("TFORMn is malformed"))?;
            let (kind, size) = element_kind(code);
            if let ColumnKind::Other(_) = kind {
                return Err(FitsError::Unsupported(
                    "this TFORM type (only L, B, I, J, K, E, D and A are read)",
                ));
            }
            let width = if kind == ColumnKind::Char {
                repeat
            } else {
                size * repeat
            };
            let name = header
                .get(&format!("TTYPE{n}"))
                .and_then(|v| text_value(v))
                .map(|t| t.trim_end().to_string())
                .unwrap_or_default();
            let scale = header
                .get(&format!("TSCAL{n}"))
                .and_then(|v| v.as_f64())
                .unwrap_or(1.0);
            let zero = header
                .get(&format!("TZERO{n}"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let null = header.get(&format!("TNULL{n}")).and_then(|v| v.as_i64());
            columns.push(Column {
                name,
                kind,
                repeat,
                offset,
                scale,
                zero,
                null,
            });
            offset += width;
        }
        if offset > row_bytes {
            return Err(FitsError::InvalidHeader(
                "the columns are wider than NAXIS1",
            ));
        }
        Ok(Table {
            data: &self.data[..table_len],
            rows,
            row_bytes,
            columns,
        })
    }
}

impl<'a> Table<'a> {
    /// Number of rows.
    #[must_use]
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Bytes per row (`NAXIS1`).
    #[must_use]
    pub fn row_bytes(&self) -> usize {
        self.row_bytes
    }

    /// The columns, in file order.
    #[must_use]
    pub fn columns(&self) -> &[Column] {
        &self.columns
    }

    /// The column called `name`, if there is one.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }

    /// Values of a numeric or logical column as `f64`, row by row. A column
    /// with `repeat > 1` gives `rows * repeat` values, row-major. Scaling
    /// (`TSCALn`, `TZEROn`) is applied; undefined values (`TNULLn`, or a
    /// logical that is neither `T` nor `F`) become NaN.
    pub fn read_f64(&self, name: &str) -> Result<Vec<f64>> {
        let col = self
            .column(name)
            .ok_or(FitsError::InvalidHeader("no column with that name"))?;
        let size = match col.kind {
            ColumnKind::Logical | ColumnKind::Byte => 1,
            ColumnKind::Short => 2,
            ColumnKind::Int | ColumnKind::Float => 4,
            ColumnKind::Long | ColumnKind::Double => 8,
            ColumnKind::Char | ColumnKind::Other(_) => {
                return Err(FitsError::Unsupported(
                    "read_f64 needs a numeric or logical column",
                ));
            }
        };
        let mut out = Vec::with_capacity(self.rows * col.repeat);
        for row in 0..self.rows {
            let base = row * self.row_bytes + col.offset;
            for k in 0..col.repeat {
                let at = base + k * size;
                out.push(decode_element(col, &self.data[at..at + size]));
            }
        }
        Ok(out)
    }

    /// Text of an `A` column, one string per row, with trailing spaces and NUL
    /// bytes removed.
    pub fn read_strings(&self, name: &str) -> Result<Vec<String>> {
        let col = self
            .column(name)
            .ok_or(FitsError::InvalidHeader("no column with that name"))?;
        if col.kind != ColumnKind::Char {
            return Err(FitsError::InvalidHeader("read_strings needs an A column"));
        }
        Ok((0..self.rows)
            .map(|row| {
                let at = row * self.row_bytes + col.offset;
                let text: String = self.data[at..at + col.repeat]
                    .iter()
                    .map(|&b| char::from(b))
                    .collect();
                text.trim_end_matches([' ', '\0']).to_string()
            })
            .collect())
    }
}

/// The physical value of one stored element of a numeric or logical column.
fn decode_element(col: &Column, raw: &[u8]) -> f64 {
    let integer = |v: i64| {
        if col.null == Some(v) {
            f64::NAN
        } else {
            col.zero + col.scale * v as f64
        }
    };
    match col.kind {
        ColumnKind::Logical => match raw[0] {
            b'T' => 1.0,
            b'F' => 0.0,
            _ => f64::NAN,
        },
        ColumnKind::Byte => integer(i64::from(raw[0])),
        ColumnKind::Short => integer(i64::from(i16::from_be_bytes([raw[0], raw[1]]))),
        ColumnKind::Int => integer(i64::from(i32::from_be_bytes([
            raw[0], raw[1], raw[2], raw[3],
        ]))),
        ColumnKind::Long => integer(i64::from_be_bytes([
            raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
        ])),
        ColumnKind::Float => {
            let v = f64::from(f32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]));
            col.zero + col.scale * v
        }
        ColumnKind::Double => {
            let v = f64::from_be_bytes([
                raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
            ]);
            col.zero + col.scale * v
        }
        ColumnKind::Char | ColumnKind::Other(_) => f64::NAN,
    }
}

/// Values of one column of a binary table, as stored (big-endian on disk).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnData<'a> {
    /// `L`: logical values, stored as `T` or `F`.
    Logical(&'a [bool]),
    /// `B`: unsigned bytes.
    Byte(&'a [u8]),
    /// `I`: 16-bit signed integers.
    Short(&'a [i16]),
    /// `J`: 32-bit signed integers.
    Int(&'a [i32]),
    /// `K`: 64-bit signed integers.
    Long(&'a [i64]),
    /// `E`: 32-bit floats.
    Float(&'a [f32]),
    /// `D`: 64-bit floats.
    Double(&'a [f64]),
    /// `A`: one string per row, each padded with spaces to the width (bytes).
    Text(&'a [&'a str], usize),
}

/// One column of a binary table to write.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TableColumn<'a> {
    /// Column name (`TTYPEn`).
    pub name: &'a str,
    /// Values, row by row: `repeat` values per row for numeric and logical columns.
    pub data: ColumnData<'a>,
    /// Elements per row (1 for a scalar column). Not used by `Text`, whose width
    /// is part of the data.
    pub repeat: usize,
    /// `TSCALn`: physical = `tzero + tscal * stored`. Use 1.0 for none.
    pub tscal: f64,
    /// `TZEROn`. Use 0.0 for none.
    pub tzero: f64,
    /// `TNULLn`: the stored integer that means undefined (integer columns only).
    pub tnull: Option<i64>,
}

impl ColumnData<'_> {
    /// The `TFORM` type letter and bytes per element.
    fn code_and_size(&self) -> (char, usize) {
        match self {
            ColumnData::Logical(_) => ('L', 1),
            ColumnData::Byte(_) => ('B', 1),
            ColumnData::Short(_) => ('I', 2),
            ColumnData::Int(_) => ('J', 4),
            ColumnData::Long(_) => ('K', 8),
            ColumnData::Float(_) => ('E', 4),
            ColumnData::Double(_) => ('D', 8),
            ColumnData::Text(_, _) => ('A', 1),
        }
    }

    /// Number of stored values (not rows).
    fn value_count(&self) -> usize {
        match self {
            ColumnData::Logical(v) => v.len(),
            ColumnData::Byte(v) => v.len(),
            ColumnData::Short(v) => v.len(),
            ColumnData::Int(v) => v.len(),
            ColumnData::Long(v) => v.len(),
            ColumnData::Float(v) => v.len(),
            ColumnData::Double(v) => v.len(),
            ColumnData::Text(v, _) => v.len(),
        }
    }
}

/// Appends the big-endian bytes of the `repeat` values of one row of a column.
fn push_row_values(data: &mut Vec<u8>, col: &TableColumn<'_>, row: usize) {
    let r = col.repeat;
    match col.data {
        ColumnData::Logical(v) => {
            for k in 0..r {
                data.push(if v[row * r + k] { b'T' } else { b'F' });
            }
        }
        ColumnData::Byte(v) => data.extend_from_slice(&v[row * r..row * r + r]),
        ColumnData::Short(v) => {
            for x in &v[row * r..row * r + r] {
                data.extend_from_slice(&x.to_be_bytes());
            }
        }
        ColumnData::Int(v) => {
            for x in &v[row * r..row * r + r] {
                data.extend_from_slice(&x.to_be_bytes());
            }
        }
        ColumnData::Long(v) => {
            for x in &v[row * r..row * r + r] {
                data.extend_from_slice(&x.to_be_bytes());
            }
        }
        ColumnData::Float(v) => {
            for x in &v[row * r..row * r + r] {
                data.extend_from_slice(&x.to_be_bytes());
            }
        }
        ColumnData::Double(v) => {
            for x in &v[row * r..row * r + r] {
                data.extend_from_slice(&x.to_be_bytes());
            }
        }
        ColumnData::Text(v, width) => {
            let s = v[row].as_bytes();
            let take = s.len().min(width);
            data.extend_from_slice(&s[..take]);
            data.extend(core::iter::repeat_n(b' ', width - take));
        }
    }
}

/// Writes a binary table: an empty primary HDU followed by a `BINTABLE` extension
/// holding `columns`. `header` adds cards to the extension header (for example
/// `EXTNAME`).
///
/// # Errors
///
/// [`FitsError::ShapeMismatch`] if the columns disagree on the row count, a
/// value count is not a whole number of rows, or a string is wider than its column; [`FitsError::Unsupported`] for a
/// non-finite `tscal` or `tzero`; [`FitsError::InvalidKeyword`] for a reserved
/// keyword in `header`.
pub fn write_bintable(columns: &[TableColumn<'_>], header: &[WriteCard<'_>]) -> Result<Vec<u8>> {
    let mut rows: Option<usize> = None;
    let mut width = 0usize;
    for c in columns {
        if !c.tscal.is_finite() || !c.tzero.is_finite() {
            return Err(FitsError::Unsupported("TSCAL and TZERO must be finite"));
        }
        let (_, size) = c.data.code_and_size();
        let (per_row, n_rows) = match c.data {
            ColumnData::Text(values, w) => {
                // A string longer than its column would be truncated: refuse.
                if values.iter().any(|v| v.len() > w) {
                    return Err(FitsError::ShapeMismatch);
                }
                (w, values.len())
            }
            _ => {
                if c.repeat == 0 || c.data.value_count() % c.repeat != 0 {
                    return Err(FitsError::ShapeMismatch);
                }
                (c.repeat * size, c.data.value_count() / c.repeat)
            }
        };
        match rows {
            None => rows = Some(n_rows),
            Some(r) if r != n_rows => return Err(FitsError::ShapeMismatch),
            Some(_) => {}
        }
        width += per_row;
    }
    let rows = rows.unwrap_or(0);

    let mut data = Vec::with_capacity(width * rows);
    for row in 0..rows {
        for c in columns {
            push_row_values(&mut data, c, row);
        }
    }

    // Primary HDU: no data.
    let mut primary = Vec::new();
    push_card(
        &mut primary,
        &logical("SIMPLE", true, "conforms to FITS standard"),
    )?;
    push_card(
        &mut primary,
        &WriteCard {
            keyword: "BITPIX",
            value: Some(Value::Integer(8)),
            comment: "",
        },
    )?;
    push_card(
        &mut primary,
        &WriteCard {
            keyword: "NAXIS",
            value: Some(Value::Integer(0)),
            comment: "",
        },
    )?;
    push_card(
        &mut primary,
        &logical("EXTEND", true, "file has extensions"),
    )?;
    let mut end = [b' '; CARD];
    end[..3].copy_from_slice(b"END");
    primary.extend_from_slice(&end);
    primary.resize(primary.len().div_ceil(BLOCK) * BLOCK, b' ');
    let primary_bytes = primary;

    // Extension header.
    let mut ext = Vec::new();
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "XTENSION",
            value: Some(Value::Text("BINTABLE")),
            comment: "binary table extension",
        },
    )?;
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "BITPIX",
            value: Some(Value::Integer(8)),
            comment: "",
        },
    )?;
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "NAXIS",
            value: Some(Value::Integer(2)),
            comment: "",
        },
    )?;
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "NAXIS1",
            value: Some(Value::Integer(width as i64)),
            comment: "bytes per row",
        },
    )?;
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "NAXIS2",
            value: Some(Value::Integer(rows as i64)),
            comment: "number of rows",
        },
    )?;
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "PCOUNT",
            value: Some(Value::Integer(0)),
            comment: "",
        },
    )?;
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "GCOUNT",
            value: Some(Value::Integer(1)),
            comment: "",
        },
    )?;
    push_card(
        &mut ext,
        &WriteCard {
            keyword: "TFIELDS",
            value: Some(Value::Integer(columns.len() as i64)),
            comment: "",
        },
    )?;
    for (i, c) in columns.iter().enumerate() {
        let n = i + 1;
        let (code, _) = c.data.code_and_size();
        let form = match c.data {
            ColumnData::Text(_, w) => format!("{w}A"),
            _ => format!("{}{code}", c.repeat),
        };
        let ttype = format!("TTYPE{n}");
        let tform = format!("TFORM{n}");
        push_card(
            &mut ext,
            &WriteCard {
                keyword: &ttype,
                value: Some(Value::Text(c.name)),
                comment: "",
            },
        )?;
        push_card(
            &mut ext,
            &WriteCard {
                keyword: &tform,
                value: Some(Value::Text(&form)),
                comment: "",
            },
        )?;
        let numeric = !matches!(c.data, ColumnData::Text(..) | ColumnData::Logical(_));
        if numeric && c.tscal != 1.0 {
            let k = format!("TSCAL{n}");
            push_card(
                &mut ext,
                &WriteCard {
                    keyword: &k,
                    value: Some(Value::Float(c.tscal)),
                    comment: "",
                },
            )?;
        }
        if numeric && c.tzero != 0.0 {
            let k = format!("TZERO{n}");
            push_card(
                &mut ext,
                &WriteCard {
                    keyword: &k,
                    value: Some(Value::Float(c.tzero)),
                    comment: "",
                },
            )?;
        }
        if let (true, Some(null)) = (numeric, c.tnull) {
            let k = format!("TNULL{n}");
            push_card(
                &mut ext,
                &WriteCard {
                    keyword: &k,
                    value: Some(Value::Integer(null)),
                    comment: "",
                },
            )?;
        }
    }
    for card in header {
        if is_reserved(card.keyword) {
            return Err(FitsError::InvalidKeyword);
        }
        push_card(&mut ext, card)?;
    }
    ext.extend_from_slice(&end);
    ext.resize(ext.len().div_ceil(BLOCK) * BLOCK, b' ');

    let padded = data.len().div_ceil(BLOCK) * BLOCK;
    let mut out = primary_bytes;
    out.extend_from_slice(&ext);
    out.extend_from_slice(&data);
    out.resize(out.len() + (padded - data.len()), 0);
    Ok(out)
}

#[cfg(test)]
#[cfg(feature = "std")]
mod tests {
    use super::*;
    use crate::wcs::Wcs;

    const BITPIX8: &[u8] = include_bytes!("testdata/bitpix8.fits");
    const BITPIX16: &[u8] = include_bytes!("testdata/bitpix16.fits");
    const BITPIX32: &[u8] = include_bytes!("testdata/bitpix32.fits");
    const BITPIX64: &[u8] = include_bytes!("testdata/bitpix64.fits");
    const BITPIXM32: &[u8] = include_bytes!("testdata/bitpixm32.fits");
    const BITPIXM64: &[u8] = include_bytes!("testdata/bitpixm64.fits");
    const UINT16: &[u8] = include_bytes!("testdata/uint16.fits");
    const UINT32: &[u8] = include_bytes!("testdata/uint32.fits");
    const UINT64: &[u8] = include_bytes!("testdata/uint64.fits");
    const MULTI: &[u8] = include_bytes!("testdata/multi.fits");
    const BINTABLE: &[u8] = include_bytes!("testdata/bintable.fits");
    const WCS_TAN: &[u8] = include_bytes!("testdata/wcs_tan.fits");

    const REF_BITPIX8: &[u8] = include_bytes!("testdata/bitpix8.f64be");
    const REF_BITPIX16: &[u8] = include_bytes!("testdata/bitpix16.f64be");
    const REF_BITPIX32: &[u8] = include_bytes!("testdata/bitpix32.f64be");
    const REF_BITPIX64: &[u8] = include_bytes!("testdata/bitpix64.f64be");
    const REF_BITPIXM32: &[u8] = include_bytes!("testdata/bitpixm32.f64be");
    const REF_BITPIXM64: &[u8] = include_bytes!("testdata/bitpixm64.f64be");
    const REF_UINT16: &[u8] = include_bytes!("testdata/uint16.f64be");
    const REF_UINT32: &[u8] = include_bytes!("testdata/uint32.f64be");
    const REF_UINT64: &[u8] = include_bytes!("testdata/uint64.f64be");
    const REF_MULTI0: &[u8] = include_bytes!("testdata/multi_0.f64be");
    const REF_MULTI1: &[u8] = include_bytes!("testdata/multi_1.f64be");
    const REF_MULTI2: &[u8] = include_bytes!("testdata/multi_2.f64be");
    const REF_WCS: &[u8] = include_bytes!("testdata/wcs_tan.f64be");

    fn ref_values(b: &[u8]) -> Vec<f64> {
        b.chunks_exact(8)
            .filter_map(|c| c.first_chunk::<8>().map(|a| f64::from_be_bytes(*a)))
            .collect()
    }

    /// Bitwise equality, with all NaNs equal.
    fn same(a: &[f64], b: &[f64]) -> bool {
        a.len() == b.len()
            && a.iter()
                .zip(b)
                .all(|(x, y)| (x.is_nan() && y.is_nan()) || x.to_bits() == y.to_bits())
    }

    /// Deterministic LCG for fuzz inputs, with no external crate.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> u32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 33) as u32
        }
    }

    #[test]
    fn astropy_fixtures_match_reference() {
        let cases: [(&[u8], &[u8], Bitpix); 9] = [
            (BITPIX8, REF_BITPIX8, Bitpix::U8),
            (BITPIX16, REF_BITPIX16, Bitpix::I16),
            (BITPIX32, REF_BITPIX32, Bitpix::I32),
            (BITPIX64, REF_BITPIX64, Bitpix::I64),
            (BITPIXM32, REF_BITPIXM32, Bitpix::F32),
            (BITPIXM64, REF_BITPIXM64, Bitpix::F64),
            (UINT16, REF_UINT16, Bitpix::I16),
            (UINT32, REF_UINT32, Bitpix::I32),
            (UINT64, REF_UINT64, Bitpix::I64),
        ];
        for (file, reference, bitpix) in cases {
            let hdu = primary(file).unwrap();
            assert_eq!(hdu.bitpix(), bitpix);
            assert_eq!(hdu.naxis(), 2);
            assert_eq!((hdu.axis_len(1), hdu.axis_len(2)), (5, 4));
            let got: Vec<f64> = hdu.values().collect();
            assert!(
                same(&got, &ref_values(reference)),
                "mismatch for {bitpix:?}"
            );
        }
    }

    #[test]
    fn header_keywords_and_scaling() {
        let hdu = primary(UINT16).unwrap();
        assert_eq!(hdu.header().get("BZERO"), Some(Value::Integer(32768)));
        assert_eq!(hdu.bzero(), 32768.0);
        assert_eq!(hdu.bscale(), 1.0);
        assert_eq!(hdu.header().get("EXTEND"), Some(Value::Logical(true)));
        assert_eq!(hdu.header().byte_len() % BLOCK, 0);

        let multi = primary(MULTI).unwrap();
        assert_eq!((multi.bscale(), multi.bzero()), (0.5, 10.0));
    }

    #[test]
    fn native_types_for_conventional_offsets() {
        match primary(UINT16).unwrap().to_native().unwrap() {
            ImageData::U16(v) => assert_eq!(&v[..5], &[0, 1, 65535, 32768, 40000]),
            other => panic!("expected U16, got {other:?}"),
        }
        match primary(UINT32).unwrap().to_native().unwrap() {
            ImageData::U32(v) => assert_eq!(v[2], 4_294_967_295),
            other => panic!("expected U32, got {other:?}"),
        }
        match primary(UINT64).unwrap().to_native().unwrap() {
            ImageData::U64(v) => assert_eq!(v[0], 0),
            other => panic!("expected U64, got {other:?}"),
        }
        match primary(BITPIX8).unwrap().to_native().unwrap() {
            ImageData::U8(v) => assert_eq!(v.len(), 20),
            other => panic!("expected U8, got {other:?}"),
        }
        // Scaled data has no native form.
        assert!(primary(MULTI).unwrap().to_native().is_err());
    }

    #[test]
    fn multi_hdu_file() {
        let all: Vec<Hdu<'_>> = hdus(MULTI).collect::<Result<_>>().unwrap();
        assert_eq!(all.len(), 3);
        // The .f64be files hold the stored (unscaled) values of HDUs 0 and 2,
        // so the physical expectation is BZERO + BSCALE * stored. HDU 1 is
        // unscaled and its file is the physical value.
        for (hdu, reference) in all.iter().zip([REF_MULTI0, REF_MULTI1, REF_MULTI2]) {
            let got: Vec<f64> = hdu.values().collect();
            let expected: Vec<f64> = ref_values(reference)
                .into_iter()
                .map(|s| hdu.bzero() + hdu.bscale() * s)
                .collect();
            assert!(same(&got, &expected));
        }
        assert_eq!(all[1].header().get("EXTNAME"), Some(Value::Text("SCI")));
        assert_eq!(all[2].bzero(), 100.0);
        assert_eq!(all[2].bscale(), 2.0);
    }

    #[test]
    fn table_extension_is_yielded_and_read() {
        // BINTABLE is an HDU; its table() reading is tested in tests/fits_table.rs.
        let mut it = hdus(BINTABLE);
        assert!(it.next().unwrap().is_ok());
        let table = it.next().unwrap().unwrap();
        assert!(table.is_table());
        assert!(it.next().is_none());
    }

    #[test]
    fn ascii_table_extension_is_rejected() {
        // Same length as 'BINTABLE', so the header layout is unchanged.
        let mut ascii = BINTABLE.to_vec();
        let at = ascii
            .windows(10)
            .position(|w| w == b"'BINTABLE'")
            .expect("XTENSION keyword");
        ascii[at..at + 10].copy_from_slice(b"'TABLE   '");
        let mut it = hdus(&ascii);
        assert!(it.next().unwrap().is_ok());
        assert!(matches!(it.next(), Some(Err(FitsError::Unsupported(_)))));
    }

    #[test]
    fn wcs_accepts_header_text() {
        let hdu = primary(WCS_TAN).unwrap();
        let wcs = Wcs::from_header(hdu.header().as_text()).unwrap();
        let (ra, dec) = wcs.pixel_to_world(4.5, 3.5).unwrap();
        assert!((ra - 150.25).abs() < 1e-9 && (dec - 2.5).abs() < 1e-9);
        let reference = ref_values(REF_WCS);
        assert_eq!(reference.len(), 48);
    }

    #[test]
    fn truncated_input_is_an_error_not_a_panic() {
        // BITPIX 16, 5x4 pixels: header is one block, data is 40 bytes.
        let data_end = BLOCK + 40;
        for n in 0..data_end {
            assert!(primary(&BITPIX16[..n]).is_err(), "prefix {n} accepted");
        }
        for n in data_end..BITPIX16.len() {
            assert!(primary(&BITPIX16[..n]).is_ok(), "prefix {n} rejected");
        }
        for n in 0..MULTI.len() {
            for h in hdus(&MULTI[..n]).flatten() {
                let _ = h.values().count();
            }
        }
    }

    #[test]
    fn garbage_and_mutated_input_never_panics() {
        let mut rng = Lcg(0x5eed_f175);
        for _ in 0..2000 {
            let len = (rng.next() % 9000) as usize;
            let buf: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
            for h in hdus(&buf).flatten() {
                let _ = h.values().count();
                let _ = h.to_native();
            }
        }
        let fixtures = [BITPIX16, UINT32, MULTI];
        for _ in 0..1500 {
            let base = fixtures[(rng.next() % 3) as usize];
            let mut buf = base.to_vec();
            for _ in 0..(1 + rng.next() % 8) {
                let pos = (rng.next() as usize) % buf.len().min(3 * BLOCK);
                buf[pos] = rng.next() as u8;
            }
            for h in hdus(&buf).flatten() {
                let _ = h.values().count();
                let _ = h.to_native();
            }
        }
    }

    #[test]
    fn card_parsing_rules() {
        let mut text = Vec::new();
        for c in [
            "SIMPLE  =                    T / conforms",
            "BITPIX  =                   -8 / bits",
            "NAXIS   =                    0",
            "FLOATD  =              1.5D+02 / Fortran exponent",
            "STR     = 'it''s ok '           / quoted",
            "NOVALUE =",
            "COMMENT  free text here",
            "END",
        ] {
            let mut card = [b' '; CARD];
            card[..c.len()].copy_from_slice(c.as_bytes());
            text.extend_from_slice(&card);
        }
        text.resize(BLOCK, b' ');
        let (header, _) = Header::parse(&text, 0).unwrap();
        let cards: Vec<Card<'_>> = header.cards().collect();
        assert_eq!(cards.len(), 7);
        assert_eq!(cards[0].value, Some(Value::Logical(true)));
        assert_eq!(cards[0].comment, "conforms");
        assert_eq!(cards[1].value, Some(Value::Integer(-8)));
        assert_eq!(cards[3].value, Some(Value::Float(150.0)));
        assert_eq!(cards[4].value, Some(Value::Text("it''s ok")));
        assert_eq!(cards[4].comment, "quoted");
        assert_eq!(cards[5].value, None);
        assert_eq!(cards[6].keyword, "COMMENT");
        assert_eq!(cards[6].value, None);
        assert_eq!(cards[6].comment, "free text here");
    }

    #[test]
    fn continue_cards_are_rejected() {
        let mut text = vec![b' '; BLOCK];
        text[..8].copy_from_slice(b"CONTINUE");
        text[8..10].copy_from_slice(b"  ");
        let mut end = [b' '; CARD];
        end[..3].copy_from_slice(b"END");
        text[CARD..2 * CARD].copy_from_slice(&end);
        assert!(matches!(
            Header::parse(&text, 0),
            Err(FitsError::Unsupported(_))
        ));
    }

    #[test]
    fn writer_round_trips_every_bitpix() {
        let dims = [4usize, 3];
        let ints: Vec<f64> = (0..12).map(|i| f64::from(i) - 5.0).collect();
        let floats: Vec<f64> = (0..12).map(|i| f64::from(i) * 0.25 - 1.5).collect();
        for (bitpix, values) in [
            (Bitpix::I16, &ints),
            (Bitpix::I32, &ints),
            (Bitpix::I64, &ints),
            (Bitpix::F32, &floats),
            (Bitpix::F64, &floats),
        ] {
            let bytes = write_primary_f64(&[], bitpix, Scaling::NONE, &dims, values).unwrap();
            assert_eq!(bytes.len() % BLOCK, 0);
            let hdu = primary(&bytes).unwrap();
            assert_eq!(hdu.bitpix(), bitpix);
            assert_eq!((hdu.axis_len(1), hdu.axis_len(2)), (4, 3));
            let got: Vec<f64> = hdu.values().collect();
            assert!(same(&got, values), "round trip failed for {bitpix:?}");
        }
        let bytes = write_primary_f64(&[], Bitpix::U8, Scaling::NONE, &dims, &[0.0; 12]).unwrap();
        assert_eq!(primary(&bytes).unwrap().values().count(), 12);
    }

    #[test]
    fn writer_scaling_and_native_round_trip() {
        let sc = Scaling {
            bscale: 0.5,
            bzero: 10.0,
        };
        let bytes = write_primary_f64(&[], Bitpix::I16, sc, &[3], &[10.0, 10.5, -3.5]).unwrap();
        let hdu = primary(&bytes).unwrap();
        assert_eq!(hdu.bscale(), 0.5);
        assert_eq!(hdu.values().collect::<Vec<_>>(), vec![10.0, 10.5, -3.5]);

        let u16s = [0u16, 1, 40000, 65535];
        let bytes = write_primary(&[], Pixels::U16(&u16s), &[4]).unwrap();
        let hdu = primary(&bytes).unwrap();
        assert_eq!(hdu.bzero(), 32768.0);
        assert_eq!(hdu.to_native().unwrap(), ImageData::U16(u16s.to_vec()));

        let i8s = [-128i8, -1, 0, 127];
        let bytes = write_primary(&[], Pixels::I8(&i8s), &[4]).unwrap();
        let native = primary(&bytes).unwrap().to_native().unwrap();
        assert_eq!(native, ImageData::I8(i8s.to_vec()));

        let u64s = [0u64, 1 << 63, u64::MAX];
        let bytes = write_primary(&[], Pixels::U64(&u64s), &[3]).unwrap();
        let native = primary(&bytes).unwrap().to_native().unwrap();
        assert_eq!(native, ImageData::U64(u64s.to_vec()));
    }

    #[test]
    fn writer_header_and_format() {
        let header = [
            WriteCard {
                keyword: "OBJECT",
                value: Some(Value::Text("O'Neil")),
                comment: "target",
            },
            WriteCard {
                keyword: "EXPTIME",
                value: Some(Value::Float(1.5e-3)),
                comment: "seconds",
            },
            WriteCard {
                keyword: "CRVAL1",
                value: Some(Value::Float(-150.25)),
                comment: "",
            },
            WriteCard {
                keyword: "HISTORY",
                value: None,
                comment: "written by tpt-planetary",
            },
        ];
        let bytes = write_primary_f64(&header, Bitpix::F64, Scaling::NONE, &[1], &[7.0]).unwrap();
        assert_eq!(bytes.len(), 2 * BLOCK);
        let hdu = primary(&bytes).unwrap();
        // Text is returned as written, with `''` escapes left as two quotes.
        assert_eq!(hdu.header().get("OBJECT"), Some(Value::Text("O''Neil")));
        assert_eq!(hdu.header().get("EXPTIME"), Some(Value::Float(1.5e-3)));
        assert_eq!(hdu.header().get("CRVAL1"), Some(Value::Float(-150.25)));
        assert_eq!(hdu.header().card("EXPTIME").unwrap().comment, "seconds");
        let text = hdu.header().as_text();
        assert_eq!(text.len() % CARD, 0);
        assert!(text.is_ascii());
        assert!(text.starts_with("SIMPLE  =                    T"));
    }

    #[test]
    fn writer_rejects_bad_input() {
        let f64s = [1.5];
        assert_eq!(
            write_primary_f64(&[], Bitpix::I16, Scaling::NONE, &[1], &f64s),
            Err(FitsError::NotRepresentable)
        );
        assert_eq!(
            write_primary_f64(&[], Bitpix::I16, Scaling::NONE, &[1], &[40000.0]),
            Err(FitsError::NotRepresentable)
        );
        assert_eq!(
            write_primary_f64(&[], Bitpix::F64, Scaling::NONE, &[2], &[1.0]),
            Err(FitsError::ShapeMismatch)
        );
        assert_eq!(
            write_primary_f64(
                &[],
                Bitpix::F32,
                Scaling {
                    bscale: 2.0,
                    bzero: 0.0
                },
                &[1],
                &[1.0]
            ),
            Err(FitsError::Unsupported(
                "scaling applies only to integer BITPIX"
            ))
        );
        let reserved = [WriteCard {
            keyword: "NAXIS3",
            value: Some(Value::Integer(1)),
            comment: "",
        }];
        assert_eq!(
            write_primary_f64(&reserved, Bitpix::U8, Scaling::NONE, &[1], &[1.0]),
            Err(FitsError::InvalidKeyword)
        );
        let bad_key = [WriteCard {
            keyword: "lower",
            value: Some(Value::Integer(1)),
            comment: "",
        }];
        assert_eq!(
            write_primary_f64(&bad_key, Bitpix::U8, Scaling::NONE, &[1], &[1.0]),
            Err(FitsError::InvalidKeyword)
        );
        let long = "x".repeat(100);
        let too_long = [WriteCard {
            keyword: "LONG",
            value: Some(Value::Text(&long)),
            comment: "",
        }];
        assert_eq!(
            write_primary_f64(&too_long, Bitpix::U8, Scaling::NONE, &[1], &[1.0]),
            Err(FitsError::InvalidCardText)
        );
        assert_eq!(
            write_primary_f64(&[], Bitpix::F64, Scaling::NONE, &[], &[]).map(|b| b.len()),
            Ok(BLOCK)
        );
    }
}
