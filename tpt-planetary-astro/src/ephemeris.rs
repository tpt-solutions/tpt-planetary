//! Pure Rust reader for NAIF DAF/SPK ephemeris files (JPL DE440-style).
//!
//! The parser works on a byte slice, so it needs no allocation and no file
//! I/O. With the `std` feature, [`read_file`] loads a file into memory. The
//! crate does not memory-map files because that requires `unsafe`.
//!
//! Supported:
//! - DAF/SPK file records and summary-record chains, little- and big-endian
//! - SPK data types 2 (Chebyshev position; velocity by differentiation) and
//!   3 (Chebyshev position and velocity)
//! - Segment lookup by target body and epoch
//! - Body-to-barycentre chaining through the segment tree (for example
//!   Earth (399) -> EMB (3) -> SSB (0))
//!
//! Epochs are seconds past J2000 in TDB (`et`). Positions are km and
//! velocities are km/s, as stored in SPK files.
//!
//! Validation status: the parser and evaluator are tested only against
//! synthetic DAF/SPK files built in the test module, which encode known
//! Chebyshev polynomials. They have not been run against a real DE440 file in
//! this crate's tests, so reading real DE440 data is unverified.

use core::fmt;

/// Speed of light in km/s (exact by definition).
#[cfg(feature = "std")]
pub const SPEED_OF_LIGHT_KM_S: f64 = 299_792.458;

/// One astronomical unit in km (IAU 2012 exact value).
#[cfg(feature = "std")]
pub const AU_KM: f64 = 149_597_870.700;

/// Julian date of J2000.0 (TDB/TT).
const J2000_JD: f64 = 2_451_545.0;

/// Seconds per day.
const SECONDS_PER_DAY: f64 = 86_400.0;

/// Size of a DAF record in bytes.
const RECORD_BYTES: usize = 1024;

/// Bytes per DAF double-precision word.
const WORD_BYTES: usize = 8;

/// Byte offset of the first summary inside a summary record.
const SUMMARY_RECORD_HEADER: usize = 24;

/// Bytes per SPK summary (two doubles and six 4-byte integers).
const SPK_SUMMARY_BYTES: usize = 40;

/// Maximum summaries that fit in one summary record for SPK.
const MAX_SUMMARIES_PER_RECORD: usize = (RECORD_BYTES - SUMMARY_RECORD_HEADER) / SPK_SUMMARY_BYTES;

/// Maximum depth of the body-to-barycentre chain before giving up.
const MAX_CHAIN_DEPTH: usize = 16;

/// Common NAIF body identifiers.
pub mod naif {
    /// Solar System Barycentre.
    pub const SSB: i32 = 0;
    /// Mercury barycentre.
    pub const MERCURY_BARYCENTRE: i32 = 1;
    /// Venus barycentre.
    pub const VENUS_BARYCENTRE: i32 = 2;
    /// Earth-Moon barycentre.
    pub const EARTH_MOON_BARYCENTRE: i32 = 3;
    /// Mars barycentre.
    pub const MARS_BARYCENTRE: i32 = 4;
    /// Jupiter barycentre.
    pub const JUPITER_BARYCENTRE: i32 = 5;
    /// Saturn barycentre.
    pub const SATURN_BARYCENTRE: i32 = 6;
    /// Uranus barycentre.
    pub const URANUS_BARYCENTRE: i32 = 7;
    /// Neptune barycentre.
    pub const NEPTUNE_BARYCENTRE: i32 = 8;
    /// Pluto barycentre.
    pub const PLUTO_BARYCENTRE: i32 = 9;
    /// Sun.
    pub const SUN: i32 = 10;
    /// Moon.
    pub const MOON: i32 = 301;
    /// Earth (geocentre).
    pub const EARTH: i32 = 399;
    /// Jupiter (planet centre).
    pub const JUPITER: i32 = 599;
}

/// Errors returned by the DAF/SPK reader.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum EphemerisError {
    /// The byte slice is shorter than a DAF file record or a referenced range.
    TooShort,
    /// The file does not start with the `DAF/SPK` identifier.
    NotSpk,
    /// The binary format string is not `LTL-IEEE` or `BIG-IEEE`.
    UnsupportedFormat,
    /// The file is structurally invalid; the message names the problem.
    Malformed(&'static str),
    /// The SPK data type is not implemented (only 2 and 3 are).
    UnsupportedDataType(i32),
    /// The requested epoch is outside the segment's coverage.
    OutOfRange {
        /// Requested epoch in seconds past J2000 TDB.
        et: f64,
        /// Segment start epoch.
        start: f64,
        /// Segment end epoch.
        end: f64,
    },
    /// No segment provides the requested body at the requested epoch.
    NotFound {
        /// Body whose segment could not be found.
        target: i32,
    },
    /// The body chain did not reach the barycentre within the depth limit.
    ChainTooDeep,
    /// Kind of an I/O error while reading a file (requires the `std` feature).
    #[cfg(feature = "std")]
    Io(std::io::ErrorKind),
}

