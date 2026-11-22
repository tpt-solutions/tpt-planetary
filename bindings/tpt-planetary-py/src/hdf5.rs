//! Bindings for the read-only HDF5 subset of `tpt-planetary-array` (groups,
//! datasets and attributes of files written with the default HDF5 format, as
//! h5py and netCDF-4 write them).
//!
//! The file is read into memory when it is opened and parsed again for each
//! call, which keeps the class free of self-references.

use std::path::PathBuf;

use numpy::{IntoPyArray, PyArrayMethods};
use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList, PyTuple};

use tpt_planetary_array::hdf5::{Hdf5File, Kind, ObjectKind};

use crate::errors::array_error;

/// An HDF5 file read into memory.
#[pyclass(name = "Hdf5File", module = "tpt_planetary", frozen)]
pub struct Hdf5Data {
    bytes: Vec<u8>,
}

impl Hdf5Data {
    fn file(&self) -> PyResult<Hdf5File<'_>> {
        Hdf5File::open(&self.bytes).map_err(array_error)
    }
}

/// Opens the HDF5 file at `path` and reads it into memory.
///
/// Raises `OSError` if the file cannot be read, and `TptPlanetaryError` if it is
/// not an HDF5 file or uses a feature the reader does not support (dense groups,
/// version 4 layouts, variable-length strings, and similar).
#[pyfunction]
pub fn open_hdf5(path: PathBuf) -> PyResult<Hdf5Data> {
    let bytes = std::fs::read(&path)?;
    let file = Hdf5Data { bytes };
    file.file()?;
    Ok(file)
}

fn kind_name(kind: ObjectKind) -> &'static str {
    match kind {
        ObjectKind::Group => "group",
        ObjectKind::Dataset => "dataset",
        ObjectKind::Other => "other",
    }
}

#[pymethods]
impl Hdf5Data {
    /// Kind of the object at `path`: ``"group"``, ``"dataset"`` or ``"other"``.
    fn kind(&self, path: &str) -> PyResult<&'static str> {
        Ok(kind_name(self.file()?.kind(path).map_err(array_error)?))
    }

    /// Names of the children of the group at `path`, in file order.
    fn list<'py>(&self, py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyList>> {
        let names = self.file()?.list(path).map_err(array_error)?;
        PyList::new(py, names)
    }

    /// Shape of the dataset at `path` as a tuple of ints.
    fn shape<'py>(&self, py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyTuple>> {
        let file = self.file()?;
        let ds = file.dataset(path).map_err(array_error)?;
        PyTuple::new(py, ds.shape())
    }

    /// Reads the dataset at `path` as a float64 array of its shape (a float for a
    /// scalar dataset). Integers and floats of any width are converted.
    fn read<'py>(&self, py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyAny>> {
        let file = self.file()?;
        let ds = file.dataset(path).map_err(array_error)?;
        let shape: Vec<usize> = ds.shape().iter().map(|&d| d as usize).collect();
        let values = ds.read_f64().map_err(array_error)?;
        if shape.is_empty() {
            let value = values.first().copied().unwrap_or(f64::NAN);
            return Ok(value.into_pyobject(py)?.into_any());
        }
        Ok(values.into_pyarray(py).reshape(shape)?.into_any())
    }

    /// Attributes of the group or dataset at `path` as a dict. Numeric attributes
    /// are float64 arrays (a float for a scalar); fixed-length string attributes
    /// are Python strings.
    fn attributes<'py>(&self, py: Python<'py>, path: &str) -> PyResult<Bound<'py, PyDict>> {
        let file = self.file()?;
        let dict = PyDict::new(py);
        for attr in file.attributes(path).map_err(array_error)? {
            let value: Bound<'py, PyAny> = match attr.dtype.kind {
                Kind::String => attr
                    .as_str()
                    .map_err(array_error)?
                    .into_pyobject(py)?
                    .into_any(),
                Kind::Integer { .. } | Kind::Float => {
                    let values = attr.values_f64().map_err(array_error)?;
                    if attr.shape.is_empty() {
                        values[0].into_pyobject(py)?.into_any()
                    } else {
                        let shape: Vec<usize> = attr.shape.iter().map(|&d| d as usize).collect();
                        values.into_pyarray(py).reshape(shape)?.into_any()
                    }
                }
                Kind::Other => continue,
            };
            dict.set_item(attr.name, value)?;
        }
        Ok(dict)
    }
}
