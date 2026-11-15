//! Bindings for `tpt-planetary-array`: opening local Zarr directories, reading
//! slices into NumPy float64 arrays, and lazy reductions.
//!
//! Copies: `read` copies the selected elements out of the chunk buffers into a
//! new `Vec<f64>`, which is then handed to NumPy without a second copy. Integer
//! and other non-float64 stores are converted to float64 during the read.
//! Reductions copy one block at a time (the lazy API's block size) and never
//! materialise the whole array.

use std::path::PathBuf;
use std::sync::Arc;

use numpy::{IntoPyArray, PyArrayMethods};
use pyo3::prelude::*;
use pyo3::types::{PyBool, PySlice, PyTuple};

use tpt_planetary_array::selection::{output_shape, resolve_all};
use tpt_planetary_array::{Array, Expr, FsStore, Selection, Slice, Store};

use crate::errors::{array_error, value_error};

/// A Zarr array on the local filesystem (v2 or v3 metadata).
///
/// Only local directories are supported in this release. Reads return float64
/// NumPy arrays regardless of the stored dtype. Reductions are lazy: they stream
/// through chunks with bounded memory and run with the GIL released.
#[pyclass(name = "ZarrArray", module = "tpt_planetary", frozen)]
pub struct ZarrArray {
    /// Handle used for slice reads.
    array: Array,
    /// Lazy expression over a second handle to the same array, used for reductions.
    expr: Expr,
}

/// Opens the Zarr array stored at a local directory `path`.
///
/// `path` may be a `str` or `os.PathLike`. Raises `FileNotFoundError` if no
/// array metadata exists there and `TptPlanetaryError` for unsupported metadata.
#[pyfunction]
pub fn open_zarr(path: PathBuf) -> PyResult<ZarrArray> {
    let store: Arc<dyn Store> = Arc::new(FsStore::open(&path).map_err(array_error)?);
    let array = Array::open(store.clone(), "").map_err(array_error)?;
    let for_expr = Array::open(store, "").map_err(array_error)?;
    Ok(ZarrArray {
        array,
        expr: Expr::from_array(for_expr),
    })
}

/// Converts one Python index item (int or slice) to a `Selection`.
fn selection_item(item: &Bound<'_, PyAny>) -> PyResult<Selection> {
    if item.is_instance_of::<PyBool>() {
        return Err(value_error("boolean indices are not supported"));
    }
    if let Ok(slice) = item.cast::<PySlice>() {
        let opt_i64 = |name: &str| -> PyResult<Option<i64>> {
            let v = slice.getattr(name)?;
            if v.is_none() {
                Ok(None)
            } else {
                v.extract::<i64>()
                    .map(Some)
                    .map_err(|_| value_error(format!("slice {name} must be an integer or None")))
            }
        };
        let start = opt_i64("start")?;
        let stop = opt_i64("stop")?;
        let step = opt_i64("step")?.unwrap_or(1);
        if step <= 0 {
            return Err(value_error("only positive slice steps are supported"));
        }
        return Ok(Selection::Range(Slice { start, stop, step }));
    }
    match item.extract::<i64>() {
        Ok(i) => Ok(Selection::Index(i)),
        Err(_) => Err(value_error(
            "index must be an integer or a slice (ellipsis and None are not supported)",
        )),
    }
}

