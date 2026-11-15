//! End-to-end test of the data-flow example in spec section 7, scaled down.
//!
//! The spec's pipeline is: open a Zarr store, build a lazy query for a
//! bounded lat/lon/time box, stream only the chunks it touches, reduce them
//! with cos(lat) area weights, convert an epoch to TDB and compute the
//! Earth-Sun distance, then export the 2-D result.
//!
//! This test runs the same steps on a synthetic dataset that is small enough
//! to finish in well under a second. The dataset is written to a temporary
//! directory as Zarr v2 with uncompressed float64 chunks.
//!
//! Dataset (time x lat x lon) = 24 x 18 x 36, chunks 6 x 9 x 12:
//! - latitude centres `lat_i = -85 + 10 i` degrees (i = 0..18);
//! - longitude centres `lon_j = 5 + 10 j` degrees (j = 0..36);
//! - field `sst(t, i, j) = lat_i + t`. The latitude part is the spec's
//!   "value = lat_deg" analogue; the `+ t` term makes the time slice matter.
//!
//! Box: time 12..24 (12 steps), lat indices 6..14 (lat -25 to 45 degrees,
//! 8 rows), lon indices 14..24 (lon 145 to 235 degrees, 10 columns). This is
//! a box in the Pacific longitudes, not a land-sea mask.
//!
//! Expected values (all derived analytically in the test below):
//! - Unweighted mean over the box: mean(lat) + mean(t) = 10 + 17.5 = 27.5.
//! - cos(lat)-weighted mean: `sum_i w_i lat_i / sum_i w_i + mean(t)`, which is
//!   about 25.83. The weighted and unweighted values differ by about 1.7, so
//!   the weights are really exercised.
//!
//! Tolerances: the field is exact in f64 and the sums have about 1000 terms of
//! magnitude below 100, so rounding error is about 1e-12. The checks use an
//! absolute tolerance of 1e-9.
//!
//! Astronomy checks use the analytic helpers only, because no SPK kernel is
//! bundled. See the comments at each check.

use std::collections::BTreeSet;
use std::env;
use std::fs;
use std::path::PathBuf;
use std::process;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use tpt_planetary_array::meta::{ArrayMetadata, ZarrFormat};
use tpt_planetary_array::ops::Reduction;
use tpt_planetary_array::ops::spherical::cos_lat_weights;
use tpt_planetary_array::{Array, DType, Expr, FsStore, Selection, Slice, Store};
use tpt_planetary_astro::ephemeris::{AU_KM, earth_sun_distance_au_approx};
use tpt_planetary_astro::time::{J2000_JD, tdb_minus_tt_seconds, tdb_to_tt, tt_to_tdb};
use tpt_planetary_core::calendar::CivilDate;
use tpt_planetary_core::leap::EMBEDDED;
use tpt_planetary_core::scale::{CivilTime, Epoch, Tt, Utc};

const NT: usize = 24;
const NLAT: usize = 18;
const NLON: usize = 36;
const CHUNK: [u64; 3] = [6, 9, 12];

/// Box in grid indices: time, lat, lon (half-open).
const T_RANGE: (i64, i64) = (12, 24);
const LAT_RANGE: (i64, i64) = (6, 14);
const LON_RANGE: (i64, i64) = (14, 24);

/// Absolute tolerance for the array checks (see the module comment).
const TOL: f64 = 1e-9;

/// Shape of the 2-D result: (lat, lon) inside the box.
const RESULT_SHAPE: [u64; 2] = [8, 10];

fn lat_deg(i: usize) -> f64 {
    -85.0 + 10.0 * i as f64
}

fn lon_deg(j: usize) -> f64 {
    5.0 + 10.0 * j as f64
}

fn sst(t: usize, i: usize) -> f64 {
    lat_deg(i) + t as f64
}

fn is_chunk_key(key: &str) -> bool {
    key.starts_with("sst/")
        && key
            .rsplit('/')
            .next()
            .is_some_and(|leaf| leaf.starts_with(|c: char| c.is_ascii_digit()))
}

/// A store that records which `sst` chunk keys were read, so the test can
/// check that the lazy query touched only the chunks it needs.
struct CountingStore {
    inner: FsStore,
    chunk_reads: Mutex<BTreeSet<String>>,
}

