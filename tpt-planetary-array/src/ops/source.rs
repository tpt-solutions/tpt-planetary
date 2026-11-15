//! In-memory expression leaves.

use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::grid::{c_strides, num_elements};
use crate::lazy::{Block, Expr, NodeOp, Region};
use crate::ops::nd::{element_count, empty_block, for_each_index, is_empty_region};

/// A leaf holding values already in memory. NaN elements are invalid.
#[derive(Debug)]
struct InMemory {
    shape: Vec<u64>,
    strides: Vec<u64>,
    values: Vec<f64>,
}

impl NodeOp for InMemory {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let n = element_count(&region.count);
        let mut values = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        for_each_index(&region.count, |idx| {
            let mut off = 0usize;
            for (d, &i) in idx.iter().enumerate() {
                let pos = region.start[d] + i * region.step[d];
                off += (pos * self.strides[d]) as usize;
            }
            let v = self.values[off];
            values.push(v);
            valid.push(!v.is_nan());
        });
        Ok(Block { values, valid })
    }
}

impl Expr {
    /// Creates a leaf from row-major (C-order) values held in memory.
    ///
    /// Returns an error if `values.len()` does not equal the product of
    /// `shape`. NaN elements are treated as invalid by every reduction.
    pub fn from_values(shape: Vec<u64>, values: Vec<f64>) -> Result<Self> {
        let expected = num_elements(&shape)? as usize;
        if values.len() != expected {
            return Err(Error::SizeMismatch {
                expected,
                actual: values.len(),
            });
        }
        let strides = c_strides(&shape);
        Ok(Expr::from_op(InMemory {
            shape,
            strides,
            values,
        }))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn from_values_checks_length_and_masks_nan() {
        assert!(Expr::from_values(vec![2, 2], vec![1.0; 3]).is_err());
        let e = Expr::from_values(vec![3], vec![1.0, f64::NAN, 3.0]).unwrap();
        assert_eq!(e.count().unwrap(), 2);
        assert_eq!(e.mean().unwrap(), Some(2.0));
    }
}
