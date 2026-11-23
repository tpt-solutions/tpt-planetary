//! The Niño-3.4 indices on a lazy field read from a Zarr store, checked against
//! a plain computation of the same quantities.

use std::sync::Arc;

use tpt_planetary_array::meta::NodeMetadata;
use tpt_planetary_array::meta::v3::parse_node;
use tpt_planetary_array::ops::climate::{nino34, nino34_relative};
use tpt_planetary_array::{Array, Expr, MemoryStore, Store};

const NT: usize = 60; // five years of months
const LAT: usize = 13; // -30 .. 30 in 5-degree steps
const LON: usize = 36; // 0 .. 350 in 10-degree steps

fn lat_coords() -> Vec<f64> {
    (0..LAT).map(|i| -30.0 + 5.0 * i as f64).collect()
}

fn lon_coords() -> Vec<f64> {
    (0..LON).map(|j| 10.0 * j as f64).collect()
}

/// A Zarr v3 array of shape [NT, LAT, LON], one chunk per year, holding `data`.
fn zarr_field(data: &[f64]) -> Expr {
    let json = format!(
        r#"{{
        "zarr_format": 3, "node_type": "array",
        "shape": [{NT}, {LAT}, {LON}], "data_type": "float64",
        "chunk_grid": {{"name": "regular", "configuration": {{"chunk_shape": [12, {LAT}, {LON}]}}}},
        "chunk_key_encoding": {{"name": "default", "configuration": {{"separator": "/"}}}},
        "fill_value": 0.0,
        "codecs": [{{"name": "bytes", "configuration": {{"endian": "little"}}}}]
        }}"#
    );
    let NodeMetadata::Array(meta) = parse_node(json.as_bytes()).unwrap() else {
        panic!("expected an array");
    };
    let store: Arc<dyn Store> = Arc::new(MemoryStore::new());
    let array = Array::create(store, "sst", meta).unwrap();
    array.write_f64(&[], data).unwrap();
    Expr::from_array(array)
}

/// The field value at month `t`, latitude index `i`, longitude index `j`.
fn value(t: usize, i: usize, j: usize, with_event: bool) -> f64 {
    let lat = lat_coords()[i];
    let lon = lon_coords()[j];
    let season = 1.2 * (2.0 * std::f64::consts::PI * t as f64 / 12.0 + lat.to_radians()).cos();
    if !with_event {
        return season + 0.1 * (lon / 40.0).sin();
    }
    // A basin-wide warming trend, a warm event in the central Pacific, and the seasonal cycle.
    let event = 0.8
        * (2.0 * std::f64::consts::PI * t as f64 / 40.0).sin()
        * (-((lat / 8.0).powi(2)) - ((lon - 215.0) / 40.0).powi(2)).exp();
    let warming = 0.02 * t as f64;
    season + event + warming + 0.1 * (lon / 40.0).sin()
}

fn field_data(with_event: bool) -> Vec<f64> {
    let mut data = Vec::with_capacity(NT * LAT * LON);
    for t in 0..NT {
        for i in 0..LAT {
            for j in 0..LON {
                data.push(value(t, i, j, with_event));
            }
        }
    }
    data
}

/// Monthly anomaly by hand: subtract the mean of each calendar month over the record.
fn anomaly_by_hand(data: &[f64]) -> Vec<f64> {
    let cell = LAT * LON;
    let mut clim = vec![0.0; 12 * cell];
    let mut count = [0usize; 12];
    for t in 0..NT {
        count[t % 12] += 1;
        for k in 0..cell {
            clim[(t % 12) * cell + k] += data[t * cell + k];
        }
    }
    let mut out = data.to_vec();
    for t in 0..NT {
        for k in 0..cell {
            out[t * cell + k] -= clim[(t % 12) * cell + k] / count[t % 12] as f64;
        }
    }
    out
}

/// Area-weighted mean over a box for each month, by hand.
fn box_mean_by_hand(anom: &[f64], lat_range: (f64, f64), lon_range: (f64, f64)) -> Vec<f64> {
    let cell = LAT * LON;
    (0..NT)
        .map(|t| {
            let (mut num, mut den) = (0.0, 0.0);
            for i in 0..LAT {
                let lat = lat_coords()[i];
                if lat < lat_range.0 || lat > lat_range.1 {
                    continue;
                }
                let w = lat.to_radians().cos();
                for j in 0..LON {
                    let lon = lon_coords()[j];
                    if lon < lon_range.0 || lon > lon_range.1 {
                        continue;
                    }
                    num += anom[t * cell + i * LON + j] * w;
                    den += w;
                }
            }
            num / den
        })
        .collect()
}

