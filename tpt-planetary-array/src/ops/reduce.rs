//! Axis-wise reductions (sum, mean, min, max) with NaN-aware and
//! NaN-propagating variants.
//!
//! The input is streamed in blocks of at most [`EvalOptions::block_elements`]
//! elements, so memory use is the size of the reduced output plus one block.

use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::grid::c_strides;
use crate::lazy::{BlockIter, EvalOptions, Expr};
use crate::ops::nd::{element_count, for_each_index};

/// A reduction applied along one or more axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reduction {
    /// Sum of the elements (zero when there are none).
    Sum,
    /// Arithmetic mean of the elements (NaN when there are none).
    Mean,
    /// Smallest element (NaN when there are none).
    Min,
    /// Largest element (NaN when there are none).
    Max,
}

/// The result of a reduction: row-major values with the reduced shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Reduced {
    /// Shape of the result, with reduced axes removed. Empty for a full
    /// reduction over every axis.
    pub shape: Vec<u64>,
    /// Values in C order. Length is the product of `shape`.
    pub values: Vec<f64>,
}

/// Running accumulator for one output cell.
#[derive(Debug, Clone, Copy)]
struct Acc {
    count: u64,
    sum: f64,
    min: f64,
    max: f64,
    has_nan: bool,
}

impl Acc {
    const EMPTY: Acc = Acc {
        count: 0,
        sum: 0.0,
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
        has_nan: false,
    };

    fn push(&mut self, v: f64) {
        if v.is_nan() {
            self.has_nan = true;
            return;
        }
        self.count += 1;
        self.sum += v;
        self.min = self.min.min(v);
        self.max = self.max.max(v);
    }

    /// Pushes every value of `xs`, equivalent to calling [`Acc::push`] on each.
    #[cfg(feature = "simd")]
    fn push_slice(&mut self, xs: &[f64]) {
        let s = crate::ops::simd::stats(xs);
        self.has_nan |= s.has_nan;
        self.count += s.count;
        self.sum += s.sum;
        self.min = self.min.min(s.min);
        self.max = self.max.max(s.max);
    }

    fn finish(&self, op: Reduction) -> f64 {
        if self.has_nan {
            return f64::NAN;
        }
        match op {
            Reduction::Sum => self.sum,
            Reduction::Mean if self.count == 0 => f64::NAN,
            Reduction::Mean => self.sum / self.count as f64,
            Reduction::Min if self.count == 0 => f64::NAN,
            Reduction::Min => self.min,
            Reduction::Max if self.count == 0 => f64::NAN,
            Reduction::Max => self.max,
        }
    }
}

impl Expr {
    /// Reduces over `axes` with default block size.
    ///
    /// With `skip_invalid` set, fill-valued and NaN elements are ignored (the
    /// NaN-aware variant). Otherwise fill values are treated as ordinary
    /// numbers and any NaN element makes its output cell NaN.
    ///
    /// An empty `axes` list reduces nothing: the result has the input shape
    /// and each cell is the element itself (use `Reduction::Sum` with
    /// `skip_invalid = false` to materialise an expression).
    pub fn reduce_axes(
        &self,
        axes: &[usize],
        op: Reduction,
        skip_invalid: bool,
    ) -> Result<Reduced> {
        self.reduce_axes_with(axes, op, skip_invalid, EvalOptions::default())
    }

