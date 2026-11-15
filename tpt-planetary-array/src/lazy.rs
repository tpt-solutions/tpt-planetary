//! Lazy expression graph and chunk-streaming reductions.
//!
//! An [`Expr`] is built from an opened [`Array`] (metadata only, no data is
//! read) by slicing and elementwise scalar operations. Reductions walk the
//! output in blocks of at most [`EvalOptions::block_elements`] elements, so
//! memory use is bounded by the block size rather than by the array size.
//!
//! Values are evaluated as `f64`. Elements equal to the array's fill value
//! and NaN elements are masked out of every reduction (NaN-aware semantics).
//! Mean, variance and standard deviation are computed with Welford's
//! algorithm, merged across blocks with the Chan et al. pairwise update.

use alloc::boxed::Box;
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;

use crate::array::Array;
use crate::error::{Error, Result};
use crate::selection::{ResolvedDim, Selection, Slice, output_shape, resolve_all};

/// Default upper bound on the number of elements evaluated at once.
pub const DEFAULT_BLOCK_ELEMENTS: usize = 1 << 18;

/// Options controlling chunk-streaming evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EvalOptions {
    /// Maximum number of elements in one evaluated block (at least 1).
    pub block_elements: usize,
}

impl Default for EvalOptions {
    fn default() -> Self {
        Self {
            block_elements: DEFAULT_BLOCK_ELEMENTS,
        }
    }
}

/// A rectangular region of an expression's output: indices
/// `start[d] + j * step[d]` for `j` in `0..count[d]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Region {
    /// First index per dimension.
    pub start: Vec<u64>,
    /// Number of indices per dimension.
    pub count: Vec<u64>,
    /// Stride per dimension (at least 1).
    pub step: Vec<u64>,
}

/// Values of one evaluated region, in C order, with a validity mask.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Element values.
    pub values: Vec<f64>,
    /// `false` where the element is fill-valued or NaN and must be ignored.
    pub valid: Vec<bool>,
}

/// Running statistics over valid elements, updated with Welford's method.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Stats {
    /// Number of valid elements.
    pub count: u64,
    /// Sum of valid elements.
    pub sum: f64,
    /// Mean of valid elements (zero when `count` is zero).
    pub mean: f64,
    /// Sum of squared deviations from the mean (Welford's `M2`).
    pub m2: f64,
    /// Smallest valid element (`+inf` when empty).
    pub min: f64,
    /// Largest valid element (`-inf` when empty).
    pub max: f64,
}

impl Default for Stats {
    fn default() -> Self {
        Self {
            count: 0,
            sum: 0.0,
            mean: 0.0,
            m2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }
}

impl Stats {
    /// Adds one value.
    pub fn push(&mut self, x: f64) {
        self.count += 1;
        let n = self.count as f64;
        let delta = x - self.mean;
        self.mean += delta / n;
        self.m2 += delta * (x - self.mean);
        self.sum += x;
        self.min = self.min.min(x);
        self.max = self.max.max(x);
    }

    /// Merges the statistics of another disjoint set of values into this one.
    pub fn merge(&mut self, other: &Stats) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = *other;
            return;
        }
        let na = self.count as f64;
        let nb = other.count as f64;
        let n = na + nb;
        let delta = other.mean - self.mean;
        self.mean += delta * nb / n;
        self.m2 += other.m2 + delta * delta * na * nb / n;
        self.count += other.count;
        self.sum += other.sum;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    /// Population variance (`ddof = 0`), or `None` when empty.
    pub fn variance(&self) -> Option<f64> {
        (self.count > 0).then(|| (self.m2 / self.count as f64).max(0.0))
    }
}

/// Counters describing one evaluation pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ExecStats {
    /// Number of blocks evaluated.
    pub blocks: u64,
    /// Largest number of elements held in one block.
    pub max_block_elements: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Op {
    Add,
    Mul,
}

