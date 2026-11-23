//! Vectorised slice kernels, compiled only with the `simd` feature.
//!
//! The kernels use the fixed-width, `no_std`, `#![forbid(unsafe_code)]` vector
//! type of `tpt-simd-vector`. Each lane applies the same IEEE operation as the
//! scalar code in [`crate::ops::elementwise`] and [`crate::ops::reduce`], which
//! remain the reference. Two differences are allowed and are documented where
//! they occur:
//!
//! - [`stats`] sums in lane-parallel order, so its sum can differ from a
//!   sequential sum in the last bits. Count, minimum, maximum and the NaN flag
//!   are exact.
//! - Element tails (the last `len % LANES` values) run the scalar operation.

use alloc::vec::Vec;

use tpt_simd_vector::Simd;

use crate::ops::elementwise::{ArithOp, CmpOp, UnaryOp};

/// Number of `f64` lanes per vector.
const LANES: usize = 4;
type V = Simd<f64, LANES>;

/// Elementwise `a op b` for equal-length slices. Returns `None` for operators
/// without a vector kernel (`Pow`), so the caller uses the scalar path.
pub(crate) fn arith(op: ArithOp, a: &[f64], b: &[f64]) -> Option<Vec<f64>> {
    let vector_op: fn(V, V) -> V = match op {
        ArithOp::Add => |x, y| x + y,
        ArithOp::Sub => |x, y| x - y,
        ArithOp::Mul => |x, y| x * y,
        ArithOp::Div => |x, y| x / y,
        #[cfg(feature = "std")]
        ArithOp::Pow => return None,
    };
    debug_assert_eq!(a.len(), b.len());
    let split = a.len() - a.len() % LANES;
    let mut out = Vec::with_capacity(a.len());
    for (ca, cb) in a[..split]
        .chunks_exact(LANES)
        .zip(b[..split].chunks_exact(LANES))
    {
        let r = vector_op(V::from_slice(ca), V::from_slice(cb));
        out.extend_from_slice(&r.0);
    }
    out.extend(
        a[split..]
            .iter()
            .zip(&b[split..])
            .map(|(&x, &y)| op.apply(x, y)),
    );
    Some(out)
}

/// Elementwise comparison, giving 1.0 where it holds and 0.0 elsewhere. Lane
/// comparisons are IEEE, so NaN compares false exactly as the scalar test does.
pub(crate) fn compare(op: CmpOp, a: &[f64], b: &[f64]) -> Option<Vec<f64>> {
    debug_assert_eq!(a.len(), b.len());
    let split = a.len() - a.len() % LANES;
    let mut out = Vec::with_capacity(a.len());
    for (ca, cb) in a[..split]
        .chunks_exact(LANES)
        .zip(b[..split].chunks_exact(LANES))
    {
        let (x, y) = (V::from_slice(ca), V::from_slice(cb));
        let holds = match op {
            CmpOp::Lt => x.simd_lt(y),
            CmpOp::Le => x.simd_le(y),
            CmpOp::Gt => x.simd_gt(y),
            CmpOp::Ge => x.simd_ge(y),
            CmpOp::Eq => x.simd_eq(y),
            CmpOp::Ne => x.simd_ne(y),
        };
        out.extend(holds.to_array().iter().map(|&h| if h { 1.0 } else { 0.0 }));
    }
    out.extend(
        a[split..]
            .iter()
            .zip(&b[split..])
            .map(|(&x, &y)| op.apply(x, y)),
    );
    Some(out)
}

/// Elementwise `op(x)` for `Abs` and `Sqrt`. Returns `None` for other
/// operators, which keep the scalar path.
pub(crate) fn unary(op: UnaryOp, xs: &[f64]) -> Option<Vec<f64>> {
    let vector_op: fn(V) -> V = match op {
        UnaryOp::Abs => |x| x.abs(),
        #[cfg(feature = "std")]
        UnaryOp::Sqrt => |x| x.sqrt(),
        #[cfg(feature = "std")]
        _ => return None,
    };
    let split = xs.len() - xs.len() % LANES;
    let mut out = Vec::with_capacity(xs.len());
    for c in xs[..split].chunks_exact(LANES) {
        out.extend_from_slice(&vector_op(V::from_slice(c)).0);
    }
    out.extend(xs[split..].iter().map(|&x| op.apply(x)));
    Some(out)
}

/// Summary of a slice for the reduction accumulator. NaN values are excluded
/// from `count`, `sum`, `min` and `max`, and set `has_nan`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Stats {
    pub count: u64,
    pub sum: f64,
    pub min: f64,
    pub max: f64,
    pub has_nan: bool,
}

