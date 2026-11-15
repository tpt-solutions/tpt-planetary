//! NumPy-style broadcasting for N-dimensional shapes.
//!
//! Shapes are aligned at their trailing dimensions. Two dimensions are
//! compatible when they are equal or when one of them is 1; a dimension of
//! length 1 is repeated to match the other. Missing leading dimensions count
//! as 1.

use alloc::format;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::grid::c_strides;
use crate::lazy::{Block, Region};
use crate::ops::nd::{element_count, for_each_index};

/// Length of dimension `d` of a shape of rank `out_rank`, after left-padding
/// `shape` with ones.
fn padded_dim(shape: &[u64], out_rank: usize, d: usize) -> u64 {
    let off = out_rank - shape.len();
    if d < off { 1 } else { shape[d - off] }
}

/// The broadcast shape of several shapes, or an error naming the first pair
/// of incompatible dimensions.
///
/// With no shapes the result is the empty (scalar) shape.
///
/// ```
/// use tpt_planetary_array::ops::broadcast::broadcast_shapes;
///
/// assert_eq!(broadcast_shapes(&[&[3, 1], &[1, 4]]).unwrap(), vec![3, 4]);
/// assert_eq!(broadcast_shapes(&[&[2, 3, 4], &[4]]).unwrap(), vec![2, 3, 4]);
/// assert!(broadcast_shapes(&[&[2], &[3]]).is_err());
/// ```
pub fn broadcast_shapes(shapes: &[&[u64]]) -> Result<Vec<u64>> {
    let rank = shapes.iter().map(|s| s.len()).max().unwrap_or(0);
    let mut out = Vec::with_capacity(rank);
    for d in 0..rank {
        let mut dim = 1u64;
        for shape in shapes {
            let len = padded_dim(shape, rank, d);
            if len == dim || len == 1 {
                continue;
            }
            if dim == 1 {
                dim = len;
            } else {
                return Err(Error::InvalidSelection(format!(
                    "shapes {shapes:?} cannot be broadcast together (dimension {d})"
                )));
            }
        }
        out.push(dim);
    }
    Ok(out)
}

/// Maps an index of a broadcast output to the index of an operand with shape
/// `src_shape`. The output index has the rank of the broadcast result; leading
/// dimensions missing from the operand are dropped and dimensions of length 1
/// map to 0.
///
/// Returns an error if the operand has more dimensions than the index.
///
/// ```
/// use tpt_planetary_array::ops::broadcast::broadcast_index;
///
/// // Output index (2, 1, 3) into an operand of shape (1, 3).
/// assert_eq!(broadcast_index(&[2, 1, 3], &[1, 3]).unwrap(), vec![0, 3]);
/// ```
pub fn broadcast_index(index: &[u64], src_shape: &[u64]) -> Result<Vec<u64>> {
    if src_shape.len() > index.len() {
        return Err(Error::InvalidSelection(
            "operand has more dimensions than the broadcast index".into(),
        ));
    }
    let off = index.len() - src_shape.len();
    Ok(src_shape
        .iter()
        .enumerate()
        .map(|(j, &len)| if len == 1 { 0 } else { index[off + j] })
        .collect())
}

/// The region of an operand that is needed to produce `region` of a broadcast
/// output. Dimensions of length 1 in the operand are read at index 0 only.
pub(crate) fn operand_region(region: &Region, src_shape: &[u64]) -> Region {
    let off = region.start.len() - src_shape.len();
    let mut start = Vec::with_capacity(src_shape.len());
    let mut count = Vec::with_capacity(src_shape.len());
    let mut step = Vec::with_capacity(src_shape.len());
    for (j, &len) in src_shape.iter().enumerate() {
        let d = off + j;
        if len == 1 {
            start.push(0);
            count.push(1);
            step.push(1);
        } else {
            start.push(region.start[d]);
            count.push(region.count[d]);
            step.push(region.step[d]);
        }
    }
    Region { start, count, step }
}

/// Expands an operand block (with counts `src_counts`, one per output
/// dimension, 1 where broadcast) to the output counts `out_counts`.
pub(crate) fn expand_block(src: Block, src_counts: &[u64], out_counts: &[u64]) -> Block {
    if src_counts == out_counts {
        return src;
    }
    let strides = c_strides(src_counts);
    let n = element_count(out_counts);
    let mut values = Vec::with_capacity(n);
    let mut valid = Vec::with_capacity(n);
    for_each_index(out_counts, |idx| {
        let mut off = 0usize;
        for (d, &i) in idx.iter().enumerate() {
            if src_counts[d] != 1 {
                off += (i * strides[d]) as usize;
            }
        }
        values.push(src.values[off]);
        valid.push(src.valid[off]);
    });
    Block { values, valid }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    #[test]
    fn shapes_follow_numpy_rules() {
        assert_eq!(broadcast_shapes(&[&[3, 1], &[1, 4]]).unwrap(), vec![3, 4]);
        assert_eq!(
            broadcast_shapes(&[&[5, 1, 6], &[3, 1]]).unwrap(),
            vec![5, 3, 6]
        );
        assert_eq!(broadcast_shapes(&[&[], &[2, 2]]).unwrap(), vec![2, 2]);
        assert_eq!(broadcast_shapes(&[&[4, 0], &[1, 0]]).unwrap(), vec![4, 0]);
        assert_eq!(broadcast_shapes(&[]).unwrap(), Vec::<u64>::new());
        assert!(broadcast_shapes(&[&[2, 3], &[4, 3]]).is_err());
        assert!(broadcast_shapes(&[&[2], &[3]]).is_err());
    }

    #[test]
    fn index_mapping_matches_reference() {
        let out = [2u64, 3, 4];
        let src = [3u64, 1];
        // Reference: an operand of shape (3, 1) broadcast to (2, 3, 4) repeats
        // along the first and last output axes.
        for i in 0..2 {
            for j in 0..3 {
                for k in 0..4 {
                    assert_eq!(broadcast_index(&[i, j, k], &src).unwrap(), vec![j, 0]);
                }
            }
        }
        assert!(broadcast_index(&out, &[1, 2, 3, 4]).is_err());
    }

    #[test]
    fn operand_region_pins_broadcast_dims() {
        let region = Region {
            start: vec![0, 1, 2],
            count: vec![2, 2, 3],
            step: vec![1, 1, 1],
        };
        let r = operand_region(&region, &[3, 1]);
        assert_eq!(r.start, vec![1, 0]);
        assert_eq!(r.count, vec![2, 1]);
    }
}