impl CountingStore {
    fn take_reads(&self) -> BTreeSet<String> {
        std::mem::take(&mut *self.chunk_reads.lock().expect("lock"))
    }
}

impl Store for CountingStore {
    fn get(&self, key: &str) -> tpt_planetary_array::Result<Option<Vec<u8>>> {
        if is_chunk_key(key) {
            self.chunk_reads
                .lock()
                .expect("lock")
                .insert(key.to_string());
        }
        self.inner.get(key)
    }

    fn set(&self, key: &str, value: &[u8]) -> tpt_planetary_array::Result<()> {
        self.inner.set(key, value)
    }

    fn delete(&self, key: &str) -> tpt_planetary_array::Result<()> {
        self.inner.delete(key)
    }

    fn list(&self, prefix: &str) -> tpt_planetary_array::Result<Vec<String>> {
        self.inner.list(prefix)
    }
}

/// Removes the temporary store directory on drop, including after a panic.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn assert_close(actual: f64, expected: f64, tol: f64, what: &str) {
    assert!(
        (actual - expected).abs() <= tol,
        "{what}: got {actual}, expected {expected} (tolerance {tol})"
    );
}

/// Solves Kepler's equation `E - e sin E = M` and returns `r = 1 - e cos E`
/// in AU. Used only as a reference for the low-precision helper.
fn kepler_distance_au(mean_anomaly_rad: f64, e: f64) -> f64 {
    let m = mean_anomaly_rad.rem_euclid(std::f64::consts::TAU);
    let mut ecc = m;
    for _ in 0..50 {
        let delta = (ecc - e * ecc.sin() - m) / (1.0 - e * ecc.cos());
        ecc -= delta;
        if delta.abs() < 1e-15 {
            break;
        }
    }
    1.0 - e * ecc.cos()
}