impl fmt::Display for EphemerisError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(f, "ephemeris data is too short"),
            Self::NotSpk => write!(f, "not a DAF/SPK file"),
            Self::UnsupportedFormat => write!(f, "unsupported DAF binary format string"),
            Self::Malformed(what) => write!(f, "malformed SPK file: {what}"),
            Self::UnsupportedDataType(t) => write!(f, "unsupported SPK data type {t}"),
            Self::OutOfRange { et, start, end } => {
                write!(f, "epoch {et} s outside segment coverage [{start}, {end}]")
            }
            Self::NotFound { target } => write!(f, "no segment for body {target} at this epoch"),
            Self::ChainTooDeep => write!(f, "body chain exceeded depth limit"),
            #[cfg(feature = "std")]
            Self::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for EphemerisError {}

#[cfg(feature = "std")]
impl From<std::io::Error> for EphemerisError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.kind())
    }
}

/// Result alias for ephemeris operations.
pub type Result<T> = core::result::Result<T, EphemerisError>;

/// Byte order of the binary data in a DAF file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Endian {
    /// Little-endian (`LTL-IEEE`).
    Little,
    /// Big-endian (`BIG-IEEE`).
    Big,
}

/// Position and velocity of a body relative to a centre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StateVector {
    /// Position in km, Cartesian, in the segment frame (ICRF for DE440).
    pub position: [f64; 3],
    /// Velocity in km/s.
    pub velocity: [f64; 3],
}

impl StateVector {
    /// The zero state.
    pub const ZERO: Self = Self {
        position: [0.0; 3],
        velocity: [0.0; 3],
    };

    /// Component-wise sum of two states.
    #[must_use]
    pub fn plus(self, other: Self) -> Self {
        let mut out = self;
        for i in 0..3 {
            out.position[i] += other.position[i];
            out.velocity[i] += other.velocity[i];
        }
        out
    }

    /// Component-wise difference `self - other`.
    #[must_use]
    pub fn minus(self, other: Self) -> Self {
        let mut out = self;
        for i in 0..3 {
            out.position[i] -= other.position[i];
            out.velocity[i] -= other.velocity[i];
        }
        out
    }
}

/// Metadata for one SPK segment, from its summary record.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Segment {
    /// First epoch covered, seconds past J2000 TDB.
    pub start_et: f64,
    /// Last epoch covered, seconds past J2000 TDB.
    pub end_et: f64,
    /// NAIF id of the body the segment describes.
    pub target: i32,
    /// NAIF id of the centre the state is relative to.
    pub center: i32,
    /// NAIF reference frame id (1 = J2000/ICRF).
    pub frame: i32,
    /// SPK data type (2 or 3 are supported).
    pub data_type: i32,
    /// First double-precision word of the segment data (1-based).
    pub start_word: u32,
    /// Last double-precision word of the segment data (1-based, inclusive).
    pub end_word: u32,
}

/// Converts a two-part Julian date in TDB to `et` (seconds past J2000 TDB).
#[must_use]
pub fn et_from_tdb_jd(jd_hi: f64, jd_lo: f64) -> f64 {
    ((jd_hi - J2000_JD) + jd_lo) * SECONDS_PER_DAY
}

fn read_bytes<const N: usize>(bytes: &[u8], offset: usize) -> Option<[u8; N]> {
    let slice = bytes.get(offset..offset.checked_add(N)?)?;
    slice.try_into().ok()
}

fn read_u32(bytes: &[u8], offset: usize, endian: Endian) -> Option<u32> {
    let a = read_bytes::<4>(bytes, offset)?;
    Some(match endian {
        Endian::Little => u32::from_le_bytes(a),
        Endian::Big => u32::from_be_bytes(a),
    })
}

fn read_i32(bytes: &[u8], offset: usize, endian: Endian) -> Option<i32> {
    let a = read_bytes::<4>(bytes, offset)?;
    Some(match endian {
        Endian::Little => i32::from_le_bytes(a),
        Endian::Big => i32::from_be_bytes(a),
    })
}

fn read_f64(bytes: &[u8], offset: usize, endian: Endian) -> Option<f64> {
    let a = read_bytes::<8>(bytes, offset)?;
    Some(match endian {
        Endian::Little => f64::from_le_bytes(a),
        Endian::Big => f64::from_be_bytes(a),
    })
}