/// Computes [`Stats`] for `xs` with lane-parallel accumulators.
pub(crate) fn stats(xs: &[f64]) -> Stats {
    let mut sum = V::splat(0.0);
    let mut lo = V::splat(f64::INFINITY);
    let mut hi = V::splat(f64::NEG_INFINITY);
    let mut nans = V::splat(0.0);
    let split = xs.len() - xs.len() % LANES;
    for c in xs[..split].chunks_exact(LANES) {
        let v = V::from_slice(c);
        // `min` and `max` ignore NaN operands, so NaN lanes do not affect them.
        lo = lo.min(v);
        hi = hi.max(v);
        sum += v.map(|x| if x.is_nan() { 0.0 } else { x });
        nans += v.map(|x| if x.is_nan() { 1.0 } else { 0.0 });
    }
    let mut sum_all = 0.0;
    let mut lo_all = f64::INFINITY;
    let mut hi_all = f64::NEG_INFINITY;
    let mut nan_count = 0.0;
    for l in 0..LANES {
        sum_all += sum.0[l];
        lo_all = lo_all.min(lo.0[l]);
        hi_all = hi_all.max(hi.0[l]);
        nan_count += nans.0[l];
    }
    for &x in &xs[split..] {
        if x.is_nan() {
            nan_count += 1.0;
        } else {
            sum_all += x;
            lo_all = lo_all.min(x);
            hi_all = hi_all.max(x);
        }
    }
    let nan_count = nan_count as u64;
    Stats {
        count: xs.len() as u64 - nan_count,
        sum: sum_all,
        min: lo_all,
        max: hi_all,
        has_nan: nan_count > 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic linear congruential generator (no external dependency).
    struct Lcg(u64);

    impl Lcg {
        fn next_f64(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            // Top 53 bits as a uniform value in [0, 1), scaled to [-100, 100).
            ((self.0 >> 11) as f64 / (1u64 << 53) as f64) * 200.0 - 100.0
        }
    }

    /// Random-like data with NaN, infinities and signed zeros mixed in.
    fn data(n: usize, seed: u64) -> Vec<f64> {
        let mut g = Lcg(seed);
        (0..n)
            .map(|i| match i % 37 {
                5 => f64::NAN,
                11 => f64::INFINITY,
                17 => -0.0,
                23 => 0.0,
                _ => g.next_f64(),
            })
            .collect()
    }

    /// Equal values, with NaN equal to NaN and signed zeros equal.
    fn same(a: f64, b: f64) -> bool {
        (a.is_nan() && b.is_nan()) || a == b
    }

    const LENGTHS: &[usize] = &[0, 1, 2, 3, 4, 5, 7, 8, 9, 13, 64, 67, 1001];

    #[test]
    fn arith_matches_scalar_reference() {
        for &n in LENGTHS {
            let a = data(n, 1);
            let b = data(n, 2);
            for op in [ArithOp::Add, ArithOp::Sub, ArithOp::Mul, ArithOp::Div] {
                let got = arith(op, &a, &b).expect("vectorised");
                assert_eq!(got.len(), n);
                for i in 0..n {
                    let want = op.apply(a[i], b[i]);
                    assert!(
                        same(got[i], want),
                        "{op:?} n={n} i={i}: {} vs {want}",
                        got[i]
                    );
                }
            }
        }
        assert!(arith(ArithOp::Pow, &[1.0], &[2.0]).is_none());
    }

    #[test]
    fn unary_matches_scalar_reference() {
        for &n in LENGTHS {
            let x = data(n, 3);
            for op in [UnaryOp::Abs, UnaryOp::Sqrt] {
                let got = unary(op, &x).expect("vectorised");
                assert_eq!(got.len(), n);
                for i in 0..n {
                    let want = op.apply(x[i]);
                    assert!(
                        same(got[i], want),
                        "{op:?} n={n} i={i}: {} vs {want}",
                        got[i]
                    );
                }
            }
        }
    }

    #[test]
    fn stats_matches_scalar_reference() {
        for &n in LENGTHS {
            let x = data(n, 4);
            let s = stats(&x);
            let mut want = Stats {
                count: 0,
                sum: 0.0,
                min: f64::INFINITY,
                max: f64::NEG_INFINITY,
                has_nan: false,
            };
            for &v in &x {
                if v.is_nan() {
                    want.has_nan = true;
                    continue;
                }
                want.count += 1;
                want.sum += v;
                want.min = want.min.min(v);
                want.max = want.max.max(v);
            }
            assert_eq!(s.count, want.count, "n={n}");
            assert_eq!(s.has_nan, want.has_nan, "n={n}");
            assert!(same(s.min, want.min) && same(s.max, want.max), "n={n}");
            let scale = want.sum.abs().max(1.0);
            // The data holds infinities, so the sum is often infinite; `same`
            // covers that case, and the tolerance covers lane-order rounding.
            assert!(
                same(s.sum, want.sum)
                    || (s.sum - want.sum).abs() <= 1e-12 * scale * n.max(1) as f64,
                "n={n}: {} vs {}",
                s.sum,
                want.sum
            );
        }
    }

    #[test]
    fn stats_on_empty_and_all_nan() {
        let e = stats(&[]);
        assert_eq!((e.count, e.sum, e.has_nan), (0, 0.0, false));
        assert_eq!((e.min, e.max), (f64::INFINITY, f64::NEG_INFINITY));
        let nan = stats(&[f64::NAN; 6]);
        assert_eq!(nan.count, 0);
        assert!(nan.has_nan);
    }
}

#[cfg(all(test, feature = "std"))]
mod compare_tests {
    use super::*;

    #[test]
    fn compare_matches_scalar_reference_including_nan_and_signed_zero() {
        let mut a = Vec::new();
        let mut b = Vec::new();
        let specials = [
            0.0,
            -0.0,
            1.0,
            -1.0,
            f64::NAN,
            f64::INFINITY,
            f64::NEG_INFINITY,
            2.5,
        ];
        for (i, &x) in specials.iter().enumerate() {
            for (j, &y) in specials.iter().enumerate() {
                if (i + j) % 3 != 0 {
                    a.push(x);
                    b.push(y);
                }
            }
        }
        // A tail that is not a whole vector, to exercise the scalar part.
        a.push(3.0);
        b.push(3.0);
        for op in [
            CmpOp::Lt,
            CmpOp::Le,
            CmpOp::Gt,
            CmpOp::Ge,
            CmpOp::Eq,
            CmpOp::Ne,
        ] {
            let got = compare(op, &a, &b).unwrap();
            let want: Vec<f64> = a.iter().zip(&b).map(|(&x, &y)| op.apply(x, y)).collect();
            assert_eq!(got.len(), want.len());
            for (k, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(g.to_bits(), w.to_bits(), "{op:?} at {k}");
            }
        }
    }
}
