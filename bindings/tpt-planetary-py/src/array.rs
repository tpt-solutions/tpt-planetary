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

use tpt_planetary_array::selection::{ResolvedDim, output_shape, resolve_all};
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
    pub(crate) expr: Expr,
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

/// Turns the Python `selection` argument into one `Selection` per dimension,
/// padding missing trailing dimensions with `:`.
fn selections_for(shape: &[u64], selection: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Selection>> {
    let ndim = shape.len();
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
    for (sel, &len) in out.iter_mut().zip(shape) {
        if let Selection::Range(slice) = sel {
            let max_step = (len as i64).max(1);
            if slice.step > max_step {
                slice.step = max_step;
            }
        }
    }
    Ok(out)
}

impl ZarrArray {
    /// Selections for this array's shape; see [`selections_for`].
    fn selections(&self, selection: Option<&Bound<'_, PyAny>>) -> PyResult<Vec<Selection>> {
        selections_for(self.array.shape(), selection)
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

/// A NetCDF-3 file (classic or 64-bit offset) read into memory.
///
/// The file is read once when opened. Variables are decoded on each `read`, as
/// float64 whatever their stored type.
#[pyclass(name = "NetCdf3File", module = "tpt_planetary", frozen)]
pub struct NetCdf3File {
    bytes: Vec<u8>,
}

impl NetCdf3File {
    fn reader(&self) -> PyResult<tpt_planetary_array::netcdf3::Reader<'_>> {
        tpt_planetary_array::netcdf3::Reader::new(&self.bytes).map_err(array_error)
    }
}

/// Opens the NetCDF-3 file at `path` and reads it into memory.
///
/// Raises `OSError` if the file cannot be read and `TptPlanetaryError` if it is
/// not a NetCDF-3 file. NetCDF-4 (HDF5) files are not supported.
#[pyfunction]
pub fn open_netcdf3(path: PathBuf) -> PyResult<NetCdf3File> {
    let bytes = std::fs::read(&path)?;
    let file = NetCdf3File { bytes };
    file.reader()?;
    Ok(file)
}

#[pymethods]
impl NetCdf3File {
    /// Variable names, in file order.
    #[getter]
    fn variables(&self) -> PyResult<Vec<String>> {
        Ok(self
            .reader()?
            .variables()
            .iter()
            .map(|v| v.name.clone())
            .collect())
    }

    /// Dimension names and lengths as a dict, in file order. The unlimited
    /// dimension reports its number of records.
    #[getter]
    fn dimensions<'py>(&self, py: Python<'py>) -> PyResult<Bound<'py, pyo3::types::PyDict>> {
        let dict = pyo3::types::PyDict::new(py);
        for d in self.reader()?.dimensions() {
            dict.set_item(&d.name, d.len)?;
        }
        Ok(dict)
    }

    /// Shape of variable `name` as a tuple of ints.
    fn shape<'py>(&self, py: Python<'py>, name: &str) -> PyResult<Bound<'py, PyTuple>> {
        let shape = self.reader()?.shape(name).map_err(array_error)?;
        PyTuple::new(py, shape)
    }

    /// Reads variable `name`, or a selection of it, into a new float64 NumPy
    /// array. `selection` follows `ZarrArray.read`: an int, a slice, or a tuple
    /// of them, one per dimension. A scalar variable gives a Python float.
    ///
    /// The whole variable is decoded first; a selection only chooses which
    /// values are copied out.
    #[pyo3(signature = (name, selection = None))]
    fn read<'py>(
        &self,
        py: Python<'py>,
        name: &str,
        selection: Option<Bound<'py, PyAny>>,
    ) -> PyResult<Bound<'py, PyAny>> {
        let reader = self.reader()?;
        let shape = reader.shape(name).map_err(array_error)?;
        let shape64: Vec<u64> = shape.iter().map(|&d| d as u64).collect();
        let data = reader.read_f64(name).map_err(array_error)?;
        let sels = selections_for(&shape64, selection.as_ref())?;
        let resolved = resolve_all(&shape64, &sels).map_err(array_error)?;
        let out_shape: Vec<usize> = output_shape(&resolved)
            .iter()
            .map(|&d| d as usize)
            .collect();
        let values = gather(&data, &shape64, &resolved);
        if out_shape.is_empty() {
            let value = values.first().copied().unwrap_or(f64::NAN);
            return Ok(value.into_pyobject(py)?.into_any());
        }
        Ok(values.into_pyarray(py).reshape(out_shape)?.into_any())
    }
}

/// Copies the selected values out of `data`, a row-major array of `shape`, in
/// the output's row-major order. `resolved` comes from `resolve_all`; dimensions
/// dropped by integer indices have a count of one.
fn gather(data: &[f64], shape: &[u64], resolved: &[ResolvedDim]) -> Vec<f64> {
    let total: u64 = resolved.iter().map(|r| r.count).product();
    if total == 0 {
        return Vec::new();
    }
    let nd = shape.len();
    let mut strides = vec![1u64; nd];
    for d in (0..nd.saturating_sub(1)).rev() {
        strides[d] = strides[d + 1] * shape[d + 1];
    }
    let mut out = Vec::with_capacity(total as usize);
    let mut idx = vec![0u64; nd];
    loop {
        let offset: u64 = (0..nd)
            .map(|d| (resolved[d].start + idx[d] * resolved[d].step) * strides[d])
            .sum();
        out.push(data[offset as usize]);
        // Advance the multi-index like an odometer, last dimension fastest.
        let mut d = nd;
        loop {
            if d == 0 {
                return out;
            }
            d -= 1;
            idx[d] += 1;
            if idx[d] < resolved[d].count {
                break;
            }
            idx[d] = 0;
        }
    }
}