/// Evaluates a Chebyshev series and its derivative at `s` in `[-1, 1]`.
///
/// `coef(k)` returns the coefficient of `T_k`. Returns `(value, d value / ds)`.
fn chebyshev_eval(s: f64, n: usize, coef: impl Fn(usize) -> Result<f64>) -> Result<(f64, f64)> {
    let mut value = coef(0)?;
    let mut deriv = 0.0;
    if n < 2 {
        return Ok((value, deriv));
    }
    let a1 = coef(1)?;
    value += a1 * s;
    deriv += a1;

    // T_{k-2}, T_{k-1} and their s-derivatives.
    let (mut t_prev, mut t_cur) = (1.0, s);
    let (mut d_prev, mut d_cur) = (0.0, 1.0);
    for k in 2..n {
        let t_next = 2.0 * s * t_cur - t_prev;
        let d_next = 2.0 * t_cur + 2.0 * s * d_cur - d_prev;
        let a = coef(k)?;
        value += a * t_next;
        deriv += a * d_next;
        t_prev = t_cur;
        t_cur = t_next;
        d_prev = d_cur;
        d_cur = d_next;
    }
    Ok((value, deriv))
}

/// A DAF/SPK file held as a byte slice.
#[derive(Debug, Clone, Copy)]
pub struct SpkFile<'a> {
    bytes: &'a [u8],
    endian: Endian,
    first_summary_record: u32,
}

impl<'a> SpkFile<'a> {
    /// Parses the DAF file record of an SPK file held in `bytes`.
    ///
    /// Only the file record is validated here. Segment data is checked when it
    /// is read.
    ///
    /// # Errors
    ///
    /// Returns an error if the slice is too short, the identifier is not
    /// `DAF/SPK`, the format string is unknown, or ND/NI are not 2 and 6.
    pub fn new(bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < RECORD_BYTES {
            return Err(EphemerisError::TooShort);
        }
        if !bytes.starts_with(b"DAF/SPK") {
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
            return Err(EphemerisError::Malformed("SPK requires ND=2 and NI=6"));
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

    /// Byte order detected from the file record.
    #[must_use]
    pub fn endian(&self) -> Endian {
        self.endian
    }

    /// Iterates over all segment summaries in file order.
    #[must_use]
    pub fn segments(&self) -> Segments<'a> {
        Segments {
            file: *self,
            next_record: self.first_summary_record,
            base: 0,
            count: 0,
            index: 0,
            failed: false,
        }
    }

    /// Finds the first segment for `target` whose coverage includes `et`.
    ///
    /// # Errors
    ///
    /// Returns [`EphemerisError::NotFound`] if no segment for `target` exists,
    /// or [`EphemerisError::OutOfRange`] if segments exist but none covers `et`.
    pub fn find_segment(&self, target: i32, et: f64) -> Result<Segment> {
        let mut seen_target = false;
        for seg in self.segments() {
            let seg = seg?;
            if seg.target != target {
                continue;
            }
            seen_target = true;
            if et >= seg.start_et && et <= seg.end_et {
                return Ok(seg);
            }
        }
        if seen_target {
            // Report the coverage of the first matching segment for diagnostics.
            let first = self
                .segments()
                .find_map(|s| s.ok().filter(|s| s.target == target));
            let (start, end) = first.map_or((0.0, 0.0), |s| (s.start_et, s.end_et));
            Err(EphemerisError::OutOfRange { et, start, end })
        } else {
            Err(EphemerisError::NotFound { target })
        }
    }

    /// State of `target` relative to `center` at epoch `et` (TDB seconds past J2000).
    ///
    /// Each body is resolved to the barycentre by walking the segment tree,
    /// and the two barycentric states are subtracted.
    ///
    /// # Errors
    ///
    /// Propagates segment lookup and evaluation errors.
    pub fn state(&self, target: i32, center: i32, et: f64) -> Result<StateVector> {
        if target == center {
            return Ok(StateVector::ZERO);
        }
        let t = self.state_to_ssb(target, et)?;
        let c = self.state_to_ssb(center, et)?;
        Ok(t.minus(c))
    }

    fn state_to_ssb(&self, body: i32, et: f64) -> Result<StateVector> {
        let mut acc = StateVector::ZERO;
        let mut id = body;
        for _ in 0..MAX_CHAIN_DEPTH {
            if id == naif::SSB {
                return Ok(acc);
            }
            let seg = self.find_segment(id, et)?;
            acc = acc.plus(self.evaluate(&seg, et)?);
            id = seg.center;
        }
        Err(EphemerisError::ChainTooDeep)
    }

    fn word(&self, w: usize) -> Result<f64> {
        if w == 0 {
            return Err(EphemerisError::Malformed("word index zero"));
        }
        read_f64(self.bytes, (w - 1) * WORD_BYTES, self.endian)
            .ok_or(EphemerisError::Malformed("segment data beyond end of file"))
    }

    /// Evaluates the Chebyshev representation of one segment at epoch `et`.
    ///
    /// # Errors
    ///
    /// Returns an error if `et` is outside the segment, the data type is not 2
    /// or 3, or the segment layout is inconsistent with the file.
    pub fn evaluate(&self, seg: &Segment, et: f64) -> Result<StateVector> {
        if !(et >= seg.start_et && et <= seg.end_et) {
            return Err(EphemerisError::OutOfRange {
                et,
                start: seg.start_et,
                end: seg.end_et,
            });
        }
        if seg.data_type != 2 && seg.data_type != 3 {
            return Err(EphemerisError::UnsupportedDataType(seg.data_type));
        }
        let start = seg.start_word as usize;
        let end = seg.end_word as usize;
        if start == 0 || end < start + 4 {
            return Err(EphemerisError::Malformed("segment word range"));
        }
        let init = self.word(end - 3)?;
        let intlen = self.word(end - 2)?;
        let rsize = self.word(end - 1)?;
        let n_records = self.word(end)?;
        if intlen.is_nan() || intlen <= 0.0 || n_records < 1.0 || rsize < 5.0 {
            return Err(EphemerisError::Malformed("segment directory"));
        }
        let rsize = rsize as usize;
        let n_records = n_records as usize;

        let frac = (et - init) / intlen;
        if frac < 0.0 {
            return Err(EphemerisError::OutOfRange {
                et,
                start: seg.start_et,
                end: seg.end_et,
            });
        }
        let index = (frac as usize).min(n_records - 1);
        let rec = start + index * rsize;
        if rec + rsize - 1 > end - 4 {
            return Err(EphemerisError::Malformed("record beyond segment data"));
        }
        let mid = self.word(rec)?;
        let radius = self.word(rec + 1)?;
        if radius.is_nan() || radius <= 0.0 {
            return Err(EphemerisError::Malformed("non-positive record radius"));
        }
        let s = (et - mid) / radius;

        let ncoef = if seg.data_type == 2 {
            (rsize - 2) / 3
        } else {
            (rsize - 2) / 6
        };
        if ncoef == 0 {
            return Err(EphemerisError::Malformed("record has no coefficients"));
        }

        let mut position = [0.0; 3];
        let mut velocity = [0.0; 3];
        for c in 0..3 {
            let pos_base = rec + 2 + c * ncoef;
            let (value, deriv) = chebyshev_eval(s, ncoef, |k| self.word(pos_base + k))?;
            position[c] = value;
            if seg.data_type == 2 {
                velocity[c] = deriv / radius;
            } else {
                let vel_base = rec + 2 + (3 + c) * ncoef;
                velocity[c] = chebyshev_eval(s, ncoef, |k| self.word(vel_base + k))?.0;
            }
        }
        Ok(StateVector { position, velocity })
    }
}

/// Iterator over the segment summaries of an SPK file.
#[derive(Debug, Clone)]
pub struct Segments<'a> {
    file: SpkFile<'a>,
    next_record: u32,
    base: usize,
    count: usize,
    index: usize,
    failed: bool,
}