#[test]
fn spec_section7_dataflow_scaled_down() {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock")
        .as_nanos();
    let root = env::temp_dir().join(format!("tpt-planetary-dataflow-{}-{nanos}", process::id()));
    let _guard = TempDir(root.clone());

    let counting = Arc::new(CountingStore {
        inner: FsStore::open(&root).expect("open store"),
        chunk_reads: Mutex::new(BTreeSet::new()),
    });
    let store: Arc<dyn Store> = counting.clone();

    // ---- Write the synthetic dataset -------------------------------------
    let sst_meta = ArrayMetadata::new(
        ZarrFormat::V2,
        vec![NT as u64, NLAT as u64, NLON as u64],
        CHUNK.to_vec(),
        DType::FLOAT64,
    )
    .expect("metadata");
    let sst_out = Array::create(store.clone(), "sst", sst_meta).expect("create sst");
    let mut values = Vec::with_capacity(NT * NLAT * NLON);
    for t in 0..NT {
        for i in 0..NLAT {
            values.extend(std::iter::repeat_n(sst(t, i), NLON));
        }
    }
    sst_out
        .write_f64(&[Selection::ALL, Selection::ALL, Selection::ALL], &values)
        .expect("write sst");

    let coord = |name: &str, v: Vec<f64>| {
        let meta = ArrayMetadata::new(
            ZarrFormat::V2,
            vec![v.len() as u64],
            vec![v.len() as u64],
            DType::FLOAT64,
        )
        .expect("coord metadata");
        let a = Array::create(store.clone(), name, meta).expect("create coord");
        a.write_f64(&[Selection::ALL], &v).expect("write coord");
    };
    coord("lat", (0..NLAT).map(lat_deg).collect());
    coord("lon", (0..NLON).map(lon_deg).collect());
    coord("time", (0..NT).map(|t| t as f64).collect());

    // ---- (i) Open the dataset and build the lazy query -------------------
    let sst = Array::open(store.clone(), "sst").expect("open sst");
    let lat_coord = Array::open(store.clone(), "lat")
        .expect("open lat")
        .read_f64(&[Selection::ALL])
        .expect("read lat");
    assert_eq!(lat_coord.len(), NLAT);
    assert_close(lat_coord[0], -85.0, 0.0, "first latitude");

    let (t0, t1) = T_RANGE;
    let (a0, a1) = LAT_RANGE;
    let (o0, o1) = LON_RANGE;
    let box_expr = Expr::from_array(sst)
        .slice(&[
            Selection::Range(Slice::range(t0, t1)),
            Selection::Range(Slice::range(a0, a1)),
            Selection::Range(Slice::range(o0, o1)),
        ])
        .expect("slice");
    assert_eq!(box_expr.shape(), vec![12, 8, 10]);

    // Building the expression reads no chunk data (only metadata at open).
    assert!(counting.take_reads().is_empty(), "lazy build read chunks");

    // ---- (ii) Lazy reductions over the box -------------------------------
    // Cos(lat) weights come from the stored latitude coordinate, shape
    // [nlat_box, 1], which broadcasts against [nt, nlat_box, nlon_box].
    let box_lats: Vec<f64> = lat_coord[a0 as usize..a1 as usize].to_vec();
    let weights = Expr::from_values(vec![box_lats.len() as u64, 1], cos_lat_weights(&box_lats))
        .expect("weights");

    let count = box_expr.count().expect("count");
    assert_eq!(count, 12 * 8 * 10, "valid element count");

    let plain_mean = box_expr.mean().expect("mean").expect("non-empty box");

    let weighted_mean = box_expr
        .weighted_mean(&weights)
        .expect("weighted mean")
        .expect("non-zero weight");

    // Analytic expectations (see module comment).
    let w: Vec<f64> = box_lats.iter().map(|l| l.to_radians().cos()).collect();
    let sum_w: f64 = w.iter().sum();
    let sum_wl: f64 = w.iter().zip(&box_lats).map(|(wi, li)| wi * li).sum();
    let mean_t = ((t0..t1).sum::<i64>() as f64) / ((t1 - t0) as f64);
    let expected_weighted = sum_wl / sum_w + mean_t;
    let expected_plain = box_lats.iter().sum::<f64>() / box_lats.len() as f64 + mean_t;

    assert_close(plain_mean, expected_plain, TOL, "unweighted box mean");
    assert_close(plain_mean, 27.5, TOL, "unweighted box mean (closed form)");
    assert_close(
        weighted_mean,
        expected_weighted,
        TOL,
        "cos(lat)-weighted box mean",
    );
    assert!(
        (weighted_mean - plain_mean).abs() > 1.0,
        "weighted and unweighted means should differ for this box"
    );

    // Only the chunks that intersect the box should have been streamed:
    // time chunks 2 and 3 (t 12..23), lat chunks 0 and 1 (lat 6..13),
    // lon chunk 1 (lon 14..23). Chunk keys are `sst/<t>.<lat>.<lon>`.
    let expected_chunks: BTreeSet<String> = ["sst/2.0.1", "sst/2.1.1", "sst/3.0.1", "sst/3.1.1"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(counting.take_reads(), expected_chunks, "chunks streamed");

    // ---- (iii) Timestamp of the last time step: UTC -> TT -> TDB --------
    // The last time step is labelled 2025-06-15T12:00:00 UTC.
    let date = CivilDate::new(2025, 6, 15);
    let utc = Epoch::<Utc>::from_civil(
        CivilTime {
            date,
            hour: 12,
            minute: 0,
            second: 0.0,
        },
        &EMBEDDED,
    )
    .expect("utc epoch");
    let tt: Epoch<Tt> = utc.to_scale(&EMBEDDED).expect("utc to tt");

    // Leap-second check: TT - UTC = (TAI - UTC) + 32.184 s = 37 + 32.184
    // = 69.184 s on this date (the table entry for 2017-01-01 is 37 s).
    let tt_expected = Epoch::<Tt>::from_civil(CivilTime {
        date,
        hour: 12,
        minute: 1,
        second: 9.184,
    })
    .expect("tt epoch");
    let leap_err_s = tt.jd().seconds_since(tt_expected.jd()).abs();
    // Two-part JD arithmetic is good to far better than 1 microsecond here.
    assert!(leap_err_s < 1e-6, "UTC->TT offset off by {leap_err_s} s");

    let jd = tt.jd();
    let (tdb_hi, tdb_lo) = tt_to_tdb(jd.day(), jd.fraction());
    let dt_s = ((tdb_hi - jd.day()) + (tdb_lo - jd.fraction())) * 86_400.0;
    // The approximation is the three leading terms of the TDB - TT series,
    // each bounded by its amplitude, so |TDB - TT| <= 1.6929 ms. The bound
    // 1.7 ms is used here.
    assert!(dt_s.abs() <= 1.7e-3, "|TDB-TT| = {dt_s} s exceeds bound");
    assert!(dt_s != 0.0, "TT -> TDB conversion did not change the epoch");
    assert_close(
        dt_s,
        tdb_minus_tt_seconds(jd.day(), jd.fraction()),
        1e-9,
        "TDB-TT consistency",
    );

    // Round trip TDB -> TT. The derivative of TDB-TT is at most about
    // 2.9e-5 s per day, so the fixed-point iteration converges to far below
    // 100 ns. The tolerance is 100 ns.
    let (back_hi, back_lo) = tdb_to_tt(tdb_hi, tdb_lo);
    let round_trip_s = ((back_hi - jd.day()) + (back_lo - jd.fraction())) * 86_400.0;
    assert!(
        round_trip_s.abs() < 1e-7,
        "TDB->TT round trip {round_trip_s} s"
    );

    // ---- Earth-Sun distance ---------------------------------------------
    // No SPK kernel is bundled, so `earth_sun_distance_km` is not used. The
    // analytic helper is checked against an independent Kepler solve with
    // the same mean anomaly, M = 357.53 + 0.98560028 d degrees, and the
    // Earth eccentricity e = 0.01671022. The reference r = 1 - e cos E is
    // exact in e, while the helper keeps terms through e^2. The difference is
    // about e^3 (~5e-6 AU) plus the error of the linear mean anomaly. A
    // tolerance of 1e-4 AU is used, which matches the helper's documented
    // accuracy.
    let r_au = earth_sun_distance_au_approx(jd.day(), jd.fraction());
    let d = (jd.day() - J2000_JD) + jd.fraction();
    let m = (357.53 + 0.985_600_28 * d).to_radians();
    let r_kepler = kepler_distance_au(m, 0.01671022);
    assert!(
        (r_au - r_kepler).abs() < 1e-4,
        "helper {r_au} AU vs Kepler {r_kepler} AU"
    );
    // Earth is between perihelion (0.983 AU) and aphelion (1.017 AU).
    assert!((0.983..=1.017).contains(&r_au), "distance {r_au} AU");
    let r_km = r_au * AU_KM;
    assert!((146.0e6..=153.0e6).contains(&r_km), "distance {r_km} km");

    // ---- (v) Export the 2-D result and read it back ----------------------
    // Time mean over the box gives a (lat, lon) field. Its value is
    // lat_i + mean(t), so each cell has a closed form.
    let reduced = box_expr
        .reduce_axes(&[0], Reduction::Mean, false)
        .expect("reduce time");
    assert_eq!(reduced.shape, RESULT_SHAPE.to_vec());
    for (i, row) in reduced.values.chunks(RESULT_SHAPE[1] as usize).enumerate() {
        for &v in row {
            assert_close(v, box_lats[i] + mean_t, TOL, "time-mean cell");
        }
    }
    assert_eq!(
        counting.take_reads(),
        expected_chunks,
        "chunks streamed by the time reduction"
    );

    let out_meta = ArrayMetadata::new(
        ZarrFormat::V2,
        RESULT_SHAPE.to_vec(),
        vec![4, 5],
        DType::FLOAT64,
    )
    .expect("result metadata");
    let result = Array::create(store.clone(), "sst_box_time_mean", out_meta).expect("create");
    result
        .write_f64(&[Selection::ALL, Selection::ALL], &reduced.values)
        .expect("write");

    let reopened = Array::open(store.clone(), "sst_box_time_mean").expect("reopen");
    assert_eq!(reopened.shape(), &RESULT_SHAPE);
    let read_back = reopened
        .read_f64(&[Selection::ALL, Selection::ALL])
        .expect("read back");
    assert_eq!(read_back.len(), reduced.values.len());
    for (a, b) in read_back.iter().zip(&reduced.values) {
        assert_close(*a, *b, TOL, "round-tripped cell");
    }

    // The weighted mean of the stored 2-D field must equal the scalar weighted
    // mean of the 3-D box, because the time mean preserves the cos(lat)
    // weighting.
    let stored_weighted = Expr::from_array(reopened)
        .weighted_mean(&weights)
        .expect("weighted stored")
        .expect("non-zero");
    assert_close(
        stored_weighted,
        expected_weighted,
        TOL,
        "stored-field weighted mean",
    );
}
