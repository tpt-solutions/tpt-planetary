//! Decoding of the RICE_1 tile compression of FITS images (CFITSIO's algorithm,
//! as in the FITS tile-compression convention).
//!
//! A tile is a stream of bits. It starts with the first pixel value, `bytepix`
//! bytes big-endian. The rest is a sequence of blocks of `nblock` pixels. Each
//! block starts with an `fs` field: `fsbits` bits, whose value minus one is the
//! Rice parameter of the block.
//!
//! - `fs < 0`: every pixel in the block equals its predecessor.
//! - `fs == fsmax`: each difference is stored raw, in `8 * bytepix` bits.
//! - otherwise each difference is a Rice code: a unary count of zero bits, a
//!   one bit, then `fs` low bits.
//!
//! Differences are mapped to unsigned by zig-zag coding (`0, -1, 1, -2, ...`
//! as `0, 1, 2, 3, ...`). The parameters per byte width are CFITSIO's: 3, 4 or
//! 5 `fs` bits, and `fsmax` 6, 14 or 25.

use alloc::vec::Vec;

use crate::fits::{FitsError, Result};

/// Bit reader, most significant bit first.
struct Bits<'a> {
    data: &'a [u8],
    pos: usize,
    acc: u64,
    nacc: u32,
}

impl<'a> Bits<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            acc: 0,
            nacc: 0,
        }
    }

    fn fill(&mut self, need: u32) -> Result<()> {
        while self.nacc < need {
            let byte = *self
                .data
                .get(self.pos)
                .ok_or(FitsError::Truncated("RICE tile data ends early"))?;
            self.pos += 1;
            self.acc = (self.acc << 8) | u64::from(byte);
            self.nacc += 8;
        }
        Ok(())
    }

    /// The next `n` bits, `n <= 32`.
    fn bits(&mut self, n: u32) -> Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        self.fill(n)?;
        self.nacc -= n;
        let value = (self.acc >> self.nacc) & ((1u64 << n) - 1);
        self.acc &= (1u64 << self.nacc) - 1;
        Ok(value as u32)
    }

    /// The number of zero bits before the next one bit; the one bit is consumed.
    fn zeros_before_one(&mut self) -> Result<u32> {
        let mut zeros = 0u32;
        while self.bits(1)? == 0 {
            zeros += 1;
            if zeros > 1 << 20 {
                return Err(FitsError::InvalidData("RICE code has no end"));
            }
        }
        Ok(zeros)
    }
}

/// Maps a zig-zag coded difference back to a signed value (modulo 2^32).
fn unzigzag(m: u32) -> u32 {
    if m & 1 == 0 { m >> 1 } else { !(m >> 1) }
}

/// Decodes one RICE_1 tile of `out.len()` pixels into `out` (modulo 2^32).
///
/// # Errors
///
/// [`FitsError::Unsupported`] for a byte width other than 1, 2 or 4,
/// [`FitsError::Truncated`] if the stream ends before the tile is full.
pub fn rice_decode(data: &[u8], bytepix: usize, nblock: usize, out: &mut [u32]) -> Result<()> {
    let (fsbits, fsmax, bbits) = match bytepix {
        1 => (3, 6, 8),
        2 => (4, 14, 16),
        4 => (5, 25, 32),
        _ => {
            return Err(FitsError::Unsupported(
                "RICE_1 with a byte width other than 1, 2 or 4",
            ));
        }
    };
    if nblock == 0 {
        return Err(FitsError::InvalidHeader("RICE_1 BLOCKSIZE is zero"));
    }
    let first = data
        .get(..bytepix)
        .ok_or(FitsError::Truncated("RICE tile has no first value"))?;
    let mut last = first.iter().fold(0u32, |acc, &b| (acc << 8) | u32::from(b));
    let mut bits = Bits::new(&data[bytepix..]);

    let n = out.len();
    let mut i = 0;
    while i < n {
        let fs = i64::from(bits.bits(fsbits)?) - 1;
        let end = (i + nblock).min(n);
        if fs < 0 {
            for slot in &mut out[i..end] {
                *slot = last;
            }
        } else if fs == i64::from(fsmax) {
            for slot in &mut out[i..end] {
                let raw = bits.bits(bbits)?;
                last = last.wrapping_add(unzigzag(raw));
                *slot = last;
            }
        } else {
            let fs = fs as u32;
            for slot in &mut out[i..end] {
                let zeros = bits.zeros_before_one()?;
                let low = bits.bits(fs)?;
                let m = (zeros << fs) | low;
                last = last.wrapping_add(unzigzag(m));
                *slot = last;
            }
        }
        i = end;
    }
    Ok(())
}