#[test]
fn nino34_matches_a_direct_computation_on_a_lazy_zarr_field() {
    let data = field_data(true);
    let sst = zarr_field(&data);
    let index = nino34(&sst, &lat_coords(), &lon_coords()).unwrap();
    let want = box_mean_by_hand(&anomaly_by_hand(&data), (-5.0, 5.0), (190.0, 240.0));
    assert_eq!(index.len(), NT);
    for (t, (g, w)) in index.iter().zip(&want).enumerate() {
        assert!((g - w).abs() < 1e-10, "month {t}: {g} vs {w}");
    }
    // The warm event is in the box, so the index is not trivially zero.
    assert!(index.iter().map(|v| v.abs()).fold(0.0, f64::max) > 0.1);
}

#[test]
fn a_purely_seasonal_field_has_zero_anomaly_and_zero_index() {
    // The seasonal cycle is exactly periodic, so its anomaly is zero, and so is
    // the index (the lon term is constant in time too).
    let mut data = Vec::with_capacity(NT * LAT * LON);
    for t in 0..NT {
        for i in 0..LAT {
            for j in 0..LON {
                data.push(value(t % 12, i, j, false));
            }
        }
    }
    let sst = zarr_field(&data);
    let index = nino34(&sst, &lat_coords(), &lon_coords()).unwrap();
    assert!(index.iter().all(|v| v.abs() < 1e-12), "{index:?}");
}

#[test]
fn relative_index_removes_the_basin_wide_warming() {
    let data = field_data(true);
    let sst = zarr_field(&data);
    let rel = nino34_relative(&sst, &lat_coords(), &lon_coords()).unwrap();
    let anom = anomaly_by_hand(&data);
    let nino = box_mean_by_hand(&anom, (-5.0, 5.0), (190.0, 240.0));
    let tropics = box_mean_by_hand(&anom, (-20.0, 20.0), (0.0, 350.0));
    for t in 0..NT {
        assert!((rel[t] - (nino[t] - tropics[t])).abs() < 1e-10, "month {t}");
    }
}

#[test]
fn lazy_expressions_match_the_eager_indices() {
    use tpt_planetary_array::lazy::Region;
    use tpt_planetary_array::ops::climate::{nino34_expr, nino34_relative_expr};

    let data = field_data(true);
    let field = zarr_field(&data);
    let lat = lat_coords();
    let lon = lon_coords();
    let eager = nino34(&field, &lat, &lon).unwrap();
    let eager_rel = nino34_relative(&field, &lat, &lon).unwrap();

    let whole = |expr: &Expr| {
        let shape = expr.shape();
        assert_eq!(shape, vec![NT as u64], "one value per month");
        expr.eval_region(&Region {
            start: vec![0],
            count: vec![NT as u64],
            step: vec![1],
        })
        .unwrap()
    };
    let lazy = whole(&nino34_expr(&field, &lat, &lon).unwrap());
    let lazy_rel = whole(&nino34_relative_expr(&field, &lat, &lon).unwrap());
    for t in 0..NT {
        assert!(lazy.valid[t]);
        assert!((lazy.values[t] - eager[t]).abs() < 1e-12, "month {t}");
        assert!(
            (lazy_rel.values[t] - eager_rel[t]).abs() < 1e-12,
            "month {t}"
        );
    }
}

#[test]
fn a_lazy_index_can_be_evaluated_in_parts() {
    use tpt_planetary_array::lazy::Region;
    use tpt_planetary_array::ops::climate::nino34_expr;

    let data = field_data(true);
    let field = zarr_field(&data);
    let expr = nino34_expr(&field, &lat_coords(), &lon_coords()).unwrap();
    let whole = expr
        .eval_region(&Region {
            start: vec![0],
            count: vec![NT as u64],
            step: vec![1],
        })
        .unwrap();
    // Two regions, the second starting mid-series, must give the same values.
    let first = expr
        .eval_region(&Region {
            start: vec![0],
            count: vec![25],
            step: vec![1],
        })
        .unwrap();
    let second = expr
        .eval_region(&Region {
            start: vec![25],
            count: vec![(NT - 25) as u64],
            step: vec![1],
        })
        .unwrap();
    let joined: Vec<f64> = first.values.iter().chain(&second.values).copied().collect();
    assert_eq!(joined, whole.values);
}