impl Iterator for Segments<'_> {
    type Item = Result<Segment>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.failed {
            return None;
        }
        loop {
            if self.index < self.count {
                let offset = self.base + SUMMARY_RECORD_HEADER + self.index * SPK_SUMMARY_BYTES;
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
            // Load the next summary record: NEXT, PREV, NSUM as three doubles.
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

fn read_summary(bytes: &[u8], offset: usize, endian: Endian) -> Result<Segment> {
    let short = EphemerisError::Malformed("summary beyond end of file");
    let start_et = read_f64(bytes, offset, endian).ok_or_else(|| short.clone())?;
    let end_et = read_f64(bytes, offset + 8, endian).ok_or_else(|| short.clone())?;
    let int_at =
        |i: usize| read_i32(bytes, offset + 16 + 4 * i, endian).ok_or_else(|| short.clone());
    Ok(Segment {
        start_et,
        end_et,
        target: int_at(0)?,
        center: int_at(1)?,
        frame: int_at(2)?,
        data_type: int_at(3)?,
        start_word: int_at(4)?.max(0) as u32,
        end_word: int_at(5)?.max(0) as u32,
    })
}

/// Reads a whole file into memory, for use with [`SpkFile::new`].
///
/// # Errors
///
/// Returns [`EphemerisError::Io`] if the file cannot be read.
#[cfg(feature = "std")]
pub fn read_file(path: impl AsRef<std::path::Path>) -> Result<std::vec::Vec<u8>> {
    Ok(std::fs::read(path)?)
}

/// Light-time corrected state of a target as seen from an observer body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LightTimeState {
    /// Target relative to the observer, evaluated at the emission epoch.
    pub state: StateVector,
    /// One-way light time in seconds.
    pub light_time_s: f64,
}

/// Applies the one-iteration light-time correction.
///
/// The geometric distance `|r_target(et) - r_observer(et)|` gives a light time
/// `tau`. The target is then evaluated at `et - tau`, and the observer stays at
/// `et`. This is the simple model described in the crate docs.
///
/// Limits of this model:
/// - One iteration only. The residual error is of order `tau * v_target / c`
///   times `tau`, so it is well below a millisecond for planets but is not
///   zero.
/// - No stellar aberration, no gravitational light deflection, and no
///   relativistic (Shapiro) delay. Those effects are not included.
/// - The result is relative to the observer's barycentric state, with no
///   correction for the observer's own motion during the light path beyond
///   what the one-iteration model already captures.
///
/// # Errors
///
/// Propagates ephemeris lookup and evaluation errors.
#[cfg(feature = "std")]
pub fn light_time_corrected(
    spk: &SpkFile<'_>,
    target: i32,
    observer: i32,
    et: f64,
) -> Result<LightTimeState> {
    let obs = spk.state(observer, naif::SSB, et)?;
    let geo = spk.state(target, naif::SSB, et)?.minus(obs);
    let distance = norm(geo.position);
    let light_time_s = distance / SPEED_OF_LIGHT_KM_S;
    let emitted = spk.state(target, naif::SSB, et - light_time_s)?;
    Ok(LightTimeState {
        state: emitted.minus(obs),
        light_time_s,
    })
}

/// Earth-Sun distance in km from an SPK file (Sun to Earth geocentre).
///
/// # Errors
///
/// Propagates ephemeris lookup and evaluation errors. The file must contain
/// segments for Earth (399) and the Sun (10), relative to a common centre.
#[cfg(feature = "std")]
pub fn earth_sun_distance_km(spk: &SpkFile<'_>, et: f64) -> Result<f64> {
    let state = spk.state(naif::EARTH, naif::SUN, et)?;
    Ok(norm(state.position))
}

/// Low-precision Earth-Sun distance in AU, from a short analytic series.
///
/// Uses the mean anomaly of the Sun, `M = 357.53 + 0.98560028 d` degrees with
/// `d` days from J2000 TT, and `R = 1.00014 - 0.01671 cos M - 0.00014 cos 2M`
/// (Meeus, Astronomical Algorithms, Chapter 25, low-precision). It needs no
/// ephemeris file. Its accuracy is about 1e-4 AU. Against `erfa.epv00` at 200
/// TT epochs over 1900-2100 the maximum difference is 9.99e-5 AU (mean 2.8e-5
/// AU); see `docs/validation.md`.
#[cfg(feature = "std")]
#[must_use]
pub fn earth_sun_distance_au_approx(tt_hi: f64, tt_lo: f64) -> f64 {
    let d = (tt_hi - J2000_JD) + tt_lo;
    let m = (357.53 + 0.985_600_28 * d).to_radians();
    1.000_14 - 0.016_71 * m.cos() - 0.000_14 * (2.0 * m).cos()
}

#[cfg(feature = "std")]
fn norm(v: [f64; 3]) -> f64 {
    (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use std::vec::Vec;

    /// Builder for a synthetic little- or big-endian SPK file.
    struct SyntheticSpk {
        endian: Endian,
        /// Segments: (summary, data words).
        segments: Vec<(Segment, Vec<f64>)>,
    }

    impl SyntheticSpk {
        fn new(endian: Endian) -> Self {
            Self {
                endian,
                segments: Vec::new(),
            }
        }

        fn push_u32(&self, out: &mut Vec<u8>, v: u32) {
            match self.endian {
                Endian::Little => out.extend_from_slice(&v.to_le_bytes()),
                Endian::Big => out.extend_from_slice(&v.to_be_bytes()),
            }
        }

        fn push_i32(&self, out: &mut Vec<u8>, v: i32) {
            match self.endian {
                Endian::Little => out.extend_from_slice(&v.to_le_bytes()),
                Endian::Big => out.extend_from_slice(&v.to_be_bytes()),
            }
        }

        fn push_f64(&self, out: &mut Vec<u8>, v: f64) {
            match self.endian {
                Endian::Little => out.extend_from_slice(&v.to_le_bytes()),
                Endian::Big => out.extend_from_slice(&v.to_be_bytes()),
            }
        }

        /// Encodes the file. Record 1 is the file record, record 2 the summary
        /// record, record 3 the name record, and data starts at record 4.
        fn build(&self) -> Vec<u8> {
            let mut file = Vec::new();
            // File record.
            let mut rec1 = Vec::new();
            rec1.extend_from_slice(b"DAF/SPK ");
            self.push_u32(&mut rec1, 2); // ND
            self.push_u32(&mut rec1, 6); // NI
            rec1.resize(76, b' ');
            self.push_u32(&mut rec1, 2); // FWARD
            self.push_u32(&mut rec1, 2); // BWARD
            self.push_u32(&mut rec1, 0); // FREE (unused here)
            rec1.resize(88, 0);
            rec1.extend_from_slice(match self.endian {
                Endian::Little => b"LTL-IEEE",
                Endian::Big => b"BIG-IEEE",
            });
            rec1.resize(RECORD_BYTES, 0);
            file.extend_from_slice(&rec1);

            // Summary record.
            let mut rec2 = Vec::new();
            self.push_f64(&mut rec2, 0.0); // NEXT
            self.push_f64(&mut rec2, 0.0); // PREV
            self.push_f64(&mut rec2, self.segments.len() as f64); // NSUM
            // Data words start at record 4 (word 3*128 + 1 = 385).
            let mut word = 3 * (RECORD_BYTES / WORD_BYTES) + 1;
            let mut data_bytes = Vec::new();
            let mut summaries = Vec::new();
            for (seg, data) in &self.segments {
                let start = word;
                let end = word + data.len() - 1;
                for v in data {
                    self.push_f64(&mut data_bytes, *v);
                }
                word = end + 1;
                summaries.push((
                    seg.start_et,
                    seg.end_et,
                    seg.target,
                    seg.center,
                    seg.frame,
                    seg.data_type,
                    start,
                    end,
                ));
            }
            for (start_et, end_et, target, center, frame, dtype, sw, ew) in summaries {
                self.push_f64(&mut rec2, start_et);
                self.push_f64(&mut rec2, end_et);
                self.push_i32(&mut rec2, target);
                self.push_i32(&mut rec2, center);
                self.push_i32(&mut rec2, frame);
                self.push_i32(&mut rec2, dtype);
                self.push_i32(&mut rec2, sw as i32);
                self.push_i32(&mut rec2, ew as i32);
            }
            rec2.resize(RECORD_BYTES, 0);
            file.extend_from_slice(&rec2);

            // Name record (empty).
            file.extend_from_slice(&[b' '; RECORD_BYTES]);
            // Data.
            file.extend_from_slice(&data_bytes);
            // Pad to a whole record.
            let rem = file.len() % RECORD_BYTES;
            if rem != 0 {
                file.resize(file.len() + RECORD_BYTES - rem, 0);
            }
            file
        }
    }

    /// Chebyshev data for a type-2 segment with two records of degree 3.
    /// Record `r` covers `[r*100, r*100+100]` s with MID = r*100+50, RADIUS = 50.
    fn type2_data(coefs: &[[[f64; 4]; 2]; 3]) -> Vec<f64> {
        let mut words = Vec::new();
        for r in 0..2 {
            words.push(50.0 + 100.0 * r as f64); // MID
            words.push(50.0); // RADIUS
            for comp in coefs.iter() {
                words.extend_from_slice(&comp[r]);
            }
        }
        // Directory: INIT, INTLEN, RSIZE, N.
        words.extend_from_slice(&[0.0, 100.0, 14.0, 2.0]);
        words
    }

    /// Reference Chebyshev polynomial value and derivative, computed from
    /// explicit closed forms T0..T3 rather than the recurrence.
    fn reference_poly(a: [f64; 4], s: f64) -> (f64, f64) {
        let t = [1.0, s, 2.0 * s * s - 1.0, 4.0 * s * s * s - 3.0 * s];
        let dt = [0.0, 1.0, 4.0 * s, 12.0 * s * s - 3.0];
        let v = (0..4).map(|k| a[k] * t[k]).sum();
        let d = (0..4).map(|k| a[k] * dt[k]).sum();
        (v, d)
    }

    fn sample_coefs() -> [[[f64; 4]; 2]; 3] {
        [
            [[1.0, 2.0, -0.5, 0.25], [3.0, -1.0, 0.75, 0.1]],
            [[-2.0, 0.5, 1.5, -0.3], [0.2, 0.4, -0.6, 0.8]],
            [[0.7, -0.9, 0.3, 0.05], [1.1, 0.0, -0.2, 0.6]],
        ]
    }

    fn type2_segment(target: i32, center: i32) -> (Segment, Vec<f64>) {
        let data = type2_data(&sample_coefs());
        let len = data.len() as u32;
        (
            Segment {
                start_et: 0.0,
                end_et: 200.0,
                target,
                center,
                frame: 1,
                data_type: 2,
                start_word: 0,
                end_word: len,
            },
            data,
        )
    }

    #[test]
    #[allow(clippy::needless_range_loop)]
    fn type2_matches_closed_form_polynomial_and_derivative() {
        for endian in [Endian::Little, Endian::Big] {
            let mut builder = SyntheticSpk::new(endian);
            builder.segments.push(type2_segment(301, 3));
            let bytes = builder.build();
            let spk = SpkFile::new(&bytes).expect("parse");
            assert_eq!(spk.endian(), endian);
            let seg = spk.find_segment(301, 30.0).expect("segment");
            let coefs = sample_coefs();
            // Record 0 covers [0, 100] s, so s = (et - 50) / 50.
            for &et in &[0.0, 17.0, 30.0, 49.5, 50.0] {
                let st = spk.evaluate(&seg, et).expect("eval");
                let s = (et - 50.0) / 50.0;
                for c in 0..3 {
                    let (v, d) = reference_poly(coefs[c][0], s);
                    assert!((st.position[c] - v).abs() < 1e-12, "pos c={c} et={et}");
                    assert!(
                        (st.velocity[c] - d / 50.0).abs() < 1e-12,
                        "vel c={c} et={et}"
                    );
                }
            }
            // Record 1 covers [100, 200] s, so s = (et - 150) / 50.
            for &et in &[100.0, 151.0, 199.0, 200.0] {
                let st = spk.evaluate(&seg, et).expect("eval");
                let s = (et - 150.0) / 50.0;
                for c in 0..3 {
                    let (v, d) = reference_poly(coefs[c][1], s);
                    assert!((st.position[c] - v).abs() < 1e-12);
                    assert!((st.velocity[c] - d / 50.0).abs() < 1e-12);
                }
            }
        }
    }

    #[test]
    fn velocity_matches_finite_difference_of_position() {
        let mut builder = SyntheticSpk::new(Endian::Little);
        builder.segments.push(type2_segment(301, 3));
        let bytes = builder.build();
        let spk = SpkFile::new(&bytes).expect("parse");
        let seg = spk.find_segment(301, 10.0).expect("segment");
        let et = 60.0;
        let h = 1e-3;
        let p1 = spk.evaluate(&seg, et + h).expect("eval").position;
        let p0 = spk.evaluate(&seg, et - h).expect("eval").position;
        let v = spk.evaluate(&seg, et).expect("eval").velocity;
        for c in 0..3 {
            let fd = (p1[c] - p0[c]) / (2.0 * h);
            assert!((fd - v[c]).abs() < 1e-7, "c={c} fd={fd} v={}", v[c]);
        }
    }

    #[test]
    fn type3_returns_stored_velocity_polynomial() {
        // One record of degree 2: MID, RADIUS, then position coefficients for
        // x, y, z and velocity coefficients for vx, vy, vz (3 each). RSIZE = 20.
        let pc = |c: usize| [c as f64 + 1.0, 0.5, -0.25];
        let vc = |c: usize| [0.1 * (c as f64 + 1.0), -0.2, 0.05];
        let mut body = vec![100.0, 100.0];
        for c in 0..3 {
            body.extend_from_slice(&pc(c));
        }
        for c in 0..3 {
            body.extend_from_slice(&vc(c));
        }
        // Directory: INIT = 0, INTLEN = 200, RSIZE = 20, N = 1.
        body.extend_from_slice(&[0.0, 200.0, 20.0, 1.0]);

        let mut builder = SyntheticSpk::new(Endian::Big);
        let len = body.len() as u32;
        builder.segments.push((
            Segment {
                start_et: 0.0,
                end_et: 200.0,
                target: 399,
                center: 3,
                frame: 1,
                data_type: 3,
                start_word: 0,
                end_word: len,
            },
            body,
        ));
        let bytes = builder.build();
        let spk = SpkFile::new(&bytes).expect("parse");
        let seg = spk.find_segment(399, 50.0).expect("segment");
        let et = 150.0;
        let st = spk.evaluate(&seg, et).expect("eval");
        // Record covers [0, 200] s around MID = 100, so s = (et - 100) / 100.
        let s = (et - 100.0) / 100.0;
        for c in 0..3 {
            let p = pc(c);
            let v = vc(c);
            let pos = p[0] + p[1] * s + p[2] * (2.0 * s * s - 1.0);
            let vel = v[0] + v[1] * s + v[2] * (2.0 * s * s - 1.0);
            assert!((st.position[c] - pos).abs() < 1e-12);
            assert!((st.velocity[c] - vel).abs() < 1e-12);
        }
    }

    #[test]
    fn out_of_range_and_missing_segments_are_reported() {
        let mut builder = SyntheticSpk::new(Endian::Little);
        builder.segments.push(type2_segment(301, 3));
        let bytes = builder.build();
        let spk = SpkFile::new(&bytes).expect("parse");
        assert!(matches!(
            spk.find_segment(301, 500.0),
            Err(EphemerisError::OutOfRange { .. })
        ));
        assert!(matches!(
            spk.find_segment(10, 0.0),
            Err(EphemerisError::NotFound { target: 10 })
        ));
        let seg = spk.find_segment(301, 0.0).expect("segment");
        assert!(matches!(
            spk.evaluate(&seg, -1.0),
            Err(EphemerisError::OutOfRange { .. })
        ));
    }

    #[test]
    fn barycentre_chain_adds_offsets() {
        // Earth (399) relative to EMB (3), EMB relative to SSB (0), Sun relative to SSB.
        // Degree-0 type-2 records (RSIZE = 5, one record) make the states constant.
        let make = |x: f64, y: f64, z: f64| -> Vec<f64> {
            let mut w = vec![100.0, 100.0, x, y, z];
            w.extend_from_slice(&[0.0, 200.0, 5.0, 1.0]);
            w
        };
        let mut builder = SyntheticSpk::new(Endian::Little);
        let mk = |target: i32, center: i32, data: Vec<f64>| {
            let len = data.len() as u32;
            (
                Segment {
                    start_et: 0.0,
                    end_et: 200.0,
                    target,
                    center,
                    frame: 1,
                    data_type: 2,
                    start_word: 0,
                    end_word: len,
                },
                data,
            )
        };
        builder.segments.push(mk(399, 3, make(1.0, 2.0, 3.0)));
        builder.segments.push(mk(3, 0, make(10.0, 20.0, 30.0)));
        builder.segments.push(mk(10, 0, make(100.0, 200.0, 300.0)));
        let bytes = builder.build();
        let spk = SpkFile::new(&bytes).expect("parse");
        let earth_ssb = spk.state(399, naif::SSB, 50.0).expect("state");
        assert_eq!(earth_ssb.position, [11.0, 22.0, 33.0]);
        // Velocities of degree-0 type-2 segments are zero.
        assert_eq!(earth_ssb.velocity, [0.0; 3]);
        let earth_sun = spk.state(399, naif::SUN, 50.0).expect("state");
        assert_eq!(
            earth_sun.position,
            [11.0 - 100.0, 22.0 - 200.0, 33.0 - 300.0]
        );
        let sun_earth = spk.state(naif::SUN, naif::EARTH, 50.0).expect("state");
        assert_eq!(
            sun_earth.position,
            [100.0 - 11.0, 200.0 - 22.0, 300.0 - 33.0]
        );
    }

    #[test]
    fn rejects_non_spk_and_short_inputs() {
        assert_eq!(
            SpkFile::new(&[0u8; 100]).err(),
            Some(EphemerisError::TooShort)
        );
        let mut junk = [0u8; RECORD_BYTES];
        junk[..8].copy_from_slice(b"DAF/XFER");
        assert_eq!(SpkFile::new(&junk).err(), Some(EphemerisError::NotSpk));
    }

    #[test]
    fn light_time_is_distance_over_c_for_static_target() {
        // Constant-position target 1e8 km from a fixed observer at the SSB:
        // light time must equal distance / c and the emitted state equals the
        // constant position minus the observer.
        // One degree-0 record covering [0, 2000] s, so the emission epoch
        // et - tau (about 667 s) stays inside coverage.
        let make = |x: f64, y: f64, z: f64| -> Vec<f64> {
            let mut w = vec![1000.0, 1000.0, x, y, z];
            w.extend_from_slice(&[0.0, 2000.0, 5.0, 1.0]);
            w
        };
        let mut builder = SyntheticSpk::new(Endian::Little);
        let mk = |target: i32, data: Vec<f64>| {
            let len = data.len() as u32;
            (
                Segment {
                    start_et: 0.0,
                    end_et: 2000.0,
                    target,
                    center: 0,
                    frame: 1,
                    data_type: 2,
                    start_word: 0,
                    end_word: len,
                },
                data,
            )
        };
        builder.segments.push(mk(5, make(1.0e8, 0.0, 0.0)));
        builder.segments.push(mk(0, make(0.0, 0.0, 0.0)));
        let bytes = builder.build();
        let spk = SpkFile::new(&bytes).expect("parse");
        let lt = light_time_corrected(&spk, 5, 0, 1000.0).expect("lt");
        assert!((lt.light_time_s - 1.0e8 / SPEED_OF_LIGHT_KM_S).abs() < 1e-9);
        assert!((lt.state.position[0] - 1.0e8).abs() < 1e-6);
    }

    #[test]
    fn earth_sun_approx_is_within_perihelion_aphelion_range() {
        // Perihelion near 3 January, aphelion near 4 July (2000).
        let jan = earth_sun_distance_au_approx(J2000_JD + 2.0, 0.0);
        let jul = earth_sun_distance_au_approx(J2000_JD + 185.0, 0.0);
        assert!((jan - 0.9833).abs() < 2e-3, "jan={jan}");
        assert!((jul - 1.0167).abs() < 2e-3, "jul={jul}");
        assert!(jan < jul);
    }

    #[test]
    fn et_conversion_is_seconds_past_j2000() {
        assert_eq!(et_from_tdb_jd(J2000_JD, 0.5), 43_200.0);
    }
}
