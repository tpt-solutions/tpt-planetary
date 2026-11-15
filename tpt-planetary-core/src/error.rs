//! Error type for the core crate.
//!
//! The type is usable without `std`. `std::error::Error` is implemented only
//! when the `std` feature is enabled.

use core::fmt;

/// Errors returned by fallible operations in `tpt-planetary-core`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Error {
    /// A calendar date does not exist in the requested calendar system
    /// (for example month 13, or 30 February).
    InvalidDate,
    /// A time of day is outside its valid range.
    InvalidTime,
    /// A UTC leap second (`second >= 60`) was given for a UTC day that does
    /// not end in a leap second in the supplied leap-second table.
    LeapSecondNotInTable,
    /// The instant lies before the first entry of the leap-second table
    /// (before 1972-01-01 for the embedded table), so UTC is not defined here.
    BeforeLeapSecondTable,
    /// The instant lies after the table's validity date. The table may be
    /// missing announced leap seconds, so the conversion is refused.
    LeapSecondTableExpired,
    /// A leap-second table failed validation (unsorted, duplicate, or with a
    /// step other than +1 second).
    InvalidLeapSecondTable,
    /// An iterative algorithm did not converge within its iteration limit.
    NonConvergence,
    /// A vector with zero length was given where a direction is required.
    ZeroVector,
    /// Two points are antipodal, so the great circle joining them is undefined.
    Antipodal,
    /// An axis or quaternion argument is not a valid, non-degenerate rotation.
    InvalidRotation,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let msg = match self {
            Error::InvalidDate => "calendar date does not exist in the given calendar",
            Error::InvalidTime => "time of day is out of range",
            Error::LeapSecondNotInTable => {
                "UTC leap second given for a day that does not end in a leap second"
            }
            Error::BeforeLeapSecondTable => "instant precedes the leap-second table",
            Error::LeapSecondTableExpired => {
                "instant lies after the leap-second table's validity date"
            }
            Error::InvalidLeapSecondTable => "leap-second table is malformed",
            Error::NonConvergence => "iterative algorithm did not converge",
            Error::ZeroVector => "zero-length vector has no direction",
            Error::Antipodal => "points are antipodal; the great circle is undefined",
            Error::InvalidRotation => "axis or quaternion is not a valid rotation",
        };
        f.write_str(msg)
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
