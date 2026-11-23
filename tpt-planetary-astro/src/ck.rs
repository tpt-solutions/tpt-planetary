//! C-kernel (CK) attitude: reading of DAF/CK files with data type 2, the
//! piecewise-constant angular velocity segments that SPICE's `ckgpav` evaluates.
//!
//! A type 2 segment is a set of disjoint intervals of spacecraft clock (SCLK)
//! time. Each interval has a pointing record of eight doubles: the quaternion
//! `q0..q3` of the C-matrix at the interval start, the angular velocity `a1..a3`
//! in rad/s with respect to the segment's base frame, and a rate that converts
//! SCLK ticks to seconds. The record is followed by the interval START times,
//! the STOP times, and a directory that is not needed here.
//!
//! The C-matrix at time `t` is `C(t) = C0 * R(a, -theta)`, where `C0` is the
//! matrix of the start quaternion, `a` the unit axis of the angular velocity and
//! `theta = |a| (t - start) rate` (verified against `ckgpav` in
//! `tests/ck_attitude.rs`). Only type 2 is read; other types are skipped.
//!
//! The reader works on byte slices and needs no allocation, so it builds for
//! `no_std` targets. Evaluating the attitude needs the trigonometric functions
//! and so `std`.

use crate::ephemeris::{Endian, EphemerisError, read_f64, read_i32, read_u32};

type Result<T> = core::result::Result<T, EphemerisError>;

const RECORD_BYTES: usize = 1024;
const SUMMARY_RECORD_HEADER: usize = 24;
/// Summary size for ND = 2 and NI = 6: two doubles and six integers.
const CK_SUMMARY_BYTES: usize = 40;
const MAX_SUMMARIES_PER_RECORD: usize = (RECORD_BYTES - SUMMARY_RECORD_HEADER) / CK_SUMMARY_BYTES;
#[cfg(feature = "std")]
/// Doubles per type 2 pointing record.
const RECORD_DOUBLES: usize = 8;
/// Data type of the segments this module reads.
pub const DATA_TYPE_2: i32 = 2;

/// One segment summary of a CK file.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CkSegment {
    /// NAIF instrument (or spacecraft) id.
    pub instrument: i32,
    /// NAIF reference frame id (1 = J2000).
    pub frame: i32,
    /// CK data type.
    pub data_type: i32,
    /// First SCLK time covered, in encoded ticks.
    pub start: f64,
    /// Last SCLK time covered, in encoded ticks.
    pub stop: f64,
    /// First double-precision word of the segment data (1-based).
    pub begin_word: u32,
    /// Last double-precision word of the segment data (1-based, inclusive).
    pub end_word: u32,
}

/// The pointing of an instrument at one time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Attitude {
    /// The C-matrix, row-major: it maps base-frame vectors to instrument-frame vectors.
    pub matrix: [[f64; 3]; 3],
    /// Angular velocity in rad/s, with respect to the base frame.
    pub angular_velocity: [f64; 3],
    /// Reference frame id of the segment.
    pub frame: i32,
}

/// A CK file held as a byte slice.
#[derive(Debug, Clone, Copy)]
pub struct CkFile<'a> {
    bytes: &'a [u8],
    endian: Endian,
    first_summary_record: u32,
}

