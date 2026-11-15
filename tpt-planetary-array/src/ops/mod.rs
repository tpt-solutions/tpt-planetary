//! Map algebra on lazy expressions: broadcasting, elementwise operations,
//! comparisons and `where`, axis-wise reductions, spherical area weighting,
//! and rolling and block-resampling along one axis.
//!
//! Every operation builds a node in the [`Expr`](crate::Expr) graph. Nothing
//! is computed until a reduction (for example `mean` or
//! `reduce_axes`) streams the result in blocks, so a chain such as
//! `a.mul_scalar(2.0).add_scalar(1.0)` reads each source chunk once and never
//! materialises an intermediate array.
//!
//! Booleans are `f64` values `1.0` and `0.0`. Invalid elements (fill-valued
//! or NaN) propagate through operations and are skipped by NaN-aware
//! reductions.
//!
//! The default build is scalar. With the `simd` feature, elementwise arithmetic
//! and unary `abs`/`sqrt`, and full-array reductions, use `tpt-simd-vector`
//! kernels (see `simd.rs`); the scalar code remains the reference.

mod nd;
#[cfg(feature = "simd")]
pub(crate) mod simd;
mod source;

pub mod broadcast;
pub mod elementwise;
pub mod reduce;
pub mod spherical;
pub mod window;

pub use elementwise::{ArithOp, CmpOp, UnaryOp};
pub use reduce::{Reduced, Reduction};

#[cfg(all(test, feature = "std"))]
mod tests {
    use alloc::sync::Arc;
    use alloc::vec::Vec;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use crate::Array;
    use crate::dtype::DType;
    use crate::error::Result;
    use crate::lazy::{EvalOptions, Expr};
    use crate::meta::{ArrayMetadata, ZarrFormat};
    use crate::selection::Selection;
    use crate::store::{MemoryStore, Store};

    /// A store that counts chunk reads (`get` calls).
    struct CountingStore {
        inner: MemoryStore,
        gets: AtomicUsize,
    }

    impl Store for CountingStore {
        fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
            self.gets.fetch_add(1, Ordering::SeqCst);
            self.inner.get(key)
        }
        fn set(&self, key: &str, value: &[u8]) -> Result<()> {
            self.inner.set(key, value)
        }
        fn delete(&self, key: &str) -> Result<()> {
            self.inner.delete(key)
        }
        fn list(&self, prefix: &str) -> Result<Vec<alloc::string::String>> {
            self.inner.list(prefix)
        }
    }

    #[test]
    fn fused_chain_makes_one_pass_over_the_source() {
        // 6x8 array in 3x4 chunks: 2 x 2 = 4 chunks.
        let store = Arc::new(CountingStore {
            inner: MemoryStore::new(),
            gets: AtomicUsize::new(0),
        });
        let meta =
            ArrayMetadata::new(ZarrFormat::V2, vec![6, 8], vec![3, 4], DType::FLOAT64).unwrap();
        let array = Array::create(store.clone(), "a", meta).unwrap();
        let values: Vec<f64> = (0..48).map(f64::from).collect();
        array
            .write_f64(&[Selection::ALL, Selection::ALL], &values)
            .unwrap();
        let source = Expr::from_array(array);
        let chain = source.mul_scalar(2.0).add_scalar(1.0);
        // Block of 3 rows x 8 columns: two chunks per block, aligned to chunks.
        let options = EvalOptions { block_elements: 24 };

        store.gets.store(0, Ordering::SeqCst);
        let (bare_stats, bare_exec) = source.stats_with(options).unwrap();
        let bare_gets = store.gets.swap(0, Ordering::SeqCst);

        let (chain_stats, chain_exec) = chain.stats_with(options).unwrap();
        let chain_gets = store.gets.swap(0, Ordering::SeqCst);

        // The chain runs in the same blocks and reads the same chunks as the
        // bare array: no extra passes.
        assert_eq!(chain_exec.blocks, bare_exec.blocks);
        assert_eq!(chain_exec.blocks, 2);
        assert_eq!(chain_gets, bare_gets);
        assert_eq!(chain_gets, 4);
        assert_eq!(bare_stats.count, 48);

        // Result matches the scalar reference 2x + 1.
        let expected_sum: f64 = values.iter().map(|x| 2.0 * x + 1.0).sum();
        assert!((chain_stats.sum - expected_sum).abs() < 1e-9);
        let whole = chain
            .stats_with(EvalOptions {
                block_elements: 1 << 20,
            })
            .unwrap()
            .1;
        assert_eq!(whole.blocks, 1);
    }
}
