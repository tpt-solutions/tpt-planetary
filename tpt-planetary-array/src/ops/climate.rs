//! Climate-index pipelines on lazy expressions: an anomaly against a monthly
//! climatology, an area-weighted box mean on a regular latitude/longitude
//! grid, and the Niño-3.4 indices built from them.
//!
//! The field is a lazy expression of shape `[time, lat, lon]` (for example
//! `Expr::from_array` on a Zarr array). Anomalies and the slice are lazy; the
//! reductions stream through the chunks with bounded memory, and only their
//! small results (one value per time step) are materialised.
//!
//! Conventions: latitudes are in degrees and increasing along the `lat` axis;
//! longitudes are in degrees east, `[0, 360)`, and increasing along the `lon`
//! axis. A box does not wrap across longitude 0. The time axis is monthly, so
//! the climatology period is 12.

use alloc::format;
use alloc::vec;
use alloc::vec::Vec;

use crate::error::{Error, Result};
use crate::lazy::{Block, Expr, NodeOp, Region};
use crate::ops::elementwise::ArithOp;
use crate::ops::reduce::Reduction;
use crate::selection::{Selection, Slice};

/// Months in a seasonal cycle.
pub const MONTHS: u64 = 12;

/// Index range `[first, end)` of the grid points `coords` (sorted ascending)
/// that lie in `[lo, hi]`. Fails if the box selects no points.
fn box_indices(coords: &[f64], lo: f64, hi: f64, what: &str) -> Result<(usize, usize)> {
    let first = coords.iter().position(|&c| c >= lo && c <= hi);
    let Some(first) = first else {
        return Err(Error::InvalidSelection(format!(
            "no {what} grid points in [{lo}, {hi}]"
        )));
    };
    let end = coords[first..]
        .iter()
        .position(|&c| c > hi)
        .map_or(coords.len(), |k| first + k);
    Ok((first, end))
}

/// Area-weighted mean of `field` over a latitude/longitude box, for every time
/// step, as one value per time step.
///
/// `field` has shape `[time, lat, lon]`; `lat_deg` and `lon_deg` give its
/// coordinates. The weight of a grid point is `cos(latitude)`, the cell area
/// on a regular grid. Invalid (fill-valued or NaN) points are left out of both
/// the numerator and the denominator, so a time step with no valid point in the
/// box is NaN.
///
/// # Errors
///
/// Fails if the field is not three-dimensional, the coordinate lengths do not
/// match its shape, or the box contains no grid point.
pub fn box_mean_series(
    field: &Expr,
    lat_deg: &[f64],
    lon_deg: &[f64],
    lat_range: (f64, f64),
    lon_range: (f64, f64),
) -> Result<Vec<f64>> {
    let shape = field.shape();
    if shape.len() != 3 {
        return Err(Error::InvalidSelection(
            "a box mean needs a [time, lat, lon] field".into(),
        ));
    }
    if shape[1] != lat_deg.len() as u64 || shape[2] != lon_deg.len() as u64 {
        return Err(Error::InvalidSelection(
            "coordinate lengths do not match the field shape".into(),
        ));
    }
    let (la, lb) = box_indices(lat_deg, lat_range.0, lat_range.1, "latitude")?;
    let (oa, ob) = box_indices(lon_deg, lon_range.0, lon_range.1, "longitude")?;
    let boxed = field.slice(&[
        Selection::ALL,
        Selection::Range(Slice::range(la as i64, lb as i64)),
        Selection::Range(Slice::range(oa as i64, ob as i64)),
    ])?;
    let weights: Vec<f64> = lat_deg[la..lb]
        .iter()
        .map(|&lat| lat.to_radians().cos().max(0.0))
        .collect();
    let w = Expr::from_values(vec![1, (lb - la) as u64, 1], weights)?;
    let num = boxed
        .arith(ArithOp::Mul, &w)?
        .reduce_axes(&[1, 2], Reduction::Sum, true)?;
    let den =
        boxed
            .valid_mask()
            .arith(ArithOp::Mul, &w)?
            .reduce_axes(&[1, 2], Reduction::Sum, false)?;
    Ok(num
        .values
        .iter()
        .zip(&den.values)
        .map(|(&n, &d)| if d == 0.0 { f64::NAN } else { n / d })
        .collect())
}

