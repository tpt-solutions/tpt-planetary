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

/// The aggregation of a group-by.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupOp {
    /// Sum of the valid inputs; zero for an empty group.
    Sum,
    /// Mean of the valid inputs; NaN (invalid) for an empty group.
    Mean,
    /// Smallest valid input; NaN (invalid) for an empty group.
    Min,
    /// Largest valid input; NaN (invalid) for an empty group.
    Max,
    /// Number of valid inputs; zero for an empty group.
    Count,
}

/// One aggregation along `axis`.
#[derive(Debug)]
struct GroupReduce {
    input: Expr,
    axis: usize,
    labels: Vec<u32>,
    op: GroupOp,
    shape: Vec<u64>,
}

/// Running sum, count, minimum and maximum of one output cell.
#[derive(Debug, Clone, Copy)]
struct Cell {
    sum: f64,
    count: u64,
    min: f64,
    max: f64,
}

impl Cell {
    const EMPTY: Cell = Cell {
        sum: 0.0,
        count: 0,
        min: f64::INFINITY,
        max: f64::NEG_INFINITY,
    };

    fn push(&mut self, v: f64) {
        self.count += 1;
        self.sum += v;
        self.min = self.min.min(v);
        self.max = self.max.max(v);
    }

    /// The output value and whether it is valid.
    fn finish(&self, op: GroupOp) -> (f64, bool) {
        let empty = self.count == 0;
        match op {
            GroupOp::Sum => (self.sum, true),
            GroupOp::Count => (self.count as f64, true),
            GroupOp::Mean => {
                if empty {
                    (f64::NAN, false)
                } else {
                    (self.sum / self.count as f64, true)
                }
            }
            GroupOp::Min => {
                if empty {
                    (f64::NAN, false)
                } else {
                    (self.min, true)
                }
            }
            GroupOp::Max => {
                if empty {
                    (f64::NAN, false)
                } else {
                    (self.max, true)
                }
            }
        }
    }
}

impl NodeOp for GroupReduce {
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
        let mut cells = vec![Cell::EMPTY; n];

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
                    cells[off].push(block.values[j]);
                }
                j += 1;
            });
        }

        let mut values = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        for cell in &cells {
            let (v, ok) = cell.finish(self.op);
            values.push(v);
            valid.push(ok);
        }
        Ok(Block { values, valid })
    }
}

impl Expr {
    /// Aggregates the valid inputs in each group along `axis` with `op`.
    ///
    /// `labels` has one entry per index along `axis` (its length must equal
    /// that axis length). Output index `g` (for `g` in `0..groups`) is the
    /// aggregate of all inputs whose label is `g`. The output has the input
    /// shape with the axis length replaced by `groups`. Labels at or above
    /// `groups` are ignored.
    ///
    /// # Errors
    ///
    /// Fails if `axis` is out of range, the label count does not match the axis
    /// length, or `groups` is zero.
    pub fn group_reduce(
        &self,
        axis: usize,
        labels: &[u32],
        groups: u64,
        op: GroupOp,
    ) -> Result<Expr> {
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
        Ok(Expr::from_op(GroupReduce {
            input: self.clone(),
            axis,
            labels: labels.to_vec(),
            op,
            shape,
        }))
    }