/// The payload of a gzip member (RFC 1952): the deflate stream after the header.
fn gzip_payload(data: &[u8]) -> Result<&[u8]> {
    let truncated = FitsError::Truncated("gzip header is cut short");
    if data.len() < 10 {
        return Err(truncated);
    }
    let flags = data[3];
    let mut pos = 10;
    if flags & 0x04 != 0 {
        // FEXTRA: a 16-bit length, then that many bytes.
        let len = usize::from(*data.get(pos).ok_or(truncated)?)
            | usize::from(*data.get(pos + 1).ok_or(truncated)?) << 8;
        pos += 2 + len;
    }
    for flag in [0x08, 0x10] {
        // FNAME and FCOMMENT: zero-terminated strings.
        if flags & flag != 0 {
            let end = data
                .get(pos..)
                .and_then(|rest| rest.iter().position(|&b| b == 0))
                .ok_or(truncated)?;
            pos += end + 1;
        }
    }
    if flags & 0x02 != 0 {
        // FHCRC: two bytes of header checksum.
        pos += 2;
    }
    data.get(pos..).ok_or(truncated)
}

/// Inflates one compressed tile. The framing is detected from the first bytes:
/// gzip (`1f 8b`), zlib (a `0x78` header), or a raw deflate stream.
///
/// # Errors
///
/// [`FitsError::InvalidData`] if the deflate stream is malformed.
pub fn inflate_tile(data: &[u8]) -> Result<Vec<u8>> {
    let bad = |_| FitsError::InvalidData("tile deflate stream is malformed");
    if data.starts_with(&[0x1f, 0x8b]) {
        miniz_oxide::inflate::decompress_to_vec(gzip_payload(data)?).map_err(bad)
    } else if data.first() == Some(&0x78) {
        miniz_oxide::inflate::decompress_to_vec_zlib(data).map_err(bad)
    } else {
        miniz_oxide::inflate::decompress_to_vec(data).map_err(bad)
    }
}

/// Decodes one GZIP_1 or GZIP_2 tile of `out.len()` pixels of `bytepix` bytes.
///
/// GZIP_1 inflates to the big-endian pixel bytes. GZIP_2 inflates to byte
/// planes: all the most significant bytes first, then the next plane, and so
/// on, each of `out.len()` bytes.
///
/// # Errors
///
/// [`FitsError::InvalidData`] for a malformed stream, [`FitsError::Truncated`]
/// if it holds fewer bytes than the tile needs.
pub fn gzip_decode(data: &[u8], bytepix: usize, shuffled: bool, out: &mut [u32]) -> Result<()> {
    let bytes = inflate_tile(data)?;
    let n = out.len();
    if bytes.len() < n * bytepix {
        return Err(FitsError::Truncated("gzip tile holds too few bytes"));
    }
    for (i, slot) in out.iter_mut().enumerate() {
        let mut value = 0u32;
        for p in 0..bytepix {
            let byte = if shuffled {
                bytes[p * n + i]
            } else {
                bytes[i * bytepix + p]
            };
            value = (value << 8) | u32::from(byte);
        }
        *slot = value;
    }
    Ok(())
}

#[cfg(feature = "std")]
/// Writes bits, most significant first.
struct BitWriter {
    out: Vec<u8>,
    acc: u64,
    nacc: u32,
}