impl<'a> CkFile<'a> {
    /// Parses the file record of a CK file held in `bytes`.
    ///
    /// # Errors
    ///
    /// [`EphemerisError::TooShort`] for a truncated file,
    /// [`EphemerisError::NotSpk`] if the identifier is not `DAF/CK`,
    /// [`EphemerisError::UnsupportedFormat`] for an unknown byte order, and
    /// [`EphemerisError::Malformed`] unless ND = 2 and NI = 6.
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < RECORD_BYTES {
            return Err(EphemerisError::TooShort);
        }
        if !bytes.starts_with(b"DAF/CK") {
            return Err(EphemerisError::NotSpk);
        }
        let endian = match bytes.get(88..96) {
            Some(b"LTL-IEEE") => Endian::Little,
            Some(b"BIG-IEEE") => Endian::Big,
            _ => return Err(EphemerisError::UnsupportedFormat),
        };
        let nd = read_u32(bytes, 8, endian).ok_or(EphemerisError::TooShort)?;
        let ni = read_u32(bytes, 12, endian).ok_or(EphemerisError::TooShort)?;
        if nd != 2 || ni != 6 {
            return Err(EphemerisError::Malformed("CK requires ND=2 and NI=6"));
        }
        let first_summary_record = read_u32(bytes, 76, endian).ok_or(EphemerisError::TooShort)?;
        if first_summary_record == 0 {
            return Err(EphemerisError::Malformed("no summary record"));
        }
        Ok(Self {
            bytes,
            endian,
            first_summary_record,
        })
    }

    /// Iterates over all segment summaries in file order.
    #[must_use]
    pub fn segments(&self) -> CkSegments<'a> {
        CkSegments {
            file: *self,
            next_record: self.first_summary_record,
            base: 0,
            count: 0,
            index: 0,
            failed: false,
        }
    }

    #[cfg(feature = "std")]
    /// Reads the double-precision word `index` (1-based) of the file's data.
    fn word(&self, index: u32) -> Result<f64> {
        let offset = (index as usize - 1) * 8;
        read_f64(self.bytes, offset, self.endian)
            .ok_or(EphemerisError::Malformed("segment data beyond end of file"))
    }

    /// The attitude of `instrument` at SCLK time `sclk` (encoded ticks), from the
    /// first type 2 segment that covers it.
    ///
    /// An epoch on a boundary between two intervals is taken from the interval
    /// that starts there, as SPICE does. The last stop time is included.
    ///
    /// # Errors
    ///
    /// [`EphemerisError::NotFound`] if no segment is for `instrument`, and
    /// [`EphemerisError::OutOfRange`] if the time is outside the coverage.
    #[cfg(feature = "std")]
    pub fn attitude(&self, instrument: i32, sclk: f64) -> Result<Attitude> {
        let mut coverage: Option<(f64, f64)> = None;
        for seg in self.segments() {
            let seg = seg?;
            if seg.instrument != instrument || seg.data_type != DATA_TYPE_2 {
                continue;
            }
            coverage = Some(match coverage {
                None => (seg.start, seg.stop),
                Some((s, e)) => (s.min(seg.start), e.max(seg.stop)),
            });
            if sclk >= seg.start && sclk <= seg.stop {
                return self.evaluate_type2(&seg, sclk);
            }
        }
        match coverage {
            Some((start, end)) => Err(EphemerisError::OutOfRange {
                et: sclk,
                start,
                end,
            }),
            None => Err(EphemerisError::NotFound { target: instrument }),
        }
    }

    #[cfg(feature = "std")]
    fn evaluate_type2(&self, seg: &CkSegment, sclk: f64) -> Result<Attitude> {
        let total = (seg.end_word as usize)
            .checked_sub(seg.begin_word as usize)
            .map(|d| d + 1)
            .ok_or(EphemerisError::Malformed("segment end before start"))?;
        let count = interval_count(total).ok_or(EphemerisError::Malformed(
            "type 2 segment length is not 10 words per interval",
        ))?;
        let begin = seg.begin_word;
        let starts_at = begin + (RECORD_DOUBLES * count) as u32;
        let stops_at = starts_at + count as u32;

        // The interval whose START is at or before sclk and whose STOP is after it.
        let mut chosen = None;
        for i in 0..count {
            let start = self.word(starts_at + i as u32)?;
            let stop = self.word(stops_at + i as u32)?;
            if start <= sclk && sclk < stop {
                chosen = Some(i);
                break;
            }
        }
        // The very last instant belongs to the final interval.
        if chosen.is_none() && count > 0 && sclk == self.word(stops_at + count as u32 - 1)? {
            chosen = Some(count - 1);
        }
        let i = chosen.ok_or(EphemerisError::OutOfRange {
            et: sclk,
            start: seg.start,
            end: seg.stop,
        })?;

        let record = begin + (RECORD_DOUBLES * i) as u32;
        let mut r = [0.0f64; RECORD_DOUBLES];
        for (k, slot) in r.iter_mut().enumerate() {
            *slot = self.word(record + k as u32)?;
        }
        let interval_start = self.word(starts_at + i as u32)?;
        let q = [r[0], r[1], r[2], r[3]];
        let w = [r[4], r[5], r[6]];
        let rate = r[7];

        let c0 = quaternion_matrix(q);
        let speed = norm3(w);
        let matrix = if speed == 0.0 {
            c0
        } else {
            let theta = speed * (sclk - interval_start) * rate;
            let axis = [w[0] / speed, w[1] / speed, w[2] / speed];
            mat_mul(c0, rodrigues(axis, -theta))
        };
        Ok(Attitude {
            matrix,
            angular_velocity: w,
            frame: seg.frame,
        })
    }
}

