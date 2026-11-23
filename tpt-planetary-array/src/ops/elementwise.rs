//! Elementwise arithmetic, unary functions, comparisons, masks and `where`,
//! all broadcasting-aware and evaluated one region at a time.
//!
//! Results are lazy: each operation adds a node to the expression graph and
//! evaluates only the region requested by a reduction. Boolean results are
//! represented as `f64` values `1.0` (true) and `0.0` (false).

use alloc::vec;
use alloc::vec::Vec;

use crate::error::Result;
use crate::lazy::{Block, Expr, NodeOp, Region};
use crate::ops::broadcast::{broadcast_shapes, expand_block, operand_region};
use crate::ops::nd::{empty_block, is_empty_region};

/// Binary arithmetic operators.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArithOp {
    /// `a + b`.
    Add,
    /// `a - b`.
    Sub,
    /// `a * b`.
    Mul,
    /// `a / b` (IEEE semantics: division by zero gives infinities or NaN).
    Div,
    /// `a.powf(b)`. Requires the `std` feature.
    #[cfg(feature = "std")]
    Pow,
}

impl ArithOp {
    pub(crate) fn apply(self, a: f64, b: f64) -> f64 {
        match self {
            ArithOp::Add => a + b,
            ArithOp::Sub => a - b,
            ArithOp::Mul => a * b,
            ArithOp::Div => a / b,
            #[cfg(feature = "std")]
            ArithOp::Pow => a.powf(b),
        }
    }
}

/// Comparison operators. The result is `1.0` when the comparison holds and
/// `0.0` otherwise; comparisons involving NaN are false, except `!=`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    /// `a < b`.
    Lt,
    /// `a <= b`.
    Le,
    /// `a > b`.
    Gt,
    /// `a >= b`.
    Ge,
    /// `a == b`.
    Eq,
    /// `a != b`.
    Ne,
}

impl CmpOp {
    pub(crate) fn apply(self, a: f64, b: f64) -> f64 {
        let holds = match self {
            CmpOp::Lt => a < b,
            CmpOp::Le => a <= b,
            CmpOp::Gt => a > b,
            CmpOp::Ge => a >= b,
            CmpOp::Eq => a == b,
            CmpOp::Ne => a != b,
        };
        if holds { 1.0 } else { 0.0 }
    }
}

/// Elementwise unary functions. Functions other than `Abs` use transcendental
/// operations from `std` and are available with the `std` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    /// Absolute value (available without `std`).
    Abs,
    /// Square root (NaN for negative inputs). Requires `std`.
    #[cfg(feature = "std")]
    Sqrt,
    /// Natural exponential. Requires `std`.
    #[cfg(feature = "std")]
    Exp,
    /// Natural logarithm. Requires `std`.
    #[cfg(feature = "std")]
    Log,
    /// Sine of an angle in radians. Requires `std`.
    #[cfg(feature = "std")]
    Sin,
    /// Cosine of an angle in radians. Requires `std`.
    #[cfg(feature = "std")]
    Cos,
}

impl UnaryOp {
    pub(crate) fn apply(self, x: f64) -> f64 {
        match self {
            // A sign test rather than `f64::abs`, which is not guaranteed to
            // be available in `core` on the MSRV. NaN propagates unchanged.
            UnaryOp::Abs => {
                if x < 0.0 {
                    -x
                } else {
                    x
                }
            }
            #[cfg(feature = "std")]
            UnaryOp::Sqrt => x.sqrt(),
            #[cfg(feature = "std")]
            UnaryOp::Exp => x.exp(),
            #[cfg(feature = "std")]
            UnaryOp::Log => x.ln(),
            #[cfg(feature = "std")]
            UnaryOp::Sin => x.sin(),
            #[cfg(feature = "std")]
            UnaryOp::Cos => x.cos(),
        }
    }
}