#[cfg(feature = "std")]
impl BitWriter {
    fn new() -> Self {
        Self {
            out: Vec::new(),
            acc: 0,
            nacc: 0,
        }
    }

    /// The low `n` bits of `value`, `n <= 32`.
    fn bits(&mut self, value: u32, n: u32) {
        if n == 0 {
            return;
        }
        self.acc = (self.acc << n) | (u64::from(value) & ((1u64 << n) - 1));
        self.nacc += n;
        while self.nacc >= 8 {
            self.nacc -= 8;
            self.out.push((self.acc >> self.nacc) as u8);
        }
        self.acc &= (1u64 << self.nacc) - 1;
    }

    /// `count` zero bits followed by a one bit.
    fn unary(&mut self, count: u64) {
        let mut left = count;
        while left >= 32 {
            self.bits(0, 32);
            left -= 32;
        }
        self.bits(0, left as u32);
        self.bits(1, 1);
    }

    fn finish(mut self) -> Vec<u8> {
        if self.nacc > 0 {
            let pad = 8 - self.nacc;
            self.bits(0, pad);
        }
        self.out
    }
}

#[cfg(feature = "std")]
/// Maps a signed difference (in `bytepix` bytes of two's complement) to the
/// unsigned zig-zag code that the decoder inverts.
fn zigzag(d: i64) -> u32 {
    // d fits in 32 bits for every supported byte width.
    let d = d as i32;
    ((d << 1) ^ (d >> 31)) as u32
}

#[cfg(feature = "std")]
/// Encodes `values` (the pixel bit patterns in their `bytepix` width) as a RICE_1
/// stream, the inverse of [`rice_decode`]. The first value is stored in full, and
/// each block of `nblock` differences uses the Rice parameter that codes it in
/// the fewest bits, or raw bits when that is shorter.
pub fn rice_encode(values: &[u32], bytepix: usize, nblock: usize) -> Result<Vec<u8>> {
    let (fsbits, fsmax, bbits) = match bytepix {
        1 => (3u32, 6u32, 8u32),
        2 => (4, 14, 16),
        4 => (5, 25, 32),
        _ => {
            return Err(FitsError::Unsupported(
                "RICE_1 with a byte width other than 1, 2 or 4",
            ));
        }
    };
    if nblock == 0 {
        return Err(FitsError::InvalidHeader("RICE_1 BLOCKSIZE is zero"));
    }
    let mask = if bytepix == 4 {
        u32::MAX
    } else {
        (1u32 << (8 * bytepix)) - 1
    };
    let mut out = Vec::new();
    let first = values.first().copied().unwrap_or(0);
    for k in (0..bytepix).rev() {
        out.push((first >> (8 * k)) as u8);
    }
    let mut bits = BitWriter::new();
    let mut last = first & mask;
    for block in values.chunks(nblock) {
        // Differences in the pixel width, zig-zag coded.
        let mut codes = Vec::with_capacity(block.len());
        for &x in block {
            let x = x & mask;
            // Sign-extend the difference from the pixel width.
            let shift = 32 - 8 * bytepix as u32;
            let d = (((x.wrapping_sub(last)) << shift) as i32) >> shift;
            codes.push(zigzag(i64::from(d)));
            last = x;
        }
        if codes.iter().all(|&m| m == 0) {
            bits.bits(0, fsbits);
            continue;
        }
        // The Rice parameter with the fewest bits, from 0 to fsmax - 1.
        let (best_k, best_cost) = (0..fsmax)
            .map(|k| {
                let cost: u64 = codes
                    .iter()
                    .map(|&m| u64::from(m >> k) + 1 + u64::from(k))
                    .sum();
                (k, cost)
            })
            .min_by_key(|&(_, cost)| cost)
            .expect("at least one parameter");
        let raw_cost = u64::from(bbits) * codes.len() as u64;
        if best_cost >= raw_cost {
            bits.bits(fsmax + 1, fsbits);
            for &m in &codes {
                bits.bits(m, bbits);
            }
        } else {
            bits.bits(best_k + 1, fsbits);
            for &m in &codes {
                bits.unary(u64::from(m >> best_k));
                bits.bits(m, best_k);
            }
        }
    }
    out.extend(bits.finish());
    Ok(out)
}

