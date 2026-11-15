//! Crate-internal helpers for N-dimensional index iteration over blocks.

use alloc::vec;
use alloc::vec::Vec;

use crate::lazy::{Block, Region};

/// Calls `f` with every multi-index of a block of the given `counts`, in C
/// order (last dimension fastest). Calls nothing if any count is zero.
pub(crate) fn for_each_index(counts: &[u64], mut f: impl FnMut(&[u64])) {
    if counts.iter().any(|&c| c == 0) {
        return;
    }
    let mut idx = vec![0u64; counts.len()];
    loop {
        f(&idx);
        let mut d = counts.len();
        loop {
            if d == 0 {
                return;
            }
            d -= 1;
            idx[d] += 1;
            if idx[d] < counts[d] {
                break;
            }
            idx[d] = 0;
        }
    }
}

/// Number of elements in a block with the given counts.
pub(crate) fn element_count(counts: &[u64]) -> usize {
    counts.iter().map(|&c| c as usize).product()
}

/// A block with no elements.
pub(crate) fn empty_block() -> Block {
    Block {
        values: Vec::new(),
        valid: Vec::new(),
    }
}

/// True if the region contains no elements.
pub(crate) fn is_empty_region(region: &Region) -> bool {
    region.count.iter().any(|&c| c == 0)
}
