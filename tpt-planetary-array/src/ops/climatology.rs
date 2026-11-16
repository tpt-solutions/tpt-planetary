//! Climatology and anomalies: a lazy group-by along one axis with a fixed
//! period.
//!
//! For monthly data with a calendar-independent annual cycle, use a period
//! of 12 along the time axis. [`Expr::climatology`] gives the mean of each
//! phase `p` over all full cycles, and [`Expr::anomaly`] subtracts that mean
//! from every element.
//!
//! Both operations are NaN-aware: invalid elements (fill-valued or NaN) are
//! skipped, and a phase with no valid element is an invalid NaN. A trailing
//! partial cycle is ignored.
//!
//! Memory: the climatology is streamed one cycle at a time, so it holds one
//! block of outputs plus one block of input. The anomaly node recomputes the
//! climatology of its own spatial slab for each block it evaluates (the mean
//! needs every cycle), so the total work grows with the number of blocks
//! along the time axis. Use a larger `block_elements` to reduce it.

use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::grid::c_strides;
use crate::lazy::{Block, Expr, NodeOp, Region};
use crate::ops::nd::{element_count, empty_block, for_each_index, is_empty_region};
use crate::ops::window::check_axis;

fn check_period(period: u64) -> Result<()> {
    if period == 0 {
        return Err(Error::InvalidSelection("period must be positive".into()));
    }
    Ok(())
}

/// Per-phase mean over full cycles. The output has the input shape with the
/// axis length replaced by `period`.
#[derive(Debug)]
struct Climatology {
    input: Expr,
    axis: usize,
    period: u64,
    cycles: u64,
    shape: Vec<u64>,
}

impl NodeOp for Climatology {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let ax = self.axis;
        let n = element_count(&region.count);
        let mut mean = vec![0.0f64; n];
        let mut count = vec![0u64; n];
        // Stream one cycle at a time: the source region for cycle `c` has the
        // same counts and steps as the output region, shifted by whole cycles.
        for c in 0..self.cycles {
            let mut src = region.clone();
            src.start[ax] = c * self.period + region.start[ax];
            let block = self.input.eval_region(&src)?;
            for ((m, k), (&x, &ok)) in mean
                .iter_mut()
                .zip(count.iter_mut())
                .zip(block.values.iter().zip(&block.valid))
            {
                if ok {
                    *k += 1;
                    // Running mean (Welford update for a single stream).
                    *m += (x - *m) / *k as f64;
                }
            }
        }
        let valid: Vec<bool> = count.iter().map(|&k| k > 0).collect();
        let values = mean
            .iter()
            .zip(&valid)
            .map(|(&m, &ok)| if ok { m } else { f64::NAN })
            .collect();
        Ok(Block { values, valid })
    }
}

/// Input minus the climatology broadcast along the axis: element `t` uses the
/// climatology of phase `t % period`.
#[derive(Debug)]
struct Anomaly {
    input: Expr,
    climatology: Expr,
    axis: usize,
    period: u64,
    shape: Vec<u64>,
}

impl NodeOp for Anomaly {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let ax = self.axis;
        let k = self.period;
        let block = self.input.eval_region(region)?;
        // Every phase of the climatology at the same spatial positions.
        let mut clim_region = region.clone();
        clim_region.start[ax] = 0;
        clim_region.count[ax] = k;
        clim_region.step[ax] = 1;
        let clim = self.climatology.eval_region(&clim_region)?;
        let clim_strides = c_strides(&clim_region.count);

        let n = element_count(&region.count);
        let mut values = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        let mut i = 0usize;
        for_each_index(&region.count, |idx| {
            let t = region.start[ax] + idx[ax] * region.step[ax];
            let phase = t % k;
            let mut off = 0usize;
            for (d, &j) in idx.iter().enumerate() {
                let j = if d == ax { phase } else { j };
                off += (j * clim_strides[d]) as usize;
            }
            let ok = block.valid[i] && clim.valid[off];
            values.push(if ok {
                block.values[i] - clim.values[off]
            } else {
                f64::NAN
            });
            valid.push(ok);
            i += 1;
        });
        Ok(Block { values, valid })
    }
}

