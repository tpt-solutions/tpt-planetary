//! Tabular bridge to tpt-strata: one-dimensional Zarr arrays as the columns of
//! an in-memory table, which can then be queried with tpt-strata's builder API
//! or SQL, and written as CSV.
//!
//! This replaces the Parquet interchange planned earlier. Parquet and Arrow are
//! not used: their dependencies are Apache-2.0 only, which the licence policy
//! excludes. tpt-strata is MIT OR Apache-2.0 with no external dependencies.
//!
//! Enabled by the `strata` feature.

use alloc::format;
use alloc::string::ToString;
use alloc::vec::Vec;

use tpt_strata::{Column, Scalar, Table};

use crate::array::Array;
use crate::error::{Error, Result};

/// Reads a one-dimensional float64 array into a column. NaN values become SQL
/// NULL, and other values become `F64` scalars.
pub fn column_from_array(name: &str, array: &Array) -> Result<Column> {
    let shape = &array.metadata().shape;
    if shape.len() != 1 {
        return Err(Error::InvalidSelection(format!(
            "column '{name}' needs a one-dimensional array, got {} dimensions",
            shape.len()
        )));
    }
    let values = array.read_f64(&[])?;
    let scalars: Vec<Scalar> = values
        .into_iter()
        .map(|v| {
            if v.is_nan() {
                Scalar::Null
            } else {
                Scalar::F64(v)
            }
        })
        .collect();
    Ok(Column::new(name, scalars))
}

/// Builds a table from named one-dimensional arrays. All columns must have the
/// same length.
pub fn table_from_arrays(columns: &[(&str, &Array)]) -> Result<Table> {
    let cols = columns
        .iter()
        .map(|(name, array)| column_from_array(name, array))
        .collect::<Result<Vec<_>>>()?;
    Table::try_new(cols).map_err(|e| Error::InvalidMetadata(e.to_string()))
}

/// Renders a table as CSV text (header line, then one line per row).
pub fn table_to_csv(table: &Table) -> alloc::string::String {
    tpt_strata::csv::to_csv(table)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dtype::DType;
    use crate::meta::{ArrayMetadata, ZarrFormat};
    use crate::store::MemoryStore;
    use alloc::sync::Arc;

    fn float_array(store: Arc<MemoryStore>, name: &str, values: &[f64]) -> Array {
        let meta = ArrayMetadata::new(
            ZarrFormat::V2,
            vec![values.len() as u64],
            vec![values.len() as u64],
            DType::FLOAT64,
        )
        .unwrap();
        let array = Array::create(store, name, meta).unwrap();
        array.write_f64(&[], values).unwrap();
        array
    }

    #[test]
    fn arrays_become_columns_and_nan_becomes_null() {
        let store = Arc::new(MemoryStore::new());
        let time = float_array(store.clone(), "time", &[0.0, 1.5, 3.0]);
        let flux = float_array(store.clone(), "flux", &[10.0, f64::NAN, 12.25]);
        let table = table_from_arrays(&[("time", &time), ("flux", &flux)]).unwrap();
        assert_eq!(table.row_count(), 3);
        assert_eq!(table.column_names(), vec!["time", "flux"]);
        let csv = table_to_csv(&table);
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 4, "csv: {csv}");
        assert!(lines[0].starts_with("time") && lines[0].contains("flux"));
        assert!(lines[2].contains("1.5"), "row 2: {}", lines[2]);
    }

    #[test]
    fn rejects_two_dimensional_arrays_and_length_mismatch() {
        let store = Arc::new(MemoryStore::new());
        let a = float_array(store.clone(), "a", &[1.0, 2.0]);
        let b = float_array(store.clone(), "b", &[1.0, 2.0, 3.0]);
        assert!(table_from_arrays(&[("a", &a), ("b", &b)]).is_err());
        let meta =
            ArrayMetadata::new(ZarrFormat::V2, vec![2, 2], vec![2, 2], DType::FLOAT64).unwrap();
        let grid = Array::create(store, "grid", meta).unwrap();
        assert!(column_from_array("grid", &grid).is_err());
    }
}
