//! Zarr v3 `transpose` array-to-array codec.
//!
//! The codec permutes the axes of a chunk. Its configuration is
//! `{"order": [p0, p1, ...]}`, a permutation of the dimensions: output axis
//! `k` is input axis `p_k`, so the output shape is `shape[p_k]`. Encoding
//! applies the permutation; decoding applies its inverse.
//!
//! The codec needs the chunk shape, which the registry does not supply, so it
//! is built by [`ArrayMetadata::pipeline`](crate::meta::ArrayMetadata::pipeline).

use alloc::boxed::Box;
use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use serde_json::Value;

use super::Codec;
use crate::dtype::DType;
use crate::error::{Error, Result};

/// Axis permutation of a chunk with a fixed shape and element size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransposeCodec {
    order: Vec<usize>,
    shape: Vec<u64>,
    item_size: usize,
}

impl TransposeCodec {
    /// Builds the codec from the `configuration` object, for a chunk of
    /// `shape` whose elements are `item_size` bytes.
    pub fn new(config: &Value, shape: &[u64], item_size: usize) -> Result<Self> {
        let arr = config
            .get("order")
            .and_then(Value::as_array)
            .ok_or_else(|| Error::InvalidMetadata("transpose needs an 'order' array".into()))?;
        let order = arr
            .iter()
            .map(|v| {
                v.as_u64()
                    .map(|x| x as usize)
                    .ok_or_else(|| Error::Unsupported("transpose order must be integers".into()))
            })
            .collect::<Result<Vec<_>>>()?;
        Self::from_order(order, shape, item_size)
    }

    /// Builds the codec from an explicit permutation.
    pub fn from_order(order: Vec<usize>, shape: &[u64], item_size: usize) -> Result<Self> {
        let nd = shape.len();
        let mut seen = vec![false; nd];
        if order.len() != nd {
            return Err(Error::InvalidMetadata(format!(
                "transpose order has {} entries for {nd} dimensions",
                order.len()
            )));
        }
        for &p in &order {
            if p >= nd || seen[p] {
                return Err(Error::InvalidMetadata(format!(
                    "transpose order {order:?} is not a permutation"
                )));
            }
            seen[p] = true;
        }
        Ok(Self {
            order,
            shape: shape.to_vec(),
            item_size,
        })
    }

    /// The permutation (output axis `k` is input axis `order()[k]`).
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// Shape of the encoded (transposed) chunk.
    pub fn output_shape(&self) -> Vec<u64> {
        self.order.iter().map(|&p| self.shape[p]).collect()
    }
}

impl Codec for TransposeCodec {
    fn name(&self) -> &str {
        "transpose"
    }

    fn encode(&self, input: &[u8]) -> Result<Vec<u8>> {
        permute(input, &self.shape, &self.order, self.item_size)
    }

    fn decode(&self, input: &[u8]) -> Result<Vec<u8>> {
        let inverse = inverse_permutation(&self.order);
        permute(input, &self.output_shape(), &inverse, self.item_size)
    }
}

fn inverse_permutation(order: &[usize]) -> Vec<usize> {
    let mut inv = vec![0usize; order.len()];
    for (k, &p) in order.iter().enumerate() {
        inv[p] = k;
    }
    inv
}

/// Copies C-ordered `input` of shape `shape` into C-ordered output of shape
/// `shape[perm[k]]`, element by element (`item` bytes each).
fn permute(input: &[u8], shape: &[u64], perm: &[usize], item: usize) -> Result<Vec<u8>> {
    let numel: usize = shape.iter().map(|&s| s as usize).product();
    if input.len() != numel * item {
        return Err(Error::SizeMismatch {
            expected: numel * item,
            actual: input.len(),
        });
    }
    let nd = shape.len();
    let out_shape: Vec<usize> = perm.iter().map(|&p| shape[p] as usize).collect();
    let mut out = vec![0u8; input.len()];
    if numel == 0 {
        return Ok(out);
    }
    // C-order strides of the input.
    let mut in_stride = vec![1usize; nd];
    for k in (0..nd.saturating_sub(1)).rev() {
        in_stride[k] = in_stride[k + 1] * shape[k + 1] as usize;
    }
    // Walk the output in C order with an odometer over its indices.
    let mut idx = vec![0usize; nd];
    for out_lin in 0..numel {
        let src: usize = (0..nd).map(|k| idx[k] * in_stride[perm[k]]).sum();
        out[out_lin * item..(out_lin + 1) * item]
            .copy_from_slice(&input[src * item..(src + 1) * item]);
        for k in (0..nd).rev() {
            idx[k] += 1;
            if idx[k] < out_shape[k] {
                break;
            }
            idx[k] = 0;
        }
    }
    Ok(out)
}

/// Factory placeholder: transpose needs the chunk shape, which the registry
/// does not have. Pipelines build it through `ArrayMetadata::pipeline`.
pub(crate) fn transpose_factory(_config: &Value, _dtype: &DType) -> Result<Box<dyn Codec>> {
    Err(Error::Unsupported(
        "transpose needs the chunk shape; build it through ArrayMetadata::pipeline".into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hand_computed_two_by_three() {
        // [[1, 2, 3],
        //  [4, 5, 6]]  transposed is  [[1, 4], [2, 5], [3, 6]].
        let c = TransposeCodec::new(&json!({"order": [1, 0]}), &[2, 3], 1).unwrap();
        assert_eq!(c.output_shape(), vec![3, 2]);
        let enc = c.encode(&[1, 2, 3, 4, 5, 6]).unwrap();
        assert_eq!(enc, vec![1, 4, 2, 5, 3, 6]);
        assert_eq!(c.decode(&enc).unwrap(), vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn two_byte_elements_move_whole() {
        let c = TransposeCodec::new(&json!({"order": [1, 0]}), &[2, 2], 2).unwrap();
        let enc = c.encode(&[1, 0, 2, 0, 3, 0, 4, 0]).unwrap();
        assert_eq!(enc, vec![1, 0, 3, 0, 2, 0, 4, 0]);
    }

    #[test]
    fn three_dimensional_round_trip() {
        let shape = [2u64, 3, 4];
        let data: Vec<u8> = (0..24u8).collect();
        for order in [[0usize, 1, 2], [2, 0, 1], [1, 2, 0], [2, 1, 0]] {
            let c = TransposeCodec::new(&json!({"order": order}), &shape, 1).unwrap();
            let enc = c.encode(&data).unwrap();
            assert_eq!(c.decode(&enc).unwrap(), data, "order {order:?}");
        }
        // Identity leaves the bytes alone.
        let id = TransposeCodec::new(&json!({"order": [0, 1, 2]}), &shape, 1).unwrap();
        assert_eq!(id.encode(&data).unwrap(), data);
    }

    #[test]
    fn rejects_non_permutations_and_strings() {
        assert!(TransposeCodec::new(&json!({"order": [0, 0]}), &[2, 2], 1).is_err());
        assert!(TransposeCodec::new(&json!({"order": [0]}), &[2, 2], 1).is_err());
        assert!(TransposeCodec::new(&json!({"order": "F"}), &[2, 2], 1).is_err());
        assert!(TransposeCodec::new(&json!({}), &[2, 2], 1).is_err());
    }
}