/// Evaluates an operand over the output `region` of a broadcast node.
fn operand_block(expr: &Expr, region: &Region, out_shape: &[u64]) -> Result<Block> {
    let src_shape = expr.shape();
    let src_region = operand_region(region, &src_shape);
    let block = expr.eval_region(&src_region)?;
    let off = out_shape.len() - src_shape.len();
    let mut src_counts = vec![1u64; out_shape.len()];
    for (j, &c) in src_region.count.iter().enumerate() {
        src_counts[off + j] = c;
    }
    Ok(expand_block(block, &src_counts, &region.count))
}

/// Combines two operand blocks pointwise. Output is valid only where both
/// inputs are valid.
#[derive(Debug)]
struct Arith {
    lhs: Expr,
    rhs: Expr,
    op: ArithOp,
    shape: Vec<u64>,
}

impl NodeOp for Arith {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let a = operand_block(&self.lhs, region, &self.shape)?;
        let b = operand_block(&self.rhs, region, &self.shape)?;
        let values = arith_values(self.op, &a.values, &b.values);
        let valid = a
            .valid
            .iter()
            .zip(&b.valid)
            .map(|(&p, &q)| p && q)
            .collect();
        Ok(Block { values, valid })
    }
}

/// Applies the comparison `op` pointwise, giving 1.0 where it holds and 0.0
/// elsewhere (NaN compares false). With the `simd` feature the vector kernel is
/// used where the operator has one.
fn compare_values(op: CmpOp, a: &[f64], b: &[f64]) -> Vec<f64> {
    #[cfg(feature = "simd")]
    {
        if let Some(values) = crate::ops::simd::compare(op, a, b) {
            return values;
        }
    }
    a.iter().zip(b).map(|(&x, &y)| op.apply(x, y)).collect()
}

/// Applies `op` pointwise. With the `simd` feature, the vector kernel is used
/// for operators that have one; otherwise the scalar loop runs.
fn arith_values(op: ArithOp, a: &[f64], b: &[f64]) -> Vec<f64> {
    #[cfg(feature = "simd")]
    {
        if let Some(values) = crate::ops::simd::arith(op, a, b) {
            return values;
        }
    }
    a.iter().zip(b).map(|(&x, &y)| op.apply(x, y)).collect()
}

#[derive(Debug)]
struct Compare {
    lhs: Expr,
    rhs: Expr,
    op: CmpOp,
    shape: Vec<u64>,
}

impl NodeOp for Compare {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let a = operand_block(&self.lhs, region, &self.shape)?;
        let b = operand_block(&self.rhs, region, &self.shape)?;
        let values = compare_values(self.op, &a.values, &b.values);
        let valid = a
            .valid
            .iter()
            .zip(&b.valid)
            .map(|(&p, &q)| p && q)
            .collect();
        Ok(Block { values, valid })
    }
}

#[derive(Debug)]
struct Select {
    cond: Expr,
    if_true: Expr,
    if_false: Expr,
    shape: Vec<u64>,
}

impl NodeOp for Select {
    fn shape(&self) -> Vec<u64> {
        self.shape.clone()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        if is_empty_region(region) {
            return Ok(empty_block());
        }
        let c = operand_block(&self.cond, region, &self.shape)?;
        let t = operand_block(&self.if_true, region, &self.shape)?;
        let f = operand_block(&self.if_false, region, &self.shape)?;
        let n = c.values.len();
        let mut values = Vec::with_capacity(n);
        let mut valid = Vec::with_capacity(n);
        for (i, &cv) in c.values.iter().enumerate() {
            // Truthiness as in NumPy: any value other than zero (NaN included)
            // selects the first branch.
            let (v, ok) = if cv != 0.0 {
                (t.values[i], t.valid[i])
            } else {
                (f.values[i], f.valid[i])
            };
            values.push(v);
            valid.push(c.valid[i] && ok);
        }
        Ok(Block { values, valid })
    }
}

#[derive(Debug)]
struct Unary {
    input: Expr,
    op: UnaryOp,
}

