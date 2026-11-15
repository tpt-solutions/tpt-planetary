# ADR-002: Pure Rust high-precision time and ephemeris

**Status:** Accepted

## Decision

Time-scale conversions and JPL ephemeris interpolation will be written in pure
Rust using `tpt-math`, rather than wrapping the ERFA C library.

## Rationale

Astropy relies on ERFA for fundamental astronomy. Implementing the IAU 2000/2006
resolutions and Chebyshev polynomial interpolation in safe Rust gives us
`no_std` compatibility for flight software and removes FFI bottlenecks.