/// Monthly anomaly of a `[time, lat, lon]` field: each value minus the mean of
/// its calendar month over the full years in the record (see
/// [`Expr::anomaly`]). The result is lazy.
///
/// # Errors
///
/// Propagates the errors of [`Expr::anomaly`].
pub fn monthly_anomaly(field: &Expr) -> Result<Expr> {
    field.anomaly(0, MONTHS)
}

/// The Niño-3.4 index: the monthly SST anomaly averaged over 5°S–5°N,
/// 170°W–120°W (190° to 240° east), one value per month.
///
/// # Errors
///
/// Propagates the errors of [`monthly_anomaly`] and [`box_mean_series`].
pub fn nino34(sst: &Expr, lat_deg: &[f64], lon_deg: &[f64]) -> Result<Vec<f64>> {
    let anom = monthly_anomaly(sst)?;
    box_mean_series(&anom, lat_deg, lon_deg, (-5.0, 5.0), (190.0, 240.0))
}

/// The relative Niño-3.4 index: the Niño-3.4 anomaly minus the tropical mean
/// anomaly (20°S–20°N, all longitudes) of the same month. The tropical mean
/// removes the basin-wide warming, which is the idea behind the relative ENSO
/// indices. It reads the same field twice, once per region.
///
/// # Errors
///
/// Propagates the errors of [`monthly_anomaly`] and [`box_mean_series`].
pub fn nino34_relative(sst: &Expr, lat_deg: &[f64], lon_deg: &[f64]) -> Result<Vec<f64>> {
    let anom = monthly_anomaly(sst)?;
    let nino = box_mean_series(&anom, lat_deg, lon_deg, (-5.0, 5.0), (190.0, 240.0))?;
    let tropics = box_mean_series(&anom, lat_deg, lon_deg, (-20.0, 20.0), (0.0, 360.0))?;
    Ok(nino.iter().zip(&tropics).map(|(n, t)| n - t).collect())
}

/// A lazy area-weighted box mean over a `[time, lat, lon]` field that has
/// already been sliced to the box. Output is one value per time step.
#[derive(Debug)]
struct BoxMean {
    /// The boxed field, shape `[time, box lat, box lon]`.
    boxed: Expr,
    /// Cosine-of-latitude weight of each latitude row in the box.
    weights: Vec<f64>,
}

impl NodeOp for BoxMean {
    fn shape(&self) -> Vec<u64> {
        vec![self.boxed.shape()[0]]
    }

    fn eval_region(&self, region: &Region) -> Result<Block> {
        let shape = self.boxed.shape();
        let (nlat, nlon) = (shape[1] as usize, shape[2] as usize);
        let count = region.count[0];
        // The same time indices, with the whole box.
        let input = Region {
            start: vec![region.start[0], 0, 0],
            count: vec![count, shape[1], shape[2]],
            step: vec![region.step[0], 1, 1],
        };
        let block = self.boxed.eval_region(&input)?;
        let per_time = nlat * nlon;
        let mut values = Vec::with_capacity(count as usize);
        let mut valid = Vec::with_capacity(count as usize);
        for t in 0..count as usize {
            let mut num = 0.0;
            let mut den = 0.0;
            for (i, &w) in self.weights.iter().enumerate() {
                for j in 0..nlon {
                    let k = t * per_time + i * nlon + j;
                    if block.valid[k] {
                        num += block.values[k] * w;
                        den += w;
                    }
                }
            }
            let value = if den == 0.0 { f64::NAN } else { num / den };
            values.push(value);
            valid.push(!value.is_nan());
        }
        Ok(Block { values, valid })
    }
}