    /// Mean of the valid inputs in each group along `axis` (see
    /// [`Expr::group_reduce`]). An empty group is an invalid NaN.
    ///
    /// ```
    /// use tpt_planetary_array::lazy::Expr;
    /// // Two groups over six points: label 0 for the first three, 1 for the rest.
    /// let e = Expr::from_values(vec![6], vec![1.0, 2.0, 3.0, 10.0, 20.0, 30.0]).unwrap();
    /// let means = e.group_mean(0, &[0, 0, 0, 1, 1, 1], 2).unwrap();
    /// assert_eq!(means.shape(), vec![2]);
    /// ```
    pub fn group_mean(&self, axis: usize, labels: &[u32], groups: u64) -> Result<Expr> {
        self.group_reduce(axis, labels, groups, GroupOp::Mean)
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

#[cfg(all(test, feature = "std"))]
mod reduce_tests {
    use super::*;

    /// A 5 x 4 field (axis 0 is the grouped axis) with holes, grouped into 3
    /// groups with label 3 ignored and group 2 left empty.
    fn field() -> (Vec<f64>, Vec<u32>) {
        let values: Vec<f64> = (0..20)
            .map(|i| {
                if i % 7 == 3 {
                    f64::NAN
                } else {
                    f64::from(i) * 0.5 - 4.0
                }
            })
            .collect();
        let labels = vec![0u32, 1, 3, 0, 1];
        (values, labels)
    }

    /// Brute-force aggregate of group `g` at column `c`.
    fn brute(values: &[f64], labels: &[u32], g: u32, c: usize, op: GroupOp) -> (f64, bool) {
        let mut cell = Cell::EMPTY;
        for (row, &label) in labels.iter().enumerate() {
            if label == g {
                let v = values[row * 4 + c];
                if !v.is_nan() {
                    cell.push(v);
                }
            }
        }
        cell.finish(op)
    }

    #[test]
    fn every_operation_matches_a_brute_force_group_aggregate() {
        let (values, labels) = field();
        let e = Expr::from_values(vec![5, 4], values.clone()).unwrap();
        for op in [
            GroupOp::Sum,
            GroupOp::Mean,
            GroupOp::Min,
            GroupOp::Max,
            GroupOp::Count,
        ] {
            let out = e.group_reduce(0, &labels, 3, op).unwrap();
            assert_eq!(out.shape(), vec![3, 4]);
            let got = out
                .eval_region(&Region {
                    start: vec![0, 0],
                    count: vec![3, 4],
                    step: vec![1, 1],
                })
                .unwrap();
            for g in 0..3u32 {
                for c in 0..4usize {
                    let (want, ok) = brute(&values, &labels, g, c, op);
                    let k = g as usize * 4 + c;
                    assert_eq!(got.valid[k], ok, "{op:?} group {g} col {c}: validity");
                    if ok {
                        assert_eq!(
                            got.values[k].to_bits(),
                            want.to_bits(),
                            "{op:?} group {g} col {c}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn empty_groups_follow_their_operation() {
        // Group 2 has no labels: Sum and Count are zero and valid; the rest are invalid.
        let e = Expr::from_values(vec![2], vec![1.0, 2.0]).unwrap();
        let region = Region {
            start: vec![0],
            count: vec![3],
            step: vec![1],
        };
        let sum = e.group_reduce(0, &[0, 1], 3, GroupOp::Sum).unwrap();
        let s = sum.eval_region(&region).unwrap();
        assert_eq!((s.values[2], s.valid[2]), (0.0, true));
        let min = e.group_reduce(0, &[0, 1], 3, GroupOp::Min).unwrap();
        let m = min.eval_region(&region).unwrap();
        assert!(!m.valid[2] && m.values[2].is_nan());
    }

    #[test]
    fn a_region_evaluated_in_parts_matches_the_whole() {
        let (values, labels) = field();
        let e = Expr::from_values(vec![5, 4], values).unwrap();
        let out = e.group_reduce(0, &labels, 3, GroupOp::Max).unwrap();
        let whole = out
            .eval_region(&Region {
                start: vec![0, 0],
                count: vec![3, 4],
                step: vec![1, 1],
            })
            .unwrap();
        for c in 0..4u64 {
            let part = out
                .eval_region(&Region {
                    start: vec![0, c],
                    count: vec![3, 1],
                    step: vec![1, 1],
                })
                .unwrap();
            for g in 0..3usize {
                let k = g * 4 + c as usize;
                assert_eq!(
                    part.values[g].to_bits(),
                    whole.values[k].to_bits(),
                    "group {g} col {c}"
                );
            }
        }
    }

    #[test]
    fn group_mean_is_the_mean_operation() {
        let (values, labels) = field();
        let e = Expr::from_values(vec![5, 4], values).unwrap();
        let mean = e.group_mean(0, &labels, 3).unwrap();
        let op = e.group_reduce(0, &labels, 3, GroupOp::Mean).unwrap();
        let region = Region {
            start: vec![0, 0],
            count: vec![3, 4],
            step: vec![1, 1],
        };
        let (a, b) = (
            mean.eval_region(&region).unwrap(),
            op.eval_region(&region).unwrap(),
        );
        assert_eq!(a.valid, b.valid);
        assert!(
            a.values
                .iter()
                .zip(&b.values)
                .all(|(x, y)| x.to_bits() == y.to_bits())
        );
    }
}