impl Expr {
    /// Mean of each phase `p` in `0..period` along `axis`, over all full
    /// cycles of `period` elements. Output `p` is the mean of the valid
    /// inputs at indices `c * period + p` for every full cycle `c`.
    ///
    /// The output has the input shape with the axis length replaced by
    /// `period`. A phase with no valid input is an invalid NaN.
    ///
    /// ```
    /// use tpt_planetary_array::Expr;
    /// use tpt_planetary_array::ops::reduce::Reduction;
    ///
    /// // Two cycles of length 3: values 0..6.
    /// let e = Expr::from_values(vec![6], (0..6).map(f64::from).collect()).unwrap();
    /// let clim = e.climatology(0, 3).unwrap();
    /// let r = clim.reduce_axes(&[], Reduction::Sum, false).unwrap();
    /// assert_eq!(r.shape, vec![3]);
    /// assert_eq!(r.values, vec![1.5, 2.5, 3.5]);
    /// ```
    pub fn climatology(&self, axis: usize, period: u64) -> Result<Expr> {
        let mut shape = self.shape();
        check_axis(&shape, axis)?;
        check_period(period)?;
        let cycles = shape[axis] / period;
        shape[axis] = period;
        Ok(Expr::from_op(Climatology {
            input: self.clone(),
            axis,
            period,
            cycles,
            shape,
        }))
    }