/// The area-weighted box mean of a `[time, lat, lon]` field as a lazy
/// expression of shape `[time]`. Its values are those of [`box_mean_series`];
/// nothing is read until the expression is evaluated.
///
/// # Errors
///
/// The same as [`box_mean_series`].
pub fn box_mean_expr(
    field: &Expr,
    lat_deg: &[f64],
    lon_deg: &[f64],
    lat_range: (f64, f64),
    lon_range: (f64, f64),
) -> Result<Expr> {
    let shape = field.shape();
    if shape.len() != 3 {
        return Err(Error::InvalidSelection(
            "a box mean needs a [time, lat, lon] field".into(),
        ));
    }
    if shape[1] != lat_deg.len() as u64 || shape[2] != lon_deg.len() as u64 {
        return Err(Error::InvalidSelection(
            "coordinate lengths do not match the field shape".into(),
        ));
    }
    let (la, lb) = box_indices(lat_deg, lat_range.0, lat_range.1, "latitude")?;
    let (oa, ob) = box_indices(lon_deg, lon_range.0, lon_range.1, "longitude")?;
    let boxed = field.slice(&[
        Selection::ALL,
        Selection::Range(Slice::range(la as i64, lb as i64)),
        Selection::Range(Slice::range(oa as i64, ob as i64)),
    ])?;
    let weights: Vec<f64> = lat_deg[la..lb]
        .iter()
        .map(|&lat| lat.to_radians().cos().max(0.0))
        .collect();
    Ok(Expr::from_op(BoxMean { boxed, weights }))
}

/// The Niño-3.4 index as a lazy `[time]` expression: the monthly anomaly
/// averaged over 5°S–5°N, 190°E–240°E (see [`nino34`]).
///
/// # Errors
///
/// Propagates the errors of [`monthly_anomaly`] and [`box_mean_expr`].
pub fn nino34_expr(field: &Expr, lat_deg: &[f64], lon_deg: &[f64]) -> Result<Expr> {
    let anom = monthly_anomaly(field)?;
    box_mean_expr(&anom, lat_deg, lon_deg, (-5.0, 5.0), (190.0, 240.0))
}

/// The relative Niño-3.4 index as a lazy `[time]` expression (see
/// [`nino34_relative`]).
///
/// # Errors
///
/// Propagates the errors of [`monthly_anomaly`] and [`box_mean_expr`].
pub fn nino34_relative_expr(field: &Expr, lat_deg: &[f64], lon_deg: &[f64]) -> Result<Expr> {
    let anom = monthly_anomaly(field)?;
    let nino = box_mean_expr(&anom, lat_deg, lon_deg, (-5.0, 5.0), (190.0, 240.0))?;
    let tropics = box_mean_expr(&anom, lat_deg, lon_deg, (-20.0, 20.0), (0.0, 360.0))?;
    nino.arith(ArithOp::Sub, &tropics)
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn coords(start: f64, step: f64, n: usize) -> Vec<f64> {
        (0..n).map(|i| start + step * i as f64).collect()
    }

    #[test]
    fn box_indices_select_the_inclusive_range() {
        let lat = coords(-30.0, 5.0, 13); // -30 .. 30
        assert_eq!(box_indices(&lat, -5.0, 5.0, "lat").unwrap(), (5, 8));
        assert_eq!(box_indices(&lat, -4.0, 4.0, "lat").unwrap(), (6, 7));
        assert!(box_indices(&lat, 31.0, 40.0, "lat").is_err());
    }

    #[test]
    fn weighted_mean_of_a_constant_field_is_that_constant() {
        let lat = coords(-10.0, 5.0, 5); // -10 .. 10
        let lon = coords(180.0, 10.0, 8);
        let nt = 3;
        let values = vec![7.5; nt * lat.len() * lon.len()];
        let field = Expr::from_values(vec![nt as u64, 5, 8], values).unwrap();
        let mean = box_mean_series(&field, &lat, &lon, (-10.0, 10.0), (180.0, 250.0)).unwrap();
        assert_eq!(mean.len(), nt);
        for m in mean {
            assert!((m - 7.5).abs() < 1e-12, "{m}");
        }
    }

    #[test]
    fn weights_follow_cosine_of_latitude() {
        // Two latitudes with different values; the weighted mean favours the
        // equator (cos 0 = 1) over 60 degrees (cos 60 = 0.5).
        let lat = vec![-60.0, 0.0];
        let lon = vec![0.0];
        let field = Expr::from_values(vec![1, 2, 1], vec![0.0, 3.0]).unwrap();
        let mean = box_mean_series(&field, &lat, &lon, (-60.0, 0.0), (0.0, 0.0)).unwrap();
        // (0 * 0.5 + 3 * 1) / (0.5 + 1) = 2.
        assert!((mean[0] - 2.0).abs() < 1e-12);
    }
}
