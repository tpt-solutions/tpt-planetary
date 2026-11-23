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
