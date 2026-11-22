//! `tptplan array index`: climate indices of a local Zarr field, computed with
//! the lazy pipelines of `tpt_planetary_array::ops::climate`.
//!
//! The field is a Zarr array of shape `[time, lat, lon]`, with monthly time
//! steps. Its coordinates are regular and ascending, given on the command line
//! (the first value and the step, for latitude and longitude). The index is one
//! value per time step: the Niño-3.4 SST anomaly, or its relative form. Only
//! the per-month results are materialised; the field is streamed from the store.

use std::path::Path;
use std::sync::Arc;

use tpt_planetary_array::ops::climate::{nino34, nino34_relative};
use tpt_planetary_array::{Array, Expr, FsStore, Store};

use crate::json;
use crate::{EXIT_RUNTIME, EXIT_USAGE, Failure, OutputFormat};

/// The indices `array index` computes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IndexName {
    /// Monthly SST anomaly averaged over 5°S–5°N, 190°E–240°E.
    Nino34,
    /// The Niño-3.4 anomaly minus the 20°S–20°N mean anomaly.
    Nino34Relative,
}

impl IndexName {
    /// Parses the index argument.
    pub fn parse(name: &str) -> Result<Self, Failure> {
        match name {
            "nino34" => Ok(Self::Nino34),
            "nino34-relative" => Ok(Self::Nino34Relative),
            other => Err(Failure::new(
                EXIT_USAGE,
                format!("unknown index '{other}': expected nino34 or nino34-relative"),
            )),
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Nino34 => "nino34",
            Self::Nino34Relative => "nino34-relative",
        }
    }
}

/// The grid of the field: regular, ascending coordinates in degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Grid {
    /// First latitude, in degrees.
    pub lat_start: f64,
    /// Latitude step, in degrees (positive).
    pub lat_step: f64,
    /// First longitude, in degrees east.
    pub lon_start: f64,
    /// Longitude step, in degrees (positive).
    pub lon_step: f64,
}

impl Grid {
    /// Checks that the steps are positive and finite.
    pub fn validate(&self) -> Result<(), Failure> {
        let ok = |v: f64| v.is_finite();
        if !ok(self.lat_start) || !ok(self.lon_start) {
            return Err(Failure::new(
                EXIT_USAGE,
                "grid starts must be finite numbers".to_string(),
            ));
        }
        let positive = |v: f64| v > 0.0 && v.is_finite();
        if !positive(self.lat_step) || !positive(self.lon_step) {
            return Err(Failure::new(
                EXIT_USAGE,
                "grid steps must be positive: the coordinates must be ascending".to_string(),
            ));
        }
        Ok(())
    }

    fn coords(start: f64, step: f64, n: u64) -> Vec<f64> {
        (0..n).map(|i| start + step * i as f64).collect()
    }
}

fn runtime(message: String) -> Failure {
    Failure::new(EXIT_RUNTIME, message)
}

/// Computes `index` for the field at `input` on `grid`, and returns the text to print.
pub fn index(
    name: IndexName,
    input: &Path,
    grid: Grid,
    format: OutputFormat,
    verbose: u8,
) -> Result<String, Failure> {
    grid.validate()?;
    let parent = input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let node = input.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        Failure::new(
            EXIT_USAGE,
            format!("'{}' has no array name", input.display()),
        )
    })?;
    if verbose > 0 {
        eprintln!("tptplan: opening {}", input.display());
    }
    let store: Arc<dyn Store> =
        Arc::new(FsStore::open(parent).map_err(|e| runtime(format!("cannot open store: {e}")))?);
    let array = Array::open(store, node)
        .map_err(|e| runtime(format!("cannot open array '{}': {e}", input.display())))?;
    let shape = array.shape().to_vec();
    if shape.len() != 3 {
        return Err(Failure::new(
            EXIT_USAGE,
            format!(
                "'{}' has {} dimensions; an index needs a [time, lat, lon] array",
                input.display(),
                shape.len()
            ),
        ));
    }
    let lat = Grid::coords(grid.lat_start, grid.lat_step, shape[1]);
    let lon = Grid::coords(grid.lon_start, grid.lon_step, shape[2]);
    let field = Expr::from_array(array);
    let values = match name {
        IndexName::Nino34 => nino34(&field, &lat, &lon),
        IndexName::Nino34Relative => nino34_relative(&field, &lat, &lon),
    }
    .map_err(|e| runtime(format!("cannot compute the index: {e}")))?;
    Ok(render(name, input, &values, format))
}

fn render(name: IndexName, input: &Path, values: &[f64], format: OutputFormat) -> String {
    match format {
        OutputFormat::Table => {
            let mut out = format!(
                "index: {}\ninput: {}\ntime steps: {}\n",
                name.label(),
                input.display(),
                values.len()
            );
            out.push_str("month value\n");
            for (t, v) in values.iter().enumerate() {
                out.push_str(&format!("{t} {v}\n"));
            }
            out.trim_end().to_string()
        }
        OutputFormat::Json => json::object(&[
            ("index", json::string(name.label())),
            ("input", json::string(&input.display().to_string())),
            ("values", json::f64_array(values)),
        ]),
    }
}