impl NodeOp for Unary {
    fn shape(&self) -> Vec<u64> {
        self.input.shape()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        let mut block = self.input.eval_region(region)?;
        #[cfg(feature = "simd")]
        {
            if let Some(values) = crate::ops::simd::unary(self.op, &block.values) {
                block.values = values;
                return Ok(block);
            }
        }
        for v in &mut block.values {
            *v = self.op.apply(*v);
        }
        Ok(block)
    }
}

#[derive(Debug)]
struct ValidMask {
    input: Expr,
}

impl NodeOp for ValidMask {
    fn shape(&self) -> Vec<u64> {
        self.input.shape()
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        let block = self.input.eval_region(region)?;
        let values = block
            .valid
            .iter()
            .map(|&ok| if ok { 1.0 } else { 0.0 })
            .collect();
        Ok(Block {
            valid: vec![true; block.valid.len()],
            values,
        })
    }
}

impl Expr {
    /// Broadcasting elementwise arithmetic between two expressions.
    ///
    /// Returns an error if the shapes cannot be broadcast together. An output
    /// element is valid only when both operand elements are valid.
    pub fn arith(&self, op: ArithOp, other: &Expr) -> Result<Expr> {
        let shape = broadcast_shapes(&[&self.shape(), &other.shape()])?;
        Ok(Expr::from_op(Arith {
            lhs: self.clone(),
            rhs: other.clone(),
            op,
            shape,
        }))
    }

    /// Broadcasting elementwise comparison, producing a boolean mask encoded
    /// as `1.0` / `0.0`. Its [`Expr::mean`] is the fraction of true elements.
    pub fn compare(&self, op: CmpOp, other: &Expr) -> Result<Expr> {
        let shape = broadcast_shapes(&[&self.shape(), &other.shape()])?;
        Ok(Expr::from_op(Compare {
            lhs: self.clone(),
            rhs: other.clone(),
            op,
            shape,
        }))
    }

    /// Applies a unary function to every element. Invalid elements stay
    /// invalid.
    pub fn unary(&self, op: UnaryOp) -> Expr {
        Expr::from_op(Unary {
            input: self.clone(),
            op,
        })
    }

    /// A mask that is `1.0` where this expression is valid (not fill-valued
    /// and not NaN) and `0.0` elsewhere. The mask itself is always valid.
    pub fn valid_mask(&self) -> Expr {
        Expr::from_op(ValidMask {
            input: self.clone(),
        })
    }

