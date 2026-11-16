//! Rolling-window means and block-averaged resampling along one axis.
//!
//! Both operations stream their input: each output block reads only the
//! input it needs (the window history for rolling means, the blocks being
//! averaged for resampling).

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::grid::c_strides;
use crate::lazy::{Block, Expr, NodeOp, Region};
use crate::ops::nd::{element_count, empty_block, for_each_index, is_empty_region};

pub(crate) fn check_axis(shape: &[u64], axis: usize) -> Result<()> {
    if axis >= shape.len() {
        return Err(Error::InvalidSelection(alloc::format!(
            "axis {axis} out of range for {} dimensions",
            shape.len()
        )));
    }
    Ok(())
}

/// Trailing rolling mean. Output `i` along the axis is the mean of inputs
/// `i - window + 1 ..= i`; the first `window - 1` outputs are invalid NaN, and
/// any window containing an invalid element yields an invalid NaN.
#[derive(Debug)]
struct RollingMean {
    input: Expr,
    axis: usize,
    window: u64,
    shape: Vec<u64>,
}

impl NodeOp for RollingMean {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let ax = self.axis;
        let w = self.window;
        let s = region.start[ax];
        let st = region.step[ax];
        let last = s + (region.count[ax] - 1) * st;
        // Input history needed by the first output of the region.
        let in_first = s.saturating_sub(w - 1);
        let mut in_region = region.clone();
        in_region.start[ax] = in_first;
        in_region.count[ax] = last - in_first + 1;
        in_region.step[ax] = 1;
        let block = self.input.eval_region(&in_region)?;
        let in_strides = c_strides(&in_region.count);

        let n = element_count(&region.count);
        let mut values = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        for_each_index(&region.count, |idx| {
            let o = s + idx[ax] * st;
            if o + 1 < w {
                values.push(f64::NAN);
                valid.push(false);
                return;
            }
            let mut base = 0u64;
            for (d, &i) in idx.iter().enumerate() {
                if d != ax {
                    base += i * in_strides[d];
                }
            }
            let first_pos = o + 1 - w - in_first;
            let mut sum = 0.0;
            let mut ok = true;
            for k in 0..w {
                let off = (base + (first_pos + k) * in_strides[ax]) as usize;
                if !block.valid[off] {
                    ok = false;
                    break;
                }
                sum += block.values[off];
            }
            values.push(if ok { sum / w as f64 } else { f64::NAN });
            valid.push(ok);
        });
        Ok(Block { values, valid })
    }
}

/// Block averaging along an axis. Output `j` is the mean of valid inputs in
/// `j * block .. (j + 1) * block`. A trailing partial block is dropped, so
/// the output length is `len / block`. Blocks with no valid input are invalid
/// NaN.
#[derive(Debug)]
struct BlockMean {
    input: Expr,
    axis: usize,
    block: u64,
    shape: Vec<u64>,
}

impl NodeOp for BlockMean {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let ax = self.axis;
        let k = self.block;
        let s = region.start[ax];
        let st = region.step[ax];
        let last = s + (region.count[ax] - 1) * st;
        // Input covers output positions s..=last, i.e. inputs s*k .. (last+1)*k.
        let in_first = s * k;
        let mut in_region = region.clone();
        in_region.start[ax] = in_first;
        in_region.count[ax] = (last + 1) * k - in_first;
        in_region.step[ax] = 1;
        let block = self.input.eval_region(&in_region)?;
        let in_strides = c_strides(&in_region.count);

        let n = element_count(&region.count);
        let mut values = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        for_each_index(&region.count, |idx| {
            let group = idx[ax] * st * k;
            let mut base = 0u64;
            for (d, &i) in idx.iter().enumerate() {
                if d != ax {
                    base += i * in_strides[d];
                }
            }
            let mut sum = 0.0;
            let mut cnt = 0u64;
            for i in 0..k {
                let off = (base + (group + i) * in_strides[ax]) as usize;
                if block.valid[off] {
                    sum += block.values[off];
                    cnt += 1;
                }
            }
            if cnt == 0 {
                values.push(f64::NAN);
                valid.push(false);
            } else {
                values.push(sum / cnt as f64);
                valid.push(true);
            }
        });
        Ok(Block { values, valid })
    }
}

impl Expr {
    /// Trailing rolling mean along `axis` over `window` elements.
    ///
    /// Outputs before the first full window are invalid (NaN), and a window
    /// containing an invalid element is invalid. The output has the input
    /// shape.
    pub fn rolling_mean(&self, axis: usize, window: u64) -> Result<Expr> {
        let shape = self.shape();
        check_axis(&shape, axis)?;
        if window == 0 {
            return Err(Error::InvalidSelection("window must be positive".into()));
        }
        Ok(Expr::from_op(RollingMean {
            input: self.clone(),
            axis,
            window,
            shape,
        }))
    }

