//! Slicing and selection: half-open ranges with positive steps and integer
//! indices, resolved against a dimension length.

use alloc::format;
use alloc::vec::Vec;

use crate::error::{Error, Result};

/// A half-open range `start..stop` with a positive `step`, using Python
/// conventions: `None` means the dimension boundary and negative values count
/// from the end.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Slice {
    /// First index, or `None` for the beginning.
    pub start: Option<i64>,
    /// Exclusive end, or `None` for the end of the dimension.
    pub stop: Option<i64>,
    /// Positive step between selected indices.
    pub step: i64,
}

impl Slice {
    /// The whole dimension with step 1.
    pub const ALL: Slice = Slice {
        start: None,
        stop: None,
        step: 1,
    };

    /// Range `start..stop` with step 1.
    pub fn range(start: i64, stop: i64) -> Self {
        Self {
            start: Some(start),
            stop: Some(stop),
            step: 1,
        }
    }

    /// Returns a copy with the given positive step.
    pub fn with_step(mut self, step: i64) -> Self {
        self.step = step;
        self
    }
}

/// Selection along one dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Selection {
    /// A single integer index; the dimension is removed from the output.
    Index(i64),
    /// A range; the dimension is kept.
    Range(Slice),
}

impl Selection {
    /// Selects the whole dimension.
    pub const ALL: Selection = Selection::Range(Slice::ALL);

    /// Resolves this selection against a dimension of length `len`.
    pub fn resolve(&self, len: u64) -> Result<ResolvedDim> {
        match *self {
            Selection::Index(i) => {
                let idx = normalize_index(i, len)?;
                Ok(ResolvedDim {
                    start: idx,
                    step: 1,
                    count: 1,
                    kept: false,
                })
            }
            Selection::Range(s) => s.resolve(len),
        }
    }
}

impl Slice {
    /// Resolves the slice against a dimension of length `len`.
    pub fn resolve(&self, len: u64) -> Result<ResolvedDim> {
        if self.step <= 0 {
            return Err(Error::InvalidSelection(format!(
                "step must be positive, got {}",
                self.step
            )));
        }
        let len_i = len as i64;
        let clamp = |v: i64| -> i64 {
            let v = if v < 0 { v + len_i } else { v };
            v.clamp(0, len_i)
        };
        let start = self.start.map_or(0, clamp);
        let stop = self.stop.map_or(len_i, clamp);
        let step = self.step as u64;
        let count = if stop > start {
            ((stop - start) as u64).div_ceil(step)
        } else {
            0
        };
        Ok(ResolvedDim {
            start: start as u64,
            step,
            count,
            kept: true,
        })
    }
}

fn normalize_index(i: i64, len: u64) -> Result<u64> {
    let len_i = len as i64;
    let idx = if i < 0 { i + len_i } else { i };
    if idx < 0 || idx >= len_i {
        return Err(Error::InvalidSelection(format!(
            "index {i} out of bounds for length {len}"
        )));
    }
    Ok(idx as u64)
}

/// A selection resolved against a concrete dimension length.
///
/// The selected indices are `start + k * step` for `k` in `0..count`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedDim {
    /// First selected index.
    pub start: u64,
    /// Distance between selected indices (always at least 1).
    pub step: u64,
    /// Number of selected indices.
    pub count: u64,
    /// Whether the dimension appears in the output (`false` for integer indices).
    pub kept: bool,
}

/// Resolves a full list of selections against `shape`. Missing trailing
/// selections are treated as [`Selection::ALL`].
pub fn resolve_all(shape: &[u64], selections: &[Selection]) -> Result<Vec<ResolvedDim>> {
    if selections.len() > shape.len() {
        return Err(Error::InvalidSelection(format!(
            "{} selections given for a {}-dimensional array",
            selections.len(),
            shape.len()
        )));
    }
    shape
        .iter()
        .enumerate()
        .map(|(d, &len)| match selections.get(d) {
            Some(s) => s.resolve(len),
            None => Selection::ALL.resolve(len),
        })
        .collect()
}

/// The shape of the output produced by a resolved selection (integer-indexed
/// dimensions are dropped).
pub fn output_shape(resolved: &[ResolvedDim]) -> Vec<u64> {
    resolved
        .iter()
        .filter(|r| r.kept)
        .map(|r| r.count)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_and_clamped_bounds() {
        let r = Slice {
            start: Some(-3),
            stop: Some(100),
            step: 1,
        }
        .resolve(10)
        .unwrap();
        assert_eq!((r.start, r.count, r.step), (7, 3, 1));
        let r = Slice {
            start: Some(5),
            stop: Some(2),
            step: 1,
        }
        .resolve(10)
        .unwrap();
        assert_eq!(r.count, 0);
    }

    #[test]
    fn steps_round_up() {
        let r = Slice {
            start: Some(1),
            stop: None,
            step: 3,
        }
        .resolve(10)
        .unwrap();
        // indices 1, 4, 7
        assert_eq!((r.start, r.step, r.count), (1, 3, 3));
    }

    #[test]
    fn integer_index_drops_dimension() {
        let res = resolve_all(
            &[4, 5, 6],
            &[Selection::Index(-1), Selection::Range(Slice::range(0, 2))],
        )
        .unwrap();
        assert_eq!(res[0].start, 3);
        assert!(!res[0].kept);
        assert_eq!(output_shape(&res), vec![2, 6]);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(Selection::Index(10).resolve(10).is_err());
        assert!(
            Slice {
                start: None,
                stop: None,
                step: 0
            }
            .resolve(4)
            .is_err()
        );
        assert!(resolve_all(&[2], &[Selection::ALL, Selection::ALL]).is_err());
    }
}
