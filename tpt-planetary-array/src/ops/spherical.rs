//! Spherical area weighting for regular latitude/longitude grids.
//!
//! On a regular grid the area of a cell is proportional to `cos(latitude)`
//! (times the constant `dlat * dlon`), so cell weights are `cos(lat)`
//! broadcast along longitude. [`Expr::weighted_mean`] uses such weights to
//! compute global means.

use crate::error::Result;
use crate::lazy::Expr;
use crate::ops::elementwise::ArithOp;

/// Area weights `cos(latitude)` for latitudes given in degrees, clamped to be
/// non-negative. Pass them as an expression of shape `[nlat, 1]` (or
/// `[nlat]` aligned to the grid's latitude axis) to weight a `[nlat, nlon]`
/// field.
///
/// Requires the `std` feature (`cos` is not available in `core`).
#[cfg(feature = "std")]
pub fn cos_lat_weights(lat_deg: &[f64]) -> Vec<f64> {
    lat_deg
        .iter()
        .map(|&lat| lat.to_radians().cos().max(0.0))
        .collect()
}

impl Expr {
    /// Weighted mean of the valid elements: `sum(x * w) / sum(w)` over the
    /// elements where `x` is valid. `weights` broadcasts against this
    /// expression, so a `[nlat, 1]` weight array weights a `[nlat, nlon]`
    /// field.
    ///
    /// Returns `None` if no element is valid or the total weight is zero.
    /// Weights must be finite.
    pub fn weighted_mean(&self, weights: &Expr) -> Result<Option<f64>> {
        let num = self.arith(ArithOp::Mul, weights)?.stats()?;
        let den = self.valid_mask().arith(ArithOp::Mul, weights)?.stats()?;
        if num.count == 0 || den.sum == 0.0 {
            return Ok(None);
        }
        Ok(Some(num.sum / den.sum))
    }
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;

    fn lats() -> Vec<f64> {
        vec![-60.0, -30.0, 0.0, 30.0, 60.0]
    }

    #[test]
    fn weights_are_cosine_of_latitude() {
        let w = cos_lat_weights(&lats());
        // cos(60) + cos(30) + cos(0) + cos(30) + cos(60) = 2 + sqrt(3).
        let total: f64 = w.iter().sum();
        assert!((total - (2.0 + 3f64.sqrt())).abs() < 1e-12);
        assert!(cos_lat_weights(&[90.0, -90.0]).iter().all(|&x| x >= 0.0));
    }

    #[test]
    fn constant_field_gives_the_constant() {
        let nlat = 5;
        let nlon = 4;
        let field =
            Expr::from_values(vec![nlat, nlon], vec![7.5; nlat as usize * nlon as usize]).unwrap();
        let w = Expr::from_values(vec![nlat, 1], cos_lat_weights(&lats())).unwrap();
        let mean = field.weighted_mean(&w).unwrap().unwrap();
        assert!((mean - 7.5).abs() < 1e-12);
    }

    #[test]
    fn latitude_field_has_zero_weighted_mean_on_symmetric_grid() {
        let nlon = 4u64;
        let mut values = Vec::new();
        for &lat in &lats() {
            for _ in 0..nlon {
                values.push(lat);
            }
        }
        let field = Expr::from_values(vec![5, nlon], values).unwrap();
        let w = Expr::from_values(vec![5, 1], cos_lat_weights(&lats())).unwrap();
        let mean = field.weighted_mean(&w).unwrap().unwrap();
        assert!(mean.abs() < 1e-12, "mean = {mean}");
        // The unweighted mean is also zero here, which is not a useful check
        // on its own; a band field distinguishes weighted from unweighted.
        let band: Vec<f64> = lats()
            .iter()
            .flat_map(|&lat| core::iter::repeat_n(if lat >= 0.0 { 1.0 } else { 0.0 }, 4))
            .collect();
        let band = Expr::from_values(vec![5, nlon], band).unwrap();
        let weighted = band.weighted_mean(&w).unwrap().unwrap();
        let c: Vec<f64> = lats().iter().map(|l| l.to_radians().cos()).collect();
        let expected = (c[2] + c[3] + c[4]) / (c[0] + c[1] + c[2] + c[3] + c[4]);
        assert!((weighted - expected).abs() < 1e-12);
        let unweighted = 3.0 / 5.0;
        assert!((weighted - unweighted).abs() > 1e-3);
    }

    #[test]
    fn invalid_cells_are_excluded() {
        let mut values = vec![5.0; 20];
        values[7] = f64::NAN;
        let field = Expr::from_values(vec![5, 4], values).unwrap();
        let w = Expr::from_values(vec![5, 1], cos_lat_weights(&lats())).unwrap();
        let mean = field.weighted_mean(&w).unwrap().unwrap();
        assert!((mean - 5.0).abs() < 1e-12);
        let all_nan = Expr::from_values(vec![2], vec![f64::NAN, f64::NAN]).unwrap();
        let w2 = Expr::from_values(vec![2], vec![1.0, 1.0]).unwrap();
        assert_eq!(all_nan.weighted_mean(&w2).unwrap(), None);
    }
}
