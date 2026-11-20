//! Group-by aggregation along one axis with integer labels.
//!
//! [`Expr::group_mean`] averages the elements of an axis that share a label.
//! Labels are supplied by the caller, one per index along the axis, so any
//! grouping key works: calendar month, season, year, or a region id. For
//! example, to get monthly climatology from monthly-sampled data, pass the
//! calendar month (0 to 11) of each time step.
//!
//! The operation is NaN-aware, like [`Expr::climatology`]: invalid elements are
//! skipped, and a group with no valid element is an invalid NaN. Labels at or
//! above the number of groups are ignored.
//!
//! Evaluation streams one input index along the axis at a time, so the memory
//! use is one output block plus one input slab. Each input index is evaluated
//! once per output block whose groups include its label.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::grid::c_strides;
use crate::lazy::{Block, Expr, NodeOp, Region};
use crate::ops::nd::{element_count, empty_block, for_each_index, is_empty_region};
use crate::ops::window::check_axis;

/// Mean of the inputs in each group along `axis`.
#[derive(Debug)]
struct GroupMean {
    input: Expr,
    axis: usize,
    labels: Vec<u32>,
    shape: Vec<u64>,
}

impl NodeOp for GroupMean {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let ax = self.axis;
        let out_counts = &region.count;
        let n = element_count(out_counts);
        let out_strides = c_strides(out_counts);
        let mut sum = vec![0.0f64; n];
        let mut count = vec![0u64; n];

        // Output index along the axis is `start + k * step`. Map each label to
        // its output position k, if it is one of the requested groups.
        let first = region.start[ax];
        let step = region.step[ax];
        let last = first + (out_counts[ax] - 1) * step;
        for (i, &label) in self.labels.iter().enumerate() {
            let g = u64::from(label);
            if g < first || g > last || !(g - first).is_multiple_of(step) {
                continue;
            }
            let k = (g - first) / step;
            // One input index along the axis, across the whole spatial region.
            let mut src = region.clone();
            src.start[ax] = i as u64;
            src.count[ax] = 1;
            src.step[ax] = 1;
            let block = self.input.eval_region(&src)?;
            let mut in_counts = out_counts.clone();
            in_counts[ax] = 1;
            let mut j = 0usize;
            for_each_index(&in_counts, |idx| {
                let mut off = 0usize;
                for (d, &v) in idx.iter().enumerate() {
                    let v = if d == ax { k } else { v };
                    off += (v * out_strides[d]) as usize;
                }
                if block.valid[j] {
                    count[off] += 1;
                    sum[off] += block.values[j];
                }
                j += 1;
            });
        }

        let mut values = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        for (&s, &c) in sum.iter().zip(&count) {
            let ok = c > 0;
            values.push(if ok { s / c as f64 } else { f64::NAN });
            valid.push(ok);
        }
        Ok(Block { values, valid })
    }
}