    /// Averages consecutive blocks of `block` elements along `axis`
    /// (resampling). The output length along `axis` is `len / block`; a
    /// trailing partial block is dropped.
    pub fn block_mean(&self, axis: usize, block: u64) -> Result<Expr> {
        let mut shape = self.shape();
        check_axis(&shape, axis)?;
        if block == 0 {
            return Err(Error::InvalidSelection("block must be positive".into()));
        }
        shape[axis] /= block;
        Ok(Expr::from_op(BlockMean {
            input: self.clone(),
            axis,
            block,
            shape,
        }))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::ops::reduce::Reduction;

    /// Materialises every element of an expression, row-major.
    fn materialise(e: &Expr) -> Vec<f64> {
        e.reduce_axes(&[], Reduction::Sum, false).unwrap().values
    }

    fn assert_close(got: &[f64], expected: &[f64]) {
        assert_eq!(got.len(), expected.len());
        for (g, x) in got.iter().zip(expected) {
            if x.is_nan() {
                assert!(g.is_nan(), "expected NaN, got {g}");
            } else {
                assert!((g - x).abs() < 1e-12, "{g} vs {x}");
            }
        }
    }

    #[test]
    fn rolling_mean_along_last_axis() {
        // Two rows of six; window 3.
        let data = vec![
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0,
        ];
        let e = Expr::from_values(vec![2, 6], data).unwrap();
        let r = e.rolling_mean(1, 3).unwrap();
        assert_eq!(r.shape(), vec![2, 6]);
        let nan = f64::NAN;
        assert_close(
            &materialise(&r),
            &[
                nan, nan, 2.0, 3.0, 4.0, 5.0, nan, nan, 20.0, 30.0, 40.0, 50.0,
            ],
        );
        assert_eq!(r.count().unwrap(), 8);
    }

    #[test]
    fn rolling_mean_along_leading_axis_with_invalid_input() {
        // Shape (4, 2): window 2 along axis 0; a NaN poisons the windows
        // that contain it.
        let data = vec![1.0, 10.0, f64::NAN, 30.0, 5.0, 50.0, 7.0, 70.0];
        let e = Expr::from_values(vec![4, 2], data).unwrap();
        let r = e.rolling_mean(0, 2).unwrap();
        let nan = f64::NAN;
        assert_close(
            &materialise(&r),
            &[nan, nan, nan, 20.0, nan, 40.0, 6.0, 60.0],
        );
    }

    #[test]
    fn rolling_mean_matches_reference_under_small_blocks() {
        let n = 50usize;
        let data: Vec<f64> = (0..n).map(|i| ((i * 7) % 13) as f64).collect();
        let e = Expr::from_values(vec![5, 10], data.clone()).unwrap();
        let r = e.rolling_mean(1, 4).unwrap();
        let mut expected = Vec::new();
        for row in 0..5 {
            for col in 0..10 {
                if col < 3 {
                    expected.push(f64::NAN);
                } else {
                    let s: f64 = (0..4).map(|k| data[row * 10 + col - k]).sum();
                    expected.push(s / 4.0);
                }
            }
        }
        let (stats, exec) = r
            .stats_with(crate::lazy::EvalOptions {
                block_elements: 7,
                workers: 1,
            })
            .unwrap();
        assert!(exec.blocks > 1);
        assert_eq!(stats.count, 5 * 7);
        assert_close(&materialise(&r), &expected);
    }

    #[test]
    fn block_mean_resamples_along_axis() {
        // Shape (6, 2), blocks of 2 along axis 0 -> shape (3, 2).
        let data = vec![
            1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0, 11.0, 12.0,
        ];
        let e = Expr::from_values(vec![6, 2], data).unwrap();
        let r = e.block_mean(0, 2).unwrap();
        assert_eq!(r.shape(), vec![3, 2]);
        assert_close(&materialise(&r), &[2.0, 3.0, 6.0, 7.0, 10.0, 11.0]);
    }

    #[test]
    fn block_mean_drops_partial_block_and_skips_invalid() {
        let data = vec![1.0, f64::NAN, 3.0, 5.0, 7.0, 9.0, 100.0];
        let e = Expr::from_values(vec![7], data).unwrap();
        let r = e.block_mean(0, 2).unwrap();
        assert_eq!(r.shape(), vec![3]);
        // Blocks: (1, NaN) -> 1; (3, 5) -> 4; (7, 9) -> 8; trailing 100 dropped.
        assert_close(&materialise(&r), &[1.0, 4.0, 8.0]);
    }

    #[test]
    fn block_mean_result_is_independent_of_block_size() {
        let data: Vec<f64> = (0..48).map(|i| f64::from(i % 9)).collect();
        let e = Expr::from_values(vec![8, 6], data).unwrap();
        let r = e.block_mean(0, 4).unwrap();
        let (a, _) = r
            .stats_with(crate::lazy::EvalOptions {
                block_elements: 1000,
                workers: 1,
            })
            .unwrap();
        let (b, exec) = r
            .stats_with(crate::lazy::EvalOptions {
                block_elements: 5,
                workers: 1,
            })
            .unwrap();
        assert!(exec.max_block_elements <= 5);
        assert!((a.mean - b.mean).abs() < 1e-12);
        assert_eq!(a.count, b.count);
    }

    #[test]
    fn invalid_arguments_are_rejected() {
        let e = Expr::from_values(vec![3], vec![1.0, 2.0, 3.0]).unwrap();
        assert!(e.rolling_mean(1, 2).is_err());
        assert!(e.rolling_mean(0, 0).is_err());
        assert!(e.block_mean(0, 0).is_err());
    }
}