/// A custom node in the expression graph, for operations implemented outside
/// this module (see [`Expr::from_op`]).
///
/// `eval_region` is called with regions already checked against
/// [`NodeOp::shape`], and must return exactly `region.count` values in C order.
pub trait NodeOp: core::fmt::Debug + Send + Sync {
    /// The shape of the node's output.
    fn shape(&self) -> Vec<u64>;

    /// Evaluates the output over `region`.
    fn eval_region(&self, region: &Region) -> Result<Block>;
}

#[derive(Debug)]
enum Node {
    Load(Arc<Array>),
    Op(Box<dyn NodeOp>),
    Slice {
        input: Expr,
        resolved: Vec<ResolvedDim>,
        shape: Vec<u64>,
    },
    Scalar {
        input: Expr,
        op: Op,
        value: f64,
    },
}

/// A lazily evaluated array expression. Cloning is cheap.
#[derive(Debug, Clone)]
pub struct Expr {
    node: Arc<Node>,
}

impl Expr {
    /// A leaf expression that reads from `array` when evaluated. Creating it
    /// reads no chunk data.
    pub fn from_array(array: Array) -> Self {
        Self {
            node: Arc::new(Node::Load(Arc::new(array))),
        }
    }

    /// The shape of the expression's output.
    pub fn shape(&self) -> Vec<u64> {
        match &*self.node {
            Node::Load(a) => a.shape().to_vec(),
            Node::Op(op) => op.shape(),
            Node::Slice { shape, .. } => shape.clone(),
            Node::Scalar { input, .. } => input.shape(),
        }
    }

    /// Wraps a custom [`NodeOp`] as an expression.
    pub fn from_op(op: impl NodeOp + 'static) -> Self {
        Self {
            node: Arc::new(Node::Op(Box::new(op))),
        }
    }

    /// Selects a sub-region. Integer indices remove a dimension.
    pub fn slice(&self, selection: &[Selection]) -> Result<Self> {
        let shape = self.shape();
        let resolved = resolve_all(&shape, selection)?;
        let out = output_shape(&resolved);
        Ok(Self {
            node: Arc::new(Node::Slice {
                input: self.clone(),
                resolved,
                shape: out,
            }),
        })
    }

    /// Adds a scalar to every element.
    pub fn add_scalar(&self, value: f64) -> Self {
        self.scalar(Op::Add, value)
    }

    /// Multiplies every element by a scalar.
    pub fn mul_scalar(&self, value: f64) -> Self {
        self.scalar(Op::Mul, value)
    }

    fn scalar(&self, op: Op, value: f64) -> Self {
        Self {
            node: Arc::new(Node::Scalar {
                input: self.clone(),
                op,
                value,
            }),
        }
    }

    /// Evaluates the expression over `region` of its output.
    pub fn eval_region(&self, region: &Region) -> Result<Block> {
        let shape = self.shape();
        if region.start.len() != shape.len()
            || region.count.len() != shape.len()
            || region.step.len() != shape.len()
        {
            return Err(Error::InvalidSelection(
                "region rank does not match expression".into(),
            ));
        }
        for (d, &len) in shape.iter().enumerate() {
            let end = region.start[d] + region.count[d].saturating_sub(1) * region.step[d];
            if region.step[d] == 0 || (region.count[d] > 0 && end >= len) {
                return Err(Error::InvalidSelection(alloc::format!(
                    "region out of bounds in dimension {d}"
                )));
            }
        }
        match &*self.node {
            Node::Load(array) => load_block(array, region),
            Node::Op(op) => op.eval_region(region),
            Node::Slice {
                input, resolved, ..
            } => {
                let in_region = map_region_through_slice(region, resolved);
                input.eval_region(&in_region)
            }
            Node::Scalar { input, op, value } => {
                let mut block = input.eval_region(region)?;
                for v in &mut block.values {
                    *v = match op {
                        Op::Add => *v + value,
                        Op::Mul => *v * value,
                    };
                }
                Ok(block)
            }
        }
    }