    /// Like [`Expr::reduce_axes`] with explicit block options.
    pub fn reduce_axes_with(
        &self,
        axes: &[usize],
        op: Reduction,
        skip_invalid: bool,
        options: EvalOptions,
    ) -> Result<Reduced> {
        if options.block_elements == 0 {
            return Err(Error::InvalidSelection(
                "block_elements must be positive".into(),
            ));
        }
        let shape = self.shape();
        let nd = shape.len();
        let mut reduced = vec![false; nd];
        for &ax in axes {
            if ax >= nd {
                return Err(Error::InvalidSelection(alloc::format!(
                    "axis {ax} out of range for {nd} dimensions"
                )));
            }
            if reduced[ax] {
                return Err(Error::InvalidSelection(alloc::format!(
                    "axis {ax} given more than once"
                )));
            }
            reduced[ax] = true;
        }
        let kept: Vec<usize> = (0..nd).filter(|&d| !reduced[d]).collect();
        let out_shape: Vec<u64> = kept.iter().map(|&d| shape[d]).collect();
        let out_strides = c_strides(&out_shape);
        // Stride in the output for each input dimension; zero when reduced.
        let mut in_strides = vec![0u64; nd];
        for (k, &d) in kept.iter().enumerate() {
            in_strides[d] = out_strides[k];
        }

        let mut acc = vec![Acc::EMPTY; element_count(&out_shape)];
        for region in BlockIter::new(&shape, options.block_elements) {
            let block = self.eval_region(&region)?;
            #[cfg(feature = "simd")]
            if acc.len() == 1 {
                // Every element feeds the single output cell, so the block can
                // be reduced as one slice.
                if skip_invalid {
                    let kept: Vec<f64> = block
                        .values
                        .iter()
                        .zip(&block.valid)
                        .filter(|(_, ok)| **ok)
                        .map(|(&v, _)| v)
                        .collect();
                    acc[0].push_slice(&kept);
                } else {
                    acc[0].push_slice(&block.values);
                }
                continue;
            }
            let mut i = 0usize;
            for_each_index(&region.count, |idx| {
                let ok = block.valid[i];
                let v = block.values[i];
                i += 1;
                if skip_invalid && !ok {
                    return;
                }
                let mut off = 0u64;
                for (d, &j) in idx.iter().enumerate() {
                    off += (region.start[d] + j) * in_strides[d];
                }
                acc[off as usize].push(v);
            });
        }
        Ok(Reduced {
            shape: out_shape,
            values: acc.iter().map(|a| a.finish(op)).collect(),
        })
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    /// Row-major 2x3x4 test data with a few holes.
    fn cube() -> (Vec<u64>, Vec<f64>) {
        let shape = vec![2u64, 3, 4];
        let values = (0..24).map(|i| f64::from(i) * 0.5 - 3.0).collect();
        (shape, values)
    }

    /// Scalar reference: reduce the cube over `axes` by brute force.
    fn reference(shape: &[u64], values: &[f64], axes: &[usize], op: Reduction) -> Vec<f64> {
        let out_dims: Vec<usize> = (0..shape.len()).filter(|d| !axes.contains(d)).collect();
        let out_shape: Vec<u64> = out_dims.iter().map(|&d| shape[d]).collect();
        let n_out: u64 = out_shape.iter().product();
        let mut out = Vec::new();
        for o in 0..n_out {
            // Decode output index.
            let mut rem = o;
            let mut oidx = vec![0u64; out_shape.len()];
            for k in (0..out_shape.len()).rev() {
                oidx[k] = rem % out_shape[k];
                rem /= out_shape[k];
            }
            let mut group = Vec::new();
            let n_in: u64 = shape.iter().product();
            for f in 0..n_in {
                let mut rem = f;
                let mut idx = vec![0u64; shape.len()];
                for d in (0..shape.len()).rev() {
                    idx[d] = rem % shape[d];
                    rem /= shape[d];
                }
                let matches = out_dims.iter().enumerate().all(|(k, &d)| idx[d] == oidx[k]);
                if matches {
                    group.push(values[f as usize]);
                }
            }
            let v = match op {
                Reduction::Sum => group.iter().sum(),
                Reduction::Mean => group.iter().sum::<f64>() / group.len() as f64,
                Reduction::Min => group.iter().copied().fold(f64::INFINITY, f64::min),
                Reduction::Max => group.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            };
            out.push(v);
        }
        out
    }

    fn check(axes: &[usize]) {
        let (shape, values) = cube();
        let e = Expr::from_values(shape.clone(), values.clone()).unwrap();
        for op in [
            Reduction::Sum,
            Reduction::Mean,
            Reduction::Min,
            Reduction::Max,
        ] {
            let r = e.reduce_axes(axes, op, true).unwrap();
            let expected = reference(&shape, &values, axes, op);
            assert_eq!(r.values.len(), expected.len(), "{axes:?} {op:?}");
            for (g, x) in r.values.iter().zip(&expected) {
                assert!((g - x).abs() < 1e-12, "{axes:?} {op:?}: {g} vs {x}");
            }
        }
    }

    #[test]
    fn single_and_multiple_axes_match_reference() {
        check(&[0]);
        check(&[1]);
        check(&[2]);
        check(&[0, 2]);
        check(&[1, 2]);
        check(&[0, 1, 2]);
    }

    #[test]
    fn reduced_shape_drops_axes() {
        let (shape, values) = cube();
        let e = Expr::from_values(shape, values).unwrap();
        assert_eq!(
            e.reduce_axes(&[1], Reduction::Sum, true).unwrap().shape,
            vec![2, 4]
        );
        assert_eq!(
            e.reduce_axes(&[0, 2], Reduction::Sum, true).unwrap().shape,
            vec![3]
        );
        assert_eq!(
            e.reduce_axes(&[0, 1, 2], Reduction::Sum, true)
                .unwrap()
                .shape,
            Vec::<u64>::new()
        );
        assert!(e.reduce_axes(&[3], Reduction::Sum, true).is_err());
        assert!(e.reduce_axes(&[1, 1], Reduction::Sum, true).is_err());
    }

    #[test]
    fn nan_aware_and_propagating_variants() {
        // Two rows of three; column 1 holds a NaN and a fill-like -9999.
        let e = Expr::from_values(vec![2, 3], vec![1.0, f64::NAN, 3.0, 4.0, 5.0, 6.0]).unwrap();
        let skip = e.reduce_axes(&[1], Reduction::Mean, true).unwrap();
        assert_eq!(skip.values, vec![2.0, 5.0]);
        let prop = e.reduce_axes(&[1], Reduction::Sum, false).unwrap();
        assert!(prop.values[0].is_nan());
        assert_eq!(prop.values[1], 15.0);
    }

    #[test]
    fn fully_masked_cells_are_nan_or_zero() {
        let e = Expr::from_values(vec![2, 2], vec![f64::NAN, f64::NAN, 1.0, 2.0]).unwrap();
        let r = e.reduce_axes(&[1], Reduction::Mean, true).unwrap();
        assert!(r.values[0].is_nan());
        assert_eq!(r.values[1], 1.5);
        let s = e.reduce_axes(&[1], Reduction::Sum, true).unwrap();
        assert_eq!(s.values, vec![0.0, 3.0]);
    }

    #[test]
    fn result_is_independent_of_block_size() {
        let (shape, values) = cube();
        let e = Expr::from_values(shape, values).unwrap();
        let big = e
            .reduce_axes_with(
                &[0],
                Reduction::Mean,
                true,
                EvalOptions {
                    block_elements: 1000,
                },
            )
            .unwrap();
        let small = e
            .reduce_axes_with(
                &[0],
                Reduction::Mean,
                true,
                EvalOptions { block_elements: 5 },
            )
            .unwrap();
        for (a, b) in big.values.iter().zip(&small.values) {
            assert!((a - b).abs() < 1e-12);
        }
    }

    #[test]
    fn empty_axis_materialises_elements() {
        let (shape, values) = cube();
        let e = Expr::from_values(shape.clone(), values.clone()).unwrap();
        let r = e.reduce_axes(&[], Reduction::Sum, false).unwrap();
        assert_eq!(r.shape, shape);
        assert_eq!(r.values, values);
    }

    /// Full reductions over lengths that are not multiples of the vector width,
    /// with NaN and fill-like holes, against a sequential reference.
    #[test]
    fn full_reduction_matches_sequential_reference() {
        let mut state = 7u64;
        for n in [1usize, 3, 4, 5, 6, 7, 9, 31, 1000] {
            let values: Vec<f64> = (0..n)
                .map(|i| {
                    state = state
                        .wrapping_mul(6_364_136_223_846_793_005)
                        .wrapping_add(1_442_695_040_888_963_407);
                    if i % 9 == 4 {
                        f64::NAN
                    } else {
                        ((state >> 11) as f64 / (1u64 << 53) as f64) * 10.0 - 5.0
                    }
                })
                .collect();
            let valid: Vec<f64> = values.iter().copied().filter(|v| !v.is_nan()).collect();
            let e = Expr::from_values(vec![n as u64], values.clone()).unwrap();
            let sum: f64 = valid.iter().sum();
            let min = valid.iter().copied().fold(f64::INFINITY, f64::min);
            let max = valid.iter().copied().fold(f64::NEG_INFINITY, f64::max);

            let skip = e.reduce_axes(&[0], Reduction::Sum, true).unwrap();
            assert!((skip.values[0] - sum).abs() < 1e-9, "n={n}");
            let skip_min = e.reduce_axes(&[0], Reduction::Min, true).unwrap();
            assert_eq!(skip_min.values[0], min, "n={n}");
            let skip_max = e.reduce_axes(&[0], Reduction::Max, true).unwrap();
            assert_eq!(skip_max.values[0], max, "n={n}");

            // Without skipping, a single NaN makes the result NaN (index 4 holds one).
            if n > 4 {
                let prop = e.reduce_axes(&[0], Reduction::Sum, false).unwrap();
                assert!(prop.values[0].is_nan(), "n={n}");
            }
        }
    }
}