impl ZarrArray {
    /// Turns the Python `selection` argument into one `Selection` per dimension,
    /// padding missing trailing dimensions with `:`.
    fn selections(&self, selection: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Selection>> {
        let ndim = self.array.shape().len();
        let items: Vec<Selection> = match selection {
            None => Vec::new(),
            Some(obj) if obj.is_none() => Vec::new(),
            Some(obj) => {
                if let Ok(tuple) = obj.cast::<PyTuple>() {
                    tuple
                        .iter()
                        .map(|it| selection_item(&it))
                        .collect::<PyResult<_>>()?
                } else {
                    vec![selection_item(obj)?]
                }
            }
        };
        if items.len() > ndim {
            return Err(value_error(format!(
                "too many indices: array has {ndim} dimensions, got {}",
                items.len()
            )));
        }
        let mut out = items;
        out.resize(ndim, Selection::ALL);
        // A step at least as long as its dimension selects only the start index,
        // so clamping it is exact. Clamping also keeps the core crate's
        // `step * stride` arithmetic from overflowing on huge steps.
        for (sel, &len) in out.iter_mut().zip(self.array.shape()) {
            if let Selection::Range(slice) = sel {
                let max_step = (len as i64).max(1);
                if slice.step > max_step {
                    slice.step = max_step;
                }
            }
        }
        Ok(out)
    }
}

#[pymethods]
impl ZarrArray {
    /// Array shape as a tuple of ints.
    #[getter]
    fn shape<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.array.shape())
    }

    /// Number of dimensions.
    #[getter]
    fn ndim(&self) -> usize {
        self.array.shape().len()
    }

    /// Chunk shape as a tuple of ints.
    #[getter]
    fn chunks<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, PyTuple>> {
        PyTuple::new(py, self.array.chunk_shape())
    }

    /// Stored dtype in NumPy typestr form, for example `"<f8"`. Reads return
    /// float64 whatever the stored dtype.
    #[getter]
    fn dtype(&self) -> String {
        self.array.dtype().to_v2()
    }

    /// Path of the array inside its store (empty for the store root).
    #[getter]
    fn path(&self) -> String {
        self.array.path().to_string()
    }

    /// Reads a selection into a new float64 NumPy array.
    ///
    /// `selection` is an int, a slice, or a tuple of them, one per dimension
    /// (missing trailing dimensions mean `:`). Integers drop a dimension. Slices
    /// use Python semantics for `start`/`stop` (negative values count from the
    /// end) and must have a positive `step`. With no ranges left, a Python float
    /// is returned. Raises `ValueError` for bad selections and `OSError` for I/O
    /// failures.
    #[pyo3(signature = (selection = None))]
    fn read<'py>(
        &self,
        py: Python<'py>,
        selection: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let sels = self.selections(selection.as_ref())?;
        let resolved = resolve_all(self.array.shape(), &sels).map_err(array_error)?;
        let shape: Vec<usize> = output_shape(&resolved)
            .iter()
            .map(|&d| d as usize)
            .collect();
        let array = &self.array;
        let data = py.detach(|| array.read_f64(&sels)).map_err(array_error)?;
        if shape.is_empty() {
            let value = data.first().copied().unwrap_or(f64::NAN);
            return Ok(value.into_pyobject(py)?.into_any());
        }
        let out = data.into_pyarray(py).reshape(shape)?;
        Ok(out.into_any())
    }

    /// Sum of all elements (lazy, chunk-streamed, GIL released).
    fn sum(&self, py: Python<'_>) -> PyResult<f64> {
        let expr = &self.expr;
        py.detach(|| expr.sum()).map_err(array_error)
    }

    /// Mean of all elements, or `None` for an empty array.
    fn mean(&self, py: Python<'_>) -> PyResult<Option<f64>> {
        let expr = &self.expr;
        py.detach(|| expr.mean()).map_err(array_error)
    }

    /// Minimum element, or `None` for an empty array.
    fn min(&self, py: Python<'_>) -> PyResult<Option<f64>> {
        let expr = &self.expr;
        py.detach(|| expr.min()).map_err(array_error)
    }

    /// Maximum element, or `None` for an empty array.
    fn max(&self, py: Python<'_>) -> PyResult<Option<f64>> {
        let expr = &self.expr;
        py.detach(|| expr.max()).map_err(array_error)
    }

    fn __repr__(&self) -> String {
        format!(
            "ZarrArray(path={:?}, shape={:?}, chunks={:?}, dtype={:?})",
            self.array.path(),
            self.array.shape(),
            self.array.chunk_shape(),
            self.array.dtype().to_v2()
        )
    }
}