    /// Computes statistics over all valid elements, streaming block by block.
    pub fn stats(&self) -> Result<Stats> {
        self.stats_with(EvalOptions::default()).map(|(s, _)| s)
    }

    /// Like [`Expr::stats`] with explicit options; also returns evaluation
    /// counters, which tests use to check the memory bound.
    pub fn stats_with(&self, options: EvalOptions) -> Result<(Stats, ExecStats)> {
        if options.block_elements == 0 {
            return Err(Error::InvalidSelection(
                "block_elements must be positive".into(),
            ));
        }
        let shape = self.shape();
        let mut total = Stats::default();
        let mut exec = ExecStats::default();
        for region in BlockIter::new(&shape, options.block_elements) {
            let block = self.eval_region(&region)?;
            exec.blocks += 1;
            exec.max_block_elements = exec.max_block_elements.max(block.values.len());
            let mut local = Stats::default();
            for (&v, &ok) in block.values.iter().zip(&block.valid) {
                if ok && !v.is_nan() {
                    local.push(v);
                }
            }
            total.merge(&local);
        }
        Ok((total, exec))
    }

    /// Sum of valid elements (zero if there are none).
    pub fn sum(&self) -> Result<f64> {
        Ok(self.stats()?.sum)
    }

    /// Number of valid (non-fill, non-NaN) elements.
    pub fn count(&self) -> Result<u64> {
        Ok(self.stats()?.count)
    }

    /// Mean of valid elements, or `None` if there are none.
    pub fn mean(&self) -> Result<Option<f64>> {
        let s = self.stats()?;
        Ok((s.count > 0).then_some(s.mean))
    }

    /// Smallest valid element, or `None` if there are none.
    pub fn min(&self) -> Result<Option<f64>> {
        let s = self.stats()?;
        Ok((s.count > 0).then_some(s.min))
    }

    /// Largest valid element, or `None` if there are none.
    pub fn max(&self) -> Result<Option<f64>> {
        let s = self.stats()?;
        Ok((s.count > 0).then_some(s.max))
    }

    /// Population standard deviation (`ddof = 0`) of valid elements, or
    /// `None` if there are none. Requires the `std` feature (square root is
    /// not available in `core`); [`Stats::variance`] works without it.
    #[cfg(feature = "std")]
    pub fn std(&self) -> Result<Option<f64>> {
        Ok(self.stats()?.variance().map(f64::sqrt))
    }
}

fn load_block(array: &Array, region: &Region) -> Result<Block> {
    let selection: Vec<Selection> = (0..region.start.len())
        .map(|d| {
            let start = region.start[d] as i64;
            let stop = if region.count[d] == 0 {
                start
            } else {
                start + ((region.count[d] - 1) * region.step[d]) as i64 + 1
            };
            Selection::Range(Slice {
                start: Some(start),
                stop: Some(stop),
                step: region.step[d] as i64,
            })
        })
        .collect();
    let (_, bytes) = array.read_bytes(&selection)?;
    let values = array.dtype().decode_f64(&bytes)?;
    let fill = array.fill_value();
    let valid = values
        .iter()
        .map(|&v| !v.is_nan() && !fill.matches(v))
        .collect();
    Ok(Block { values, valid })
}

fn map_region_through_slice(region: &Region, resolved: &[ResolvedDim]) -> Region {
    let mut start = Vec::with_capacity(resolved.len());
    let mut count = Vec::with_capacity(resolved.len());
    let mut step = Vec::with_capacity(resolved.len());
    let mut k = 0usize;
    for r in resolved {
        if r.kept {
            start.push(r.start + region.start[k] * r.step);
            count.push(region.count[k]);
            step.push(region.step[k] * r.step);
            k += 1;
        } else {
            start.push(r.start);
            count.push(1);
            step.push(1);
        }
    }
    Region { start, count, step }
}

/// Iterates over contiguous blocks covering a shape, with each block holding
/// at most `budget` elements (where possible; a single element always fits).
#[derive(Debug)]
pub(crate) struct BlockIter {
    shape: Vec<u64>,
    block: Vec<u64>,
    grid_idx: Vec<u64>,
    done: bool,
}

