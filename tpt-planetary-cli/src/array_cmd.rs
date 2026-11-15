//! `tptplan array inspect`: shape, dtype, chunking and attributes of a local
//! Zarr v2 or v3 array directory.
//!
//! Remote locations (`s3://`, `https://`, any other `scheme://`) are not
//! supported in this build and return exit 3. `array convert` and `array math`
//! are not implemented and are reported from `main.rs`.

use std::path::Path;
use std::sync::Arc;

use tpt_planetary_array::meta::{ArrayMetadata, NodeMetadata, ZarrFormat};
use tpt_planetary_array::{Array, FsStore, Store, hierarchy};

use crate::json;
use crate::{EXIT_NOT_IMPLEMENTED, EXIT_RUNTIME, EXIT_USAGE, Failure, OutputFormat};

/// Prints a progress line to stderr when verbose output is enabled.
fn stage(verbose: u8, message: &str) {
    if verbose > 0 {
        eprintln!("tptplan: {message}");
    }
}

/// Returns the scheme of a `scheme://` location, if it has one.
fn uri_scheme(uri: &str) -> Option<&str> {
    // RFC 3986: a letter, then letters, digits, '+', '-' or '.'.
    let (scheme, _) = uri.split_once("://")?;
    let mut chars = scheme.chars();
    let first_ok = chars.next().is_some_and(|c| c.is_ascii_alphabetic());
    let rest_ok = chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    (first_ok && rest_ok).then_some(scheme)
}

/// Strips the `\\?\` prefix that `canonicalize` adds on Windows, for display.
fn display_path(path: &Path) -> String {
    let s = path.display().to_string();
    match s.strip_prefix(r"\\?\") {
        Some(rest) => rest.to_string(),
        None => s,
    }
}

/// Inspects the array at `uri` and returns the text to print (without a final
/// newline).
pub fn inspect(uri: &str, format: OutputFormat, verbose: u8) -> Result<String, Failure> {
    stage(verbose, "checking location");
    if let Some(scheme) = uri_scheme(uri) {
        return Err(Failure::new(
            EXIT_NOT_IMPLEMENTED,
            format!(
                "'{scheme}://' locations are not supported in this build; \
                 only local Zarr array directories can be inspected"
            ),
        ));
    }

    stage(verbose, "resolving local path");
    let path = Path::new(uri);
    let info = std::fs::metadata(path)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("cannot access '{uri}': {e}")))?;
    if !info.is_dir() {
        return Err(Failure::new(
            EXIT_RUNTIME,
            format!("'{uri}' is not a directory; a Zarr array is a directory"),
        ));
    }
    let abs = std::fs::canonicalize(path)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("cannot resolve '{uri}': {e}")))?;
    let (Some(parent), Some(name)) = (abs.parent(), abs.file_name().and_then(|n| n.to_str()))
    else {
        return Err(Failure::new(
            EXIT_USAGE,
            format!("cannot determine an array name from '{uri}'"),
        ));
    };

    stage(
        verbose,
        &format!("opening store at {}", display_path(parent)),
    );
    let store = FsStore::open(parent)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("cannot open store: {e}")))?;
    let store: Arc<dyn Store> = Arc::new(store);

    stage(
        verbose,
        "reading node metadata (zarr.json, .zarray or .zgroup)",
    );
    match hierarchy::read_node(store.as_ref(), name)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("cannot read metadata: {e}")))?
    {
        Some(NodeMetadata::Array(_)) => {}
        Some(NodeMetadata::Group(_)) => {
            return Err(Failure::new(
                EXIT_RUNTIME,
                format!("'{uri}' is a Zarr group, not an array"),
            ));
        }
        None => {
            return Err(Failure::new(
                EXIT_RUNTIME,
                format!("'{uri}' has no Zarr metadata (zarr.json, .zarray or .zgroup)"),
            ));
        }
    }

    stage(verbose, "validating dtype, chunk grid and codecs");
    let array = Array::open(store, name)
        .map_err(|e| Failure::new(EXIT_RUNTIME, format!("cannot open array: {e}")))?;

    stage(verbose, "formatting output");
    let display = display_path(&abs);
    let grid_shape = array.grid().grid_shape();
    Ok(match format {
        OutputFormat::Table => render_table(&display, array.metadata(), &grid_shape),
        OutputFormat::Json => render_json(&display, array.metadata(), &grid_shape),
    })
}

