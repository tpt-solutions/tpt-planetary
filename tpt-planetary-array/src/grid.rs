//! Shapes, C-order strides, the regular chunk grid and chunk-key encoding.

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;

use crate::error::{Error, Result};

/// Number of elements in an N-D shape, checking for overflow.
pub fn num_elements(shape: &[u64]) -> Result<u64> {
    shape.iter().try_fold(1u64, |acc, &d| {
        acc.checked_mul(d)
            .ok_or_else(|| Error::InvalidMetadata("shape element count overflows u64".into()))
    })
}

/// C-order (row-major) strides measured in elements.
pub fn c_strides(shape: &[u64]) -> Vec<u64> {
    let mut strides = alloc::vec![1u64; shape.len()];
    for d in (0..shape.len().saturating_sub(1)).rev() {
        strides[d] = strides[d + 1] * shape[d + 1];
    }
    strides
}

/// Regular (fixed-size) chunk grid over an array of `shape`.
///
/// Every chunk is stored with the full `chunk_shape`, including edge chunks
/// that extend past the array boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkGrid {
    shape: Vec<u64>,
    chunk_shape: Vec<u64>,
}

impl ChunkGrid {
    /// Creates a chunk grid, validating that dimensions agree and that chunk
    /// sizes are non-zero.
    pub fn new(shape: Vec<u64>, chunk_shape: Vec<u64>) -> Result<Self> {
        if shape.len() != chunk_shape.len() {
            return Err(Error::InvalidMetadata(format!(
                "shape has {} dimensions but chunks has {}",
                shape.len(),
                chunk_shape.len()
            )));
        }
        if chunk_shape.contains(&0) {
            return Err(Error::InvalidMetadata(
                "chunk sizes must be positive".into(),
            ));
        }
        num_elements(&shape)?;
        Ok(Self { shape, chunk_shape })
    }

    /// Array shape.
    pub fn shape(&self) -> &[u64] {
        &self.shape
    }

    /// Chunk shape (uniform across the grid).
    pub fn chunk_shape(&self) -> &[u64] {
        &self.chunk_shape
    }

    /// Number of dimensions.
    pub fn ndim(&self) -> usize {
        self.shape.len()
    }

    /// Number of chunks along each dimension.
    pub fn grid_shape(&self) -> Vec<u64> {
        self.shape
            .iter()
            .zip(&self.chunk_shape)
            .map(|(&s, &c)| s.div_ceil(c))
            .collect()
    }

    /// Number of elements in one stored chunk (full chunk shape).
    pub fn chunk_elements(&self) -> Result<u64> {
        num_elements(&self.chunk_shape)
    }

    /// Element origin (array coordinates) of the chunk at `coord`.
    pub fn chunk_origin(&self, coord: &[u64]) -> Vec<u64> {
        coord
            .iter()
            .zip(&self.chunk_shape)
            .map(|(&c, &s)| c * s)
            .collect()
    }

    /// Checks that `coord` addresses a chunk inside the grid.
    pub fn check_coord(&self, coord: &[u64]) -> Result<()> {
        if coord.len() != self.ndim() {
            return Err(Error::InvalidSelection(
                "chunk coordinate has wrong rank".into(),
            ));
        }
        for (d, (&c, &n)) in coord.iter().zip(&self.grid_shape()).enumerate() {
            if c >= n {
                return Err(Error::InvalidSelection(format!(
                    "chunk index {c} out of range for dimension {d} with {n} chunks"
                )));
            }
        }
        Ok(())
    }
}

/// How chunk coordinates map to store keys.
///
/// With `prefix` set the key starts with `c` (Zarr v3 default encoding:
/// `c/0/1`). Without it the key is the bare coordinate joined by the
/// separator (Zarr v2 and v3 `v2` encoding: `0.1` or `0/1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkKeyEncoding {
    prefix: bool,
    separator: char,
}

impl ChunkKeyEncoding {
    /// Zarr v3 `default` encoding with the given separator (usually `/`).
    pub fn v3_default(separator: char) -> Self {
        Self {
            prefix: true,
            separator,
        }
    }

    /// Zarr v2 style encoding (`.zarray` `dimension_separator`), also the
    /// v3 `v2` encoding. Separator is usually `.` for v2 and may be `/`.
    pub fn v2(separator: char) -> Self {
        Self {
            prefix: false,
            separator,
        }
    }

    /// The separator between coordinate components.
    pub fn separator(&self) -> char {
        self.separator
    }

    /// Whether keys carry the `c` prefix.
    pub fn has_prefix(&self) -> bool {
        self.prefix
    }

    /// Builds the store key for chunk `coord`.
    pub fn key(&self, coord: &[u64]) -> String {
        let mut key = String::new();
        if self.prefix {
            key.push('c');
            if coord.is_empty() {
                return key;
            }
            key.push(self.separator);
        } else if coord.is_empty() {
            return "0".into();
        }
        for (i, c) in coord.iter().enumerate() {
            if i > 0 {
                key.push(self.separator);
            }
            key.push_str(&format!("{c}"));
        }
        key
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strides_are_c_order() {
        assert_eq!(c_strides(&[2, 3, 4]), vec![12, 4, 1]);
        assert_eq!(c_strides(&[]), Vec::<u64>::new());
    }

    #[test]
    fn grid_counts_partial_chunks() {
        let g = ChunkGrid::new(vec![10, 7], vec![4, 4]).unwrap();
        assert_eq!(g.grid_shape(), vec![3, 2]);
        assert_eq!(g.chunk_origin(&[2, 1]), vec![8, 4]);
        assert!(g.check_coord(&[2, 1]).is_ok());
        assert!(g.check_coord(&[3, 0]).is_err());
        assert!(ChunkGrid::new(vec![4], vec![0]).is_err());
    }

    #[test]
    fn key_encodings() {
        let v3 = ChunkKeyEncoding::v3_default('/');
        assert_eq!(v3.key(&[0, 1]), "c/0/1");
        assert_eq!(v3.key(&[]), "c");
        let v2 = ChunkKeyEncoding::v2('.');
        assert_eq!(v2.key(&[0, 1]), "0.1");
        assert_eq!(v2.key(&[]), "0");
        assert_eq!(ChunkKeyEncoding::v2('/').key(&[3, 4]), "3/4");
    }
}