#[cfg(feature = "std")]
/// Number of intervals in a type 2 segment of `total` doubles: ten per interval
/// (eight of record, START and STOP), plus one directory entry per 100 intervals
/// after the first 100.
fn interval_count(total: usize) -> Option<usize> {
    let guess = total / 10;
    (guess.saturating_sub(2)..=guess + 1)
        .rev()
        .find(|&n| n >= 1 && 10 * n + (n - 1) / 100 == total)
}

/// Iterator over the segment summaries of a CK file.
pub struct CkSegments<'a> {
    file: CkFile<'a>,
    next_record: u32,
    base: usize,
    count: usize,
    index: usize,
    failed: bool,
}

impl Iterator for CkSegments<'_> {
    type Item = Result<CkSegment>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if self.index < self.count {
                let offset = self.base + SUMMARY_RECORD_HEADER + self.index * CK_SUMMARY_BYTES;
                self.index += 1;
                let result = read_summary(self.file.bytes, offset, self.file.endian);
                if result.is_err() {
                    self.failed = true;
                }
                return Some(result);
            }
            if self.next_record == 0 {
                return None;
            }
            // A summary record starts with NEXT, PREV and NSUM as three doubles.
            let base = (self.next_record as usize - 1) * RECORD_BYTES;
            let header = (|| {
                let next = read_f64(self.file.bytes, base, self.file.endian)?;
                let nsum = read_f64(self.file.bytes, base + 16, self.file.endian)?;
                Some((next, nsum))
            })();
            let Some((next, nsum)) = header else {
                self.failed = true;
                return Some(Err(EphemerisError::TooShort));
            };
            if !(nsum >= 0.0
                && (nsum as usize) as f64 == nsum
                && nsum as usize <= MAX_SUMMARIES_PER_RECORD)
            {
                self.failed = true;
                return Some(Err(EphemerisError::Malformed("summary count")));
            }
            self.base = base;
            self.count = nsum as usize;
            self.index = 0;
            self.next_record = next.max(0.0) as u32;
        }
    }
}

fn read_summary(bytes: &[u8], offset: usize, endian: Endian) -> Result<CkSegment> {
    let short = EphemerisError::Malformed("summary beyond end of file");
    let start = read_f64(bytes, offset, endian).ok_or_else(|| short.clone())?;
    let stop = read_f64(bytes, offset + 8, endian).ok_or_else(|| short.clone())?;
    let int_at =
        |i: usize| read_i32(bytes, offset + 16 + 4 * i, endian).ok_or_else(|| short.clone());
    Ok(CkSegment {
        instrument: int_at(0)?,
        frame: int_at(1)?,
        data_type: int_at(2)?,
        start,
        stop,
        begin_word: int_at(4)?.max(0) as u32,
        end_word: int_at(5)?.max(0) as u32,
    })
}

#[cfg(feature = "std")]
/// Rotation matrix of a scalar-first quaternion, as SPICE's `q2m` gives it.
fn quaternion_matrix(q: [f64; 4]) -> [[f64; 3]; 3] {
    let [q0, q1, q2, q3] = q;
    [
        [
            1.0 - 2.0 * (q2 * q2 + q3 * q3),
            2.0 * (q1 * q2 - q0 * q3),
            2.0 * (q1 * q3 + q0 * q2),
        ],
        [
            2.0 * (q1 * q2 + q0 * q3),
            1.0 - 2.0 * (q1 * q1 + q3 * q3),
            2.0 * (q2 * q3 - q0 * q1),
        ],
        [
            2.0 * (q1 * q3 - q0 * q2),
            2.0 * (q2 * q3 + q0 * q1),
            1.0 - 2.0 * (q1 * q1 + q2 * q2),
        ],
    ]
}

#[cfg(feature = "std")]
/// Rotation by `angle` radians about the unit vector `axis` (Rodrigues' formula).
fn rodrigues(axis: [f64; 3], angle: f64) -> [[f64; 3]; 3] {
    let [x, y, z] = axis;
    let (s, c) = angle.sin_cos();
    let t = 1.0 - c;
    [
        [t * x * x + c, t * x * y - s * z, t * x * z + s * y],
        [t * x * y + s * z, t * y * y + c, t * y * z - s * x],
        [t * x * z - s * y, t * y * z + s * x, t * z * z + c],
    ]
}

#[cfg(feature = "std")]
fn mat_mul(a: [[f64; 3]; 3], b: [[f64; 3]; 3]) -> [[f64; 3]; 3] {
    let mut out = [[0.0; 3]; 3];
    for (i, row) in out.iter_mut().enumerate() {
        for (j, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[i][k] * b[k][j]).sum();
        }
    }
    out
}

#[cfg(feature = "std")]
fn norm3(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}