fn format_name(format: ZarrFormat) -> &'static str {
    match format {
        ZarrFormat::V2 => "v2",
        ZarrFormat::V3 => "v3",
    }
}

fn render_table(path: &str, meta: &ArrayMetadata, grid_shape: &[u64]) -> String {
    let mut lines = vec![
        format!("path: {path}"),
        format!("format: zarr {}", format_name(meta.format)),
        format!("shape: {}", json::u64_array(&meta.shape)),
        format!("chunk shape: {}", json::u64_array(&meta.chunk_shape)),
        format!("chunk grid: {}", json::u64_array(grid_shape)),
        format!("dtype: {}", meta.dtype.to_v2()),
        format!("fill value: {}", meta.fill_value.to_json()),
    ];
    let dims = match &meta.dimension_names {
        None => "none".to_string(),
        Some(names) => {
            let items: Vec<&str> = names.iter().map(|n| n.as_deref().unwrap_or("_")).collect();
            format!("[{}]", items.join(", "))
        }
    };
    lines.push(format!("dimension names: {dims}"));
    if meta.codecs.is_empty() {
        lines.push("codecs: none".to_string());
    } else {
        let codecs: Vec<String> = meta
            .codecs
            .iter()
            .map(|c| format!("{} {}", c.name, c.config))
            .collect();
        lines.push(format!("codecs: {}", codecs.join("; ")));
    }
    if meta.attributes.is_empty() {
        lines.push("attributes: none".to_string());
    } else {
        lines.push("attributes:".to_string());
        for (key, value) in &meta.attributes {
            lines.push(format!("  {key}: {value}"));
        }
    }
    lines.join("\n")
}

fn render_json(path: &str, meta: &ArrayMetadata, grid_shape: &[u64]) -> String {
    let dims = match &meta.dimension_names {
        None => "null".to_string(),
        Some(names) => {
            let items: Vec<String> = names
                .iter()
                .map(|n| json::opt_string(n.as_deref()))
                .collect();
            format!("[{}]", items.join(", "))
        }
    };
    let codecs: Vec<String> = meta
        .codecs
        .iter()
        .map(|c| {
            json::object(&[
                ("name", json::string(&c.name)),
                ("configuration", c.config.to_string()),
            ])
        })
        .collect();
    let attributes: Vec<(&str, String)> = meta
        .attributes
        .iter()
        .map(|(k, v)| (k.as_str(), v.to_string()))
        .collect();
    json::object(&[
        ("path", json::string(path)),
        (
            "format",
            json::string(&format!("zarr {}", format_name(meta.format))),
        ),
        ("shape", json::u64_array(&meta.shape)),
        ("chunk_shape", json::u64_array(&meta.chunk_shape)),
        ("chunk_grid", json::u64_array(grid_shape)),
        ("dtype", json::string(&meta.dtype.to_v2())),
        ("fill_value", meta.fill_value.to_json().to_string()),
        (
            "chunk_key_separator",
            json::string(&meta.chunk_key.separator().to_string()),
        ),
        ("dimension_names", dims),
        ("codecs", format!("[{}]", codecs.join(", "))),
        ("attributes", json::object(&attributes)),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_remote_schemes() {
        assert_eq!(uri_scheme("s3://bucket/a.zarr"), Some("s3"));
        assert_eq!(uri_scheme("https://example.org/a.zarr"), Some("https"));
        assert_eq!(uri_scheme(r"C:\data\a.zarr"), None);
        assert_eq!(uri_scheme("./a.zarr"), None);
    }

    #[test]
    fn remote_locations_are_not_implemented() {
        let e = inspect("https://example.org/a.zarr", OutputFormat::Table, 0).unwrap_err();
        assert_eq!(e.code, EXIT_NOT_IMPLEMENTED);
        assert!(e.message.contains("not supported in this build"));
    }

    #[test]
    fn missing_path_is_runtime_error() {
        let e = inspect("definitely-missing-dir.zarr", OutputFormat::Table, 0).unwrap_err();
        assert_eq!(e.code, EXIT_RUNTIME);
    }
}