    /// Elementwise `cond ? if_true : if_false` with broadcasting across all
    /// three operands. An element takes the `if_true` value when its condition
    /// value is non-zero.
    pub fn select(cond: &Expr, if_true: &Expr, if_false: &Expr) -> Result<Expr> {
        let shape = broadcast_shapes(&[&cond.shape(), &if_true.shape(), &if_false.shape()])?;
        Ok(Expr::from_op(Select {
            cond: cond.clone(),
            if_true: if_true.clone(),
            if_false: if_false.clone(),
            shape,
        }))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::ops::broadcast::broadcast_index;
    use crate::ops::reduce::Reduction;

    type BinaryFn = fn(f64, f64) -> f64;
    type UnaryFn = fn(f64) -> f64;

    /// Scalar reference for a broadcast binary operation over full arrays.
    fn reference(
        a: (&[u64], &[f64]),
        b: (&[u64], &[f64]),
        f: impl Fn(f64, f64) -> f64,
    ) -> (Vec<u64>, Vec<f64>) {
        let shape = broadcast_shapes(&[a.0, b.0]).unwrap();
        let n: u64 = shape.iter().product();
        let lin = |s: &[u64], i: &[u64]| {
            let st = c_strides(s);
            i.iter().zip(&st).map(|(x, y)| x * y).sum::<u64>() as usize
        };
        let mut out = Vec::new();
        for flat in 0..n {
            let mut rem = flat;
            let mut idx = vec![0u64; shape.len()];
            for d in (0..shape.len()).rev() {
                idx[d] = rem % shape[d];
                rem /= shape[d];
            }
            let ia = broadcast_index(&idx, a.0).unwrap();
            let ib = broadcast_index(&idx, b.0).unwrap();
            out.push(f(a.1[lin(a.0, &ia)], b.1[lin(b.0, &ib)]));
        }
        (shape, out)
    }

    use crate::grid::c_strides;

    /// Materialises every element (one value per cell, no reduction).
    fn all_values(e: &Expr) -> Vec<f64> {
        e.reduce_axes(&[], Reduction::Sum, false).unwrap().values
    }

    fn data(n: usize, seed: usize) -> Vec<f64> {
        (0..n)
            .map(|i| (((i * 37 + seed * 11) % 23) as f64) - 9.0)
            .collect()
    }

    #[test]
    fn broadcast_arith_matches_reference() {
        let a_shape = vec![2u64, 3, 4];
        let b_shape = vec![3u64, 1];
        let a_vals = data(24, 1);
        let b_vals: Vec<f64> = data(3, 2).iter().map(|x| x + 20.0).collect();
        let a = Expr::from_values(a_shape.clone(), a_vals.clone()).unwrap();
        let b = Expr::from_values(b_shape.clone(), b_vals.clone()).unwrap();
        let ops: [(ArithOp, BinaryFn); 5] = [
            (ArithOp::Add, |x, y| x + y),
            (ArithOp::Sub, |x, y| x - y),
            (ArithOp::Mul, |x, y| x * y),
            (ArithOp::Div, |x, y| x / y),
            (ArithOp::Pow, f64::powf),
        ];
        for (op, f) in ops {
            let e = a.arith(op, &b).unwrap();
            let (shape, expected) = reference((&a_shape, &a_vals), (&b_shape, &b_vals), f);
            assert_eq!(e.shape(), shape);
            let got = all_values(&e);
            assert_eq!(got.len(), expected.len());
            for (g, x) in got.iter().zip(&expected) {
                assert!(
                    (g - x).abs() <= 1e-9 * x.abs().max(1.0),
                    "{op:?}: {g} vs {x}"
                );
            }
        }
    }

    #[test]
    fn incompatible_shapes_error() {
        let a = Expr::from_values(vec![2, 3], vec![0.0; 6]).unwrap();
        let b = Expr::from_values(vec![4], vec![0.0; 4]).unwrap();
        assert!(a.arith(ArithOp::Add, &b).is_err());
        assert!(a.compare(CmpOp::Lt, &b).is_err());
    }

    #[test]
    fn streaming_result_is_independent_of_block_size() {
        let a = Expr::from_values(vec![4, 5], data(20, 3)).unwrap();
        let b = Expr::from_values(vec![5], data(5, 4)).unwrap();
        let e = a.arith(ArithOp::Mul, &b).unwrap();
        let big = e
            .stats_with(crate::lazy::EvalOptions {
                block_elements: 1000,
                workers: 1,
            })
            .unwrap()
            .0;
        let (small, exec) = e
            .stats_with(crate::lazy::EvalOptions {
                block_elements: 3,
                workers: 1,
            })
            .unwrap();
        assert!(exec.max_block_elements <= 3);
        assert!(exec.blocks > 1);
        assert_eq!(big.count, small.count);
        assert!((big.sum - small.sum).abs() < 1e-9);
    }

    #[test]
    fn unary_functions_match_std() {
        let xs = vec![0.25, 1.0, 2.5, 9.0];
        let e = Expr::from_values(vec![4], xs.clone()).unwrap();
        let cases: [(UnaryOp, UnaryFn); 6] = [
            (UnaryOp::Abs, f64::abs),
            (UnaryOp::Sqrt, f64::sqrt),
            (UnaryOp::Exp, f64::exp),
            (UnaryOp::Log, f64::ln),
            (UnaryOp::Sin, f64::sin),
            (UnaryOp::Cos, f64::cos),
        ];
        for (op, f) in cases {
            let got = all_values(&e.unary(op));
            for (g, x) in got.iter().zip(&xs) {
                assert_eq!(*g, f(*x), "{op:?}");
            }
        }
        let neg = Expr::from_values(vec![2], vec![-3.0, f64::NAN]).unwrap();
        assert_eq!(all_values(&neg.unary(UnaryOp::Abs))[0], 3.0);
        assert_eq!(neg.unary(UnaryOp::Abs).count().unwrap(), 1);
    }

    #[test]
    fn comparisons_produce_boolean_masks() {
        let a = Expr::from_values(vec![2, 2], vec![1.0, 5.0, f64::NAN, 3.0]).unwrap();
        let b = Expr::from_values(vec![2], vec![2.0, 2.0]).unwrap();
        // Expected values at the three valid positions (index 2 is NaN).
        let cases = [
            (CmpOp::Lt, [1.0, 0.0, 0.0]),
            (CmpOp::Le, [1.0, 0.0, 0.0]),
            (CmpOp::Gt, [0.0, 1.0, 1.0]),
            (CmpOp::Ge, [0.0, 1.0, 1.0]),
            (CmpOp::Eq, [0.0, 0.0, 0.0]),
            (CmpOp::Ne, [1.0, 1.0, 1.0]),
        ];
        for (op, expected) in cases {
            let m = a.compare(op, &b).unwrap();
            assert_eq!(m.count().unwrap(), 3, "{op:?}");
            let raw = all_values(&m);
            assert_eq!(raw[0], expected[0], "{op:?} at 0");
            assert_eq!(raw[1], expected[1], "{op:?} at 1");
            assert_eq!(raw[3], expected[2], "{op:?} at 3");
        }
        let lt = a.compare(CmpOp::Lt, &b).unwrap();
        assert_eq!(lt.sum().unwrap(), 1.0);
        assert!((lt.mean().unwrap().unwrap() - 1.0 / 3.0).abs() < 1e-12);
    }

    #[test]
    fn where_selects_with_broadcasting() {
        let x = Expr::from_values(vec![2, 3], vec![1.0, -2.0, 3.0, -4.0, 5.0, -6.0]).unwrap();
        let zero = Expr::from_values(vec![], vec![0.0]).unwrap();
        let cond = x.compare(CmpOp::Gt, &zero).unwrap();
        let neg = x.arith(ArithOp::Mul, &zero).unwrap();
        // Positive entries take the first branch (x), others 0 * x = 0.
        let sel = Expr::select(&cond, &x, &neg).unwrap();
        assert_eq!(sel.shape(), vec![2, 3]);
        assert_eq!(all_values(&sel), vec![1.0, 0.0, 3.0, 0.0, 5.0, 0.0]);
        // A row of shape (3,) broadcast as the true branch.
        let row = Expr::from_values(vec![3], vec![10.0, 20.0, 30.0]).unwrap();
        let sel2 = Expr::select(&cond, &row, &x).unwrap();
        assert_eq!(all_values(&sel2), vec![10.0, -2.0, 30.0, -4.0, 20.0, -6.0]);
    }

    #[test]
    fn invalid_inputs_stay_invalid() {
        let a = Expr::from_values(vec![3], vec![1.0, f64::NAN, 3.0]).unwrap();
        let b = Expr::from_values(vec![3], vec![1.0, 1.0, 1.0]).unwrap();
        let s = a.arith(ArithOp::Add, &b).unwrap();
        assert_eq!(s.count().unwrap(), 2);
        assert_eq!(s.sum().unwrap(), 6.0);
        assert_eq!(a.valid_mask().sum().unwrap(), 2.0);
        assert_eq!(a.valid_mask().count().unwrap(), 3);
    }
}