impl Expr {
    /// Mean of the valid inputs in each group along `axis`.
    ///
    /// `labels` has one entry per index along `axis` (its length must equal
    /// that axis length). Output index `g` (for `g` in `0..groups`) is the mean
    /// of all inputs whose label is `g`. The output has the input shape with
    /// the axis length replaced by `groups`.
    ///
    /// ```
    /// use tpt_planetary_array::lazy::Expr;
    /// // Two groups over six points: label 0 for the first three, 1 for the rest.
    /// let e = Expr::from_values(vec![6], vec![1.0, 2.0, 3.0, 10.0, 20.0, 30.0]).unwrap();
    /// let means = e.group_mean(0, &[0, 0, 0, 1, 1, 1], 2).unwrap();
    /// assert_eq!(means.shape(), vec![2]);
    /// ```
    pub fn group_mean(&self, axis: usize, labels: &[u32], groups: u64) -> Result<Expr> {
        let mut shape = self.shape();
        check_axis(&shape, axis)?;
        if labels.len() as u64 != shape[axis] {
            return Err(Error::InvalidSelection(format!(
                "{} labels given for an axis of length {}",
                labels.len(),
                shape[axis]
            )));
        }
        if groups == 0 {
            return Err(Error::InvalidSelection("groups must be positive".into()));
        }
        shape[axis] = groups;
        Ok(Expr::from_op(GroupMean {
            input: self.clone(),
            axis,
            labels: labels.to_vec(),
            shape,
        }))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::ops::reduce::Reduction;

    fn materialise(e: &Expr) -> Vec<f64> {
        e.reduce_axes(&[], Reduction::Sum, false).unwrap().values
    }

    #[test]
    fn groups_along_axis_zero() {
        // Shape [6, 2]: row t holds (t, 10 + t).
        let mut v = Vec::new();
        for t in 0..6 {
            v.push(t as f64);
            v.push(10.0 + t as f64);
        }
        let e = Expr::from_values(vec![6, 2], v).unwrap();
        // Labels: even rows are group 0, odd rows are group 1.
        let labels = [0, 1, 0, 1, 0, 1];
        let g = e.group_mean(0, &labels, 2).unwrap();
        assert_eq!(g.shape(), vec![2, 2]);
        // Group 0 rows 0, 2, 4: mean 2 and 12. Group 1 rows 1, 3, 5: mean 3 and 13.
        assert_eq!(materialise(&g), vec![2.0, 12.0, 3.0, 13.0]);
    }

    #[test]
    fn groups_along_last_axis_and_nan_skipping() {
        // Shape [2, 4]; NaN entries are skipped.
        let e = Expr::from_values(
            vec![2, 4],
            vec![1.0, f64::NAN, 3.0, 4.0, 5.0, 6.0, f64::NAN, 8.0],
        )
        .unwrap();
        let g = e.group_mean(1, &[0, 0, 1, 1], 2).unwrap();
        assert_eq!(g.shape(), vec![2, 2]);
        let v = materialise(&g);
        assert_eq!(v[0], 1.0); // row 0, group 0: only 1.0 is valid
        assert_eq!(v[1], 3.5); // row 0, group 1: (3 + 4) / 2
        assert_eq!(v[2], 5.5); // row 1, group 0: (5 + 6) / 2
        assert_eq!(v[3], 8.0);
    }

    #[test]
    fn empty_group_is_invalid_and_out_of_range_labels_are_ignored() {
        let e = Expr::from_values(vec![4], vec![1.0, 2.0, 3.0, 4.0]).unwrap();
        // Group 1 has no members; label 7 is beyond the two groups.
        let g = e.group_mean(0, &[0, 0, 7, 0], 2).unwrap();
        let block = g
            .eval_region(&Region {
                start: vec![0],
                count: vec![2],
                step: vec![1],
            })
            .unwrap();
        assert_eq!(block.valid, vec![true, false]);
        assert!((block.values[0] - 7.0 / 3.0).abs() < 1e-12);
        assert!(block.values[1].is_nan());
    }

    #[test]
    fn monthly_grouping_matches_climatology_for_regular_labels() {
        // Two years of monthly data: value = month + 100 * year.
        let mut v = Vec::new();
        let mut labels = Vec::new();
        for t in 0..24u32 {
            v.push((t % 12) as f64 + 100.0 * (t / 12) as f64);
            labels.push(t % 12);
        }
        let e = Expr::from_values(vec![24], v).unwrap();
        let g = e.group_mean(0, &labels, 12).unwrap();
        let got = materialise(&g);
        let clim = e.climatology(0, 12).unwrap();
        let expected = materialise(&clim);
        for (a, b) in got.iter().zip(&expected) {
            assert!((a - b).abs() < 1e-9, "{a} vs {b}");
        }
    }

    #[test]
    fn rejects_bad_arguments() {
        let e = Expr::from_values(vec![3], vec![1.0, 2.0, 3.0]).unwrap();
        assert!(e.group_mean(0, &[0, 1], 2).is_err());
        assert!(e.group_mean(0, &[0, 1, 0], 0).is_err());
        assert!(e.group_mean(1, &[0, 1, 0], 2).is_err());
    }
}