    /// Anomaly along `axis`: each element minus the climatology of its phase
    /// (see [`Expr::climatology`]). The output has the input shape.
    pub fn anomaly(&self, axis: usize, period: u64) -> Result<Expr> {
        let shape = self.shape();
        check_axis(&shape, axis)?;
        check_period(period)?;
        let climatology = self.climatology(axis, period)?;
        Ok(Expr::from_op(Anomaly {
            input: self.clone(),
            climatology,
            axis,
            period,
            shape,
        }))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::array::Array;
    use crate::dtype::DType;
    use crate::fill::FillValue;
    use crate::lazy::EvalOptions;
    use crate::meta::{ArrayMetadata, ZarrFormat};
    use crate::ops::reduce::Reduction;
    use crate::selection::Selection;
    use crate::store::MemoryStore;
    use alloc::sync::Arc;

    const K: u64 = 12;
    const CYCLES: u64 = 5;
    const SPATIAL: u64 = 3;

    /// Materialises an expression, row-major, with no masking.
    fn materialise(e: &Expr) -> Vec<f64> {
        e.reduce_axes(&[], Reduction::Sum, false).unwrap().values
    }

    /// `(t, s)` in a `[60, 3]` field: `phase + 10 * cycle + s`, where
    /// `phase = t % 12` and `cycle = t / 12`.
    fn field(t: u64, s: u64) -> f64 {
        (t % K) as f64 + 10.0 * (t / K) as f64 + s as f64
    }

    fn analytic_field() -> Expr {
        let n = (K * CYCLES) as usize;
        let mut values = Vec::with_capacity(n * SPATIAL as usize);
        for t in 0..K * CYCLES {
            for s in 0..SPATIAL {
                values.push(field(t, s));
            }
        }
        Expr::from_values(vec![K * CYCLES, SPATIAL], values).unwrap()
    }

    #[test]
    fn climatology_of_linear_cycle_field_is_analytic() {
        let clim = analytic_field().climatology(0, K).unwrap();
        assert_eq!(clim.shape(), vec![K, SPATIAL]);
        let got = materialise(&clim);
        // Mean cycle index over 0..5 is 2, so the climatology is phase + 20 + s.
        for p in 0..K {
            for s in 0..SPATIAL {
                let g = got[(p * SPATIAL + s) as usize];
                let expected = p as f64 + 10.0 * 2.0 + s as f64;
                assert!(
                    (g - expected).abs() < 1e-9,
                    "p={p} s={s}: {g} vs {expected}"
                );
            }
        }
    }

    #[test]
    fn anomaly_removes_the_climatology_and_has_zero_phase_means() {
        let field_expr = analytic_field();
        let anom = field_expr.anomaly(0, K).unwrap();
        assert_eq!(anom.shape(), vec![K * CYCLES, SPATIAL]);
        let values = materialise(&anom);
        for t in 0..K * CYCLES {
            for s in 0..SPATIAL {
                let v = values[(t * SPATIAL + s) as usize];
                let expected = 10.0 * ((t / K) as f64 - 2.0);
                assert!((v - expected).abs() < 1e-9, "t={t} s={s}: {v}");
            }
        }
        // The climatology of the anomaly is zero for every phase.
        let zero_mean = anom.climatology(0, K).unwrap();
        assert!(materialise(&zero_mean).iter().all(|v| v.abs() < 1e-9));
    }

    #[test]
    fn invalid_elements_are_skipped_and_empty_phases_are_nan() {
        let mut values: Vec<f64> = Vec::new();
        for t in 0..K * CYCLES {
            for s in 0..SPATIAL {
                let mut v = field(t, s);
                // Element (t=0, s=0) is NaN: phase 0 uses cycles 1..5 only.
                if t == 0 && s == 0 {
                    v = f64::NAN;
                }
                // Phase 1 has no valid element at spatial index 1.
                if t % K == 1 && s == 1 {
                    v = f64::NAN;
                }
                values.push(v);
            }
        }
        let e = Expr::from_values(vec![K * CYCLES, SPATIAL], values).unwrap();
        let got = materialise(&e.climatology(0, K).unwrap());
        // Phase 0, spatial 0: mean of 10, 20, 30, 40 (cycles 1..=4).
        assert!((got[0] - 25.0).abs() < 1e-9);
        // Phase 1, spatial 1: every cycle is NaN.
        assert!(got[(SPATIAL + 1) as usize].is_nan());
        // Other entries are unaffected.
        assert!((got[(SPATIAL + 2) as usize] - (1.0 + 20.0 + 2.0)).abs() < 1e-9);

        // The NaN input (t=0, s=0) and the NaN climatology at phase 1, s=1
        // (t=13, s=1) both give invalid anomalies.
        let anom = materialise(&e.anomaly(0, K).unwrap());
        assert!(anom[0].is_nan());
        assert!(anom[(13 * SPATIAL + 1) as usize].is_nan());
        assert!(anom[(25 * SPATIAL + 1) as usize].is_nan());
        // Phase 2 at the same spatial index is valid.
        assert!(anom[(14 * SPATIAL + 1) as usize].is_finite());
    }

    #[test]
    fn trailing_partial_cycle_is_ignored() {
        // Length 14 with period 12: the last two elements form a partial cycle.
        let mut values: Vec<f64> = (0..14).map(f64::from).collect();
        values[12] = 1e9;
        values[13] = 1e9;
        let e = Expr::from_values(vec![14], values).unwrap();
        let got = materialise(&e.climatology(0, K).unwrap());
        assert_eq!(got, (0..12).map(f64::from).collect::<Vec<_>>());
    }

    #[test]
    fn period_longer_than_the_series_gives_all_nan() {
        let e = Expr::from_values(vec![5], vec![1.0; 5]).unwrap();
        let clim = e.climatology(0, 12).unwrap();
        assert_eq!(clim.shape(), vec![12]);
        assert!(materialise(&clim).iter().all(|v| v.is_nan()));
    }

    #[test]
    fn result_is_independent_of_block_size() {
        let e = analytic_field();
        let clim = e.climatology(0, K).unwrap();
        let big = clim
            .reduce_axes_with(
                &[],
                Reduction::Sum,
                false,
                EvalOptions {
                    block_elements: 1 << 20,
                    workers: 1,
                },
            )
            .unwrap();
        let small = clim
            .reduce_axes_with(
                &[],
                Reduction::Sum,
                false,
                EvalOptions {
                    block_elements: 5,
                    workers: 1,
                },
            )
            .unwrap();
        assert_eq!(big.values, small.values);
        let anom = e.anomaly(0, K).unwrap();
        let a_small = anom
            .reduce_axes_with(
                &[],
                Reduction::Sum,
                false,
                EvalOptions {
                    block_elements: 7,
                    workers: 1,
                },
            )
            .unwrap();
        let a_big = materialise(&anom);
        for (x, y) in a_small.values.iter().zip(&a_big) {
            assert!((x - y).abs() < 1e-12);
        }
    }

    #[test]
    fn chunked_array_with_fill_value_matches_in_memory_values() {
        let store = Arc::new(MemoryStore::new());
        let meta = ArrayMetadata::new(
            ZarrFormat::V2,
            vec![K * CYCLES, SPATIAL],
            vec![7, 2],
            DType::FLOAT64,
        )
        .unwrap()
        .with_fill_value(FillValue::Float(-9999.0));
        let array = Array::create(store, "t", meta).unwrap();
        let mut values = Vec::new();
        for t in 0..K * CYCLES {
            for s in 0..SPATIAL {
                values.push(if t == 24 { -9999.0 } else { field(t, s) });
            }
        }
        array
            .write_f64(&[Selection::ALL, Selection::ALL], &values)
            .unwrap();
        let from_array = materialise(&Expr::from_array(array).climatology(0, K).unwrap());
        // Phase 0 at cycle 2 is filled, so the mean is over cycles 0, 1, 3, 4.
        let expected0 = (0.0 + 10.0 + 30.0 + 40.0) / 4.0;
        assert!((from_array[0] - expected0).abs() < 1e-9);
        // Phase 1 is untouched by the fill: mean of 1 + 10c + s over c.
        assert!((from_array[SPATIAL as usize + 1] - (1.0 + 20.0 + 1.0)).abs() < 1e-9);
    }

    #[test]
    fn invalid_arguments_are_rejected() {
        let e = analytic_field();
        assert!(e.climatology(0, 0).is_err());
        assert!(e.climatology(2, 12).is_err());
        assert!(e.anomaly(0, 0).is_err());
        assert!(e.anomaly(5, 12).is_err());
    }
}