#[cfg(feature = "std")]
/// CRC-32 (IEEE, as used by gzip), bit by bit.
fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xFFFF_FFFFu32;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(feature = "std")]
/// Wraps `data` in a gzip member (RFC 1952) with a deflate body.
pub fn gzip_encode(data: &[u8]) -> Vec<u8> {
    let mut out = vec![0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 6));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

#[cfg(feature = "std")]
/// Encodes one GZIP_1 or GZIP_2 tile of pixel bit patterns of `bytepix` bytes:
/// the big-endian bytes, or for GZIP_2 the byte planes, most significant first.
pub fn gzip_tile_encode(values: &[u32], bytepix: usize, shuffled: bool) -> Vec<u8> {
    let n = values.len();
    let mut bytes = vec![0u8; n * bytepix];
    for (i, &v) in values.iter().enumerate() {
        for p in 0..bytepix {
            let byte = (v >> (8 * (bytepix - 1 - p))) as u8;
            if shuffled {
                bytes[p * n + i] = byte;
            } else {
                bytes[i * bytepix + p] = byte;
            }
        }
    }
    gzip_encode(&bytes)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    /// Deterministic pseudo-random words (xorshift), for test data.
    fn words(seed: u32, n: usize, mask: u32) -> Vec<u32> {
        let mut x = seed.max(1);
        (0..n)
            .map(|_| {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                x & mask
            })
            .collect()
    }

    fn rice_roundtrip(values: &[u32], bytepix: usize) {
        let stream = rice_encode(values, bytepix, 32).unwrap();
        let mut out = vec![0u32; values.len()];
        rice_decode(&stream, bytepix, 32, &mut out).unwrap();
        let mask = if bytepix == 4 {
            u32::MAX
        } else {
            (1 << (8 * bytepix)) - 1
        };
        let want: Vec<u32> = values.iter().map(|v| v & mask).collect();
        let got: Vec<u32> = out.iter().map(|v| v & mask).collect();
        assert_eq!(got, want, "bytepix {bytepix}");
    }

    #[test]
    fn rice_round_trips_smooth_noisy_constant_and_extreme_data() {
        let smooth: Vec<u32> = (0..200u32).map(|i| (i * 3 + (i % 7)) & 0xFF).collect();
        let constant = vec![42u32; 100];
        for bytepix in [1usize, 2, 4] {
            rice_roundtrip(&smooth, bytepix);
            rice_roundtrip(&constant, bytepix);
            rice_roundtrip(&words(9, 333, u32::MAX), bytepix);
            rice_roundtrip(&words(3, 97, 0xFF), bytepix);
        }
        // Extremes of the 16- and 32-bit ranges, alternating.
        let extremes16: Vec<u32> = (0..64)
            .map(|i| if i % 2 == 0 { 0 } else { 0xFFFF })
            .collect();
        rice_roundtrip(&extremes16, 2);
        let extremes32: Vec<u32> = (0..64)
            .map(|i| if i % 2 == 0 { 0 } else { u32::MAX })
            .collect();
        rice_roundtrip(&extremes32, 4);
    }

    #[test]
    fn gzip_tiles_round_trip_in_both_layouts() {
        let values = words(5, 40, 0xFFFF);
        for shuffled in [false, true] {
            let stream = gzip_tile_encode(&values, 2, shuffled);
            let mut out = vec![0u32; values.len()];
            gzip_decode(&stream, 2, shuffled, &mut out).unwrap();
            assert_eq!(out, values, "shuffled {shuffled}");
        }
    }

    #[test]
    fn crc32_matches_the_check_value() {
        // The standard check value of CRC-32/IEEE.
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
    }
}