impl BlockIter {
    pub(crate) fn new(shape: &[u64], budget: usize) -> Self {
        let nd = shape.len();
        let mut block = vec![1u64; nd];
        let mut left = budget.max(1) as u64;
        for d in (0..nd).rev() {
            let take = shape[d].min(left).max(1);
            block[d] = take;
            left = (left / take).max(1);
        }
        let done = shape.contains(&0);
        Self {
            shape: shape.to_vec(),
            block,
            grid_idx: vec![0; nd],
            done,
        }
    }
}

impl Iterator for BlockIter {
    type Item = Region;

    fn next(&mut self) -> Option<Region> {
        if self.done {
            return None;
        }
        let nd = self.shape.len();
        let start: Vec<u64> = self
            .grid_idx
            .iter()
            .zip(&self.block)
            .map(|(&i, &b)| i * b)
            .collect();
        let count: Vec<u64> = start
            .iter()
            .zip(&self.block)
            .zip(&self.shape)
            .map(|((&s, &b), &n)| b.min(n - s))
            .collect();
        let region = Region {
            start,
            count,
            step: vec![1; nd],
        };
        // Advance the grid index, last dimension fastest.
        let mut d = nd;
        loop {
            if d == 0 {
                self.done = true;
                break;
            }
            d -= 1;
            self.grid_idx[d] += 1;
            let blocks_d = self.shape[d].div_ceil(self.block[d]);
            if self.grid_idx[d] < blocks_d {
                break;
            }
            self.grid_idx[d] = 0;
        }
        Some(region)
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::dtype::DType;
    use crate::fill::FillValue;
    use crate::meta::{ArrayMetadata, ZarrFormat};
    use crate::store::MemoryStore;

    fn make_array(shape: Vec<u64>, chunks: Vec<u64>, data: &[f64], fill: FillValue) -> Array {
        let store = Arc::new(MemoryStore::new());
        let meta = ArrayMetadata::new(ZarrFormat::V2, shape.clone(), chunks, DType::FLOAT64)
            .unwrap()
            .with_fill_value(fill);
        let a = Array::create(store, "d", meta).unwrap();
        let sel: Vec<Selection> = shape.iter().map(|_| Selection::ALL).collect();
        a.write_f64(&sel, data).unwrap();
        a
    }

    #[test]
    fn welford_matches_two_pass() {
        let xs = [1.0e9 + 4.0, 1.0e9 + 7.0, 1.0e9 + 13.0, 1.0e9 + 16.0];
        let mut s = Stats::default();
        for &x in &xs {
            s.push(x);
        }
        let mean = xs.iter().sum::<f64>() / 4.0;
        let var = xs.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / 4.0;
        assert!((s.mean - mean).abs() < 1e-6);
        assert!((s.variance().unwrap() - var).abs() < 1e-6);
    }

    #[test]
    fn merge_equals_single_pass() {
        let xs: Vec<f64> = (0..100).map(|i| f64::from(i).sin() * 10.0).collect();
        let mut whole = Stats::default();
        xs.iter().for_each(|&x| whole.push(x));
        let (mut a, mut b) = (Stats::default(), Stats::default());
        xs[..37].iter().for_each(|&x| a.push(x));
        xs[37..].iter().for_each(|&x| b.push(x));
        a.merge(&b);
        assert_eq!(a.count, whole.count);
        assert!((a.mean - whole.mean).abs() < 1e-12);
        assert!((a.m2 - whole.m2).abs() < 1e-9);
        assert_eq!(a.min, whole.min);
    }

    #[test]
    fn reductions_over_chunked_array() {
        let data: Vec<f64> = (1..=12).map(f64::from).collect();
        let a = make_array(vec![3, 4], vec![2, 3], &data, FillValue::Null);
        let e = Expr::from_array(a);
        assert_eq!(e.sum().unwrap(), 78.0);
        assert_eq!(e.count().unwrap(), 12);
        assert_eq!(e.mean().unwrap(), Some(6.5));
        assert_eq!(e.min().unwrap(), Some(1.0));
        assert_eq!(e.max().unwrap(), Some(12.0));
        let expected_std = (data.iter().map(|x| (x - 6.5).powi(2)).sum::<f64>() / 12.0).sqrt();
        assert!((e.std().unwrap().unwrap() - expected_std).abs() < 1e-12);
    }

    #[test]
    fn fill_and_nan_are_masked() {
        let data = [1.0, -9999.0, f64::NAN, 3.0];
        let a = make_array(vec![4], vec![3], &data, FillValue::Int(-9999));
        let e = Expr::from_array(a);
        assert_eq!(e.count().unwrap(), 2);
        assert_eq!(e.mean().unwrap(), Some(2.0));
    }

    #[test]
    fn nan_fill_masks_nan_values() {
        let data = [f64::NAN, 2.0, 4.0];
        let a = make_array(vec![3], vec![2], &data, FillValue::Float(f64::NAN));
        assert_eq!(Expr::from_array(a).mean().unwrap(), Some(3.0));
    }

    #[test]
    fn slice_and_scalar_ops_compose() {
        let data: Vec<f64> = (0..20).map(f64::from).collect();
        let a = make_array(vec![4, 5], vec![3, 2], &data, FillValue::Null);
        let e = Expr::from_array(a)
            .slice(&[
                Selection::Range(Slice {
                    start: Some(1),
                    stop: None,
                    step: 2,
                }),
                Selection::Index(4),
            ])
            .unwrap();
        // rows 1 and 3, column 4 -> values 9 and 19
        assert_eq!(e.shape(), vec![2]);
        assert_eq!(e.sum().unwrap(), 28.0);
        let scaled = e.mul_scalar(2.0).add_scalar(1.0);
        assert_eq!(scaled.sum().unwrap(), 2.0 * 28.0 + 2.0);
        assert_eq!(scaled.count().unwrap(), 2);
    }

    #[test]
    fn block_budget_bounds_memory_and_result_is_independent_of_blocking() {
        let n = 60 * 50;
        let data: Vec<f64> = (0..n).map(|i| ((i * 37) % 101) as f64).collect();
        let a = make_array(vec![60, 50], vec![7, 9], &data, FillValue::Float(0.0));
        let e = Expr::from_array(a);
        let (big, _) = e
            .stats_with(EvalOptions {
                block_elements: 1 << 20,
            })
            .unwrap();
        let (small, exec) = e.stats_with(EvalOptions { block_elements: 64 }).unwrap();
        assert!(exec.max_block_elements <= 64);
        assert!(exec.blocks > 1);
        assert_eq!(big.count, small.count);
        assert!((big.mean - small.mean).abs() < 1e-9);
        assert!((big.m2 - small.m2).abs() < 1e-6);
        assert_eq!(big.min, small.min);
        assert_eq!(big.max, small.max);
    }

    #[test]
    fn empty_selection_reductions() {
        let a = make_array(vec![4], vec![2], &[1.0, 2.0, 3.0, 4.0], FillValue::Null);
        let e = Expr::from_array(a)
            .slice(&[Selection::Range(Slice::range(2, 2))])
            .unwrap();
        assert_eq!(e.sum().unwrap(), 0.0);
        assert_eq!(e.count().unwrap(), 0);
        assert_eq!(e.mean().unwrap(), None);
        assert_eq!(e.min().unwrap(), None);
    }

    #[test]
    fn block_iter_covers_shape_exactly_once() {
        let blocks: Vec<Region> = BlockIter::new(&[5, 4], 6).collect();
        let total: u64 = blocks.iter().map(|r| r.count.iter().product::<u64>()).sum();
        assert_eq!(total, 20);
        assert!(blocks.iter().all(|r| r.count.iter().product::<u64>() <= 6));
    }
}