/// Length of the dither sequence (CFITSIO's `N_RANDOM`).
pub const N_RANDOM: usize = 10_000;

/// The dither sequence of CFITSIO: `N_RANDOM` uniform values in (0, 1), from the
/// Park–Miller minimal standard generator (a = 16807, m = 2^31 - 1, seed 1),
/// stored as float32 as CFITSIO does. The 10000th seed is 1043618065.
pub fn dither_sequence() -> Vec<f32> {
    let a = 16807.0f64;
    let m = 2_147_483_647.0f64;
    let mut seed = 1.0f64;
    (0..N_RANDOM)
        .map(|_| {
            let temp = a * seed;
            seed = temp - m * ((temp / m) as i64 as f64);
            (seed / m) as f32
        })
        .collect()
}

/// Unquantizes one dithered float tile in place of its pixels.
///
/// `q` holds the stored integers of the tile in pixel order, `row` is the tile's
/// dither row (its 1-based table row plus `ZDITHER0`, minus 1), and `scale` and
/// `zero` are the tile's `ZSCALE` and `ZZERO`. Each pixel is
/// `(q - r + 0.5) * scale + zero` with `r` the next value of the sequence, as
/// CFITSIO computes it, rounded to float32. Undefined pixels (`q == blank`) are
/// NaN. With `zero_preserving` (SUBTRACTIVE_DITHER_2), pixels stored as the zero
/// code are exactly 0.0. Every pixel advances the sequence, undefined or not.
/// The parameters of one dithered float tile.
#[derive(Debug, Clone, Copy)]
pub struct Dither<'a> {
    /// The tile's dither row: its 1-based table row plus `ZDITHER0`, minus 1.
    pub row: usize,
    /// `ZSCALE` of the tile.
    pub scale: f64,
    /// `ZZERO` of the tile.
    pub zero: f64,
    /// The stored value of an undefined pixel.
    pub blank: i32,
    /// SUBTRACTIVE_DITHER_2: pixels stored as the zero code are exactly 0.0.
    pub zero_preserving: bool,
    /// The dither sequence, from [`dither_sequence`].
    pub seq: &'a [f32],
}

pub fn unquantize_dithered(q: &[i32], params: &Dither<'_>, out: &mut [f64]) {
    let Dither {
        row,
        scale,
        zero,
        blank,
        zero_preserving,
        seq,
    } = *params;
    // The start: iseed = (row - 1) mod N, nextrand = int(500 * seq[iseed]).
    let mut iseed = (row - 1) % N_RANDOM;
    let mut nextrand = (f64::from(seq[iseed]) * 500.0) as usize;
    for (value, &stored) in out.iter_mut().zip(q) {
        let r = f64::from(seq[nextrand]);
        *value = if stored == blank {
            f64::NAN
        } else if zero_preserving && stored == ZERO_CODE {
            0.0
        } else {
            f64::from(((f64::from(stored) - r + 0.5) * scale + zero) as f32)
        };
        nextrand += 1;
        if nextrand == N_RANDOM {
            iseed += 1;
            if iseed == N_RANDOM {
                iseed = 0;
            }
            nextrand = (f64::from(seq[iseed]) * 500.0) as usize;
        }
    }
}

/// The integer stored for a zero pixel by SUBTRACTIVE_DITHER_2 (CFITSIO's `ZERO_VALUE`).
pub const ZERO_CODE: i32 = -2_147_483_646;

#[cfg(all(test, feature = "std"))]
mod dither_tests {
    use super::*;

    #[test]
    fn the_dither_sequence_matches_cfitsio() {
        let seq = dither_sequence();
        assert_eq!(seq.len(), N_RANDOM);
        // The 10000th seed is 1043618065; the stored value is that over m, in float32.
        let want = (1_043_618_065.0f64 / 2_147_483_647.0) as f32;
        assert_eq!(seq[N_RANDOM - 1], want);
        assert!(seq.iter().all(|&v| v > 0.0 && v < 1.0));
    }
}
