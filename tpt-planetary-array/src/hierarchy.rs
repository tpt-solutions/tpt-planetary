//! Node-level access to a store: reading and writing array and group
//! metadata, listing a hierarchy, and consolidating Zarr v2 metadata.

use alloc::collections::BTreeSet;
use alloc::string::{String, ToString};
use alloc::vec::Vec;

use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::meta::v2::{self, ZARRAY, ZATTRS, ZGROUP, ZMETADATA};
use crate::meta::v3::{self, ZARR_JSON};
use crate::meta::{ArrayMetadata, GroupMetadata, NodeMetadata, ZarrFormat};
use crate::store::{Store, join};

/// A node discovered while listing a hierarchy.
#[derive(Debug, Clone, PartialEq)]
pub struct NodeEntry {
    /// Path of the node relative to the store root (empty for the root).
    pub path: String,
    /// Zarr format of the node.
    pub format: ZarrFormat,
    /// True for arrays, false for groups.
    pub is_array: bool,
}

/// Reads the metadata of the node at `path`, or `None` if there is no node.
///
/// A v3 `zarr.json` takes precedence; otherwise `.zarray` or `.zgroup` is
/// read, together with the optional `.zattrs`.
pub fn read_node(store: &dyn Store, path: &str) -> Result<Option<NodeMetadata>> {
    if let Some(bytes) = store.get(&join(path, ZARR_JSON))? {
        return v3::parse_node(&bytes).map(Some);
    }
    let zattrs = store.get(&join(path, ZATTRS))?;
    if let Some(bytes) = store.get(&join(path, ZARRAY))? {
        return v2::parse_array(&bytes, zattrs.as_deref()).map(|m| Some(NodeMetadata::Array(m)));
    }
    if let Some(bytes) = store.get(&join(path, ZGROUP))? {
        return v2::parse_group(&bytes, zattrs.as_deref()).map(|g| Some(NodeMetadata::Group(g)));
    }
    Ok(None)
}

/// Writes array metadata for `path`, replacing any existing array metadata.
pub fn write_array(store: &dyn Store, path: &str, meta: &ArrayMetadata) -> Result<()> {
    match meta.format {
        ZarrFormat::V3 => store.set(&join(path, ZARR_JSON), v3::array_json(meta)?.as_bytes()),
        ZarrFormat::V2 => {
            store.set(&join(path, ZARRAY), v2::array_json(meta)?.as_bytes())?;
            if meta.attributes.is_empty() && meta.dimension_names.is_none() {
                store.delete(&join(path, ZATTRS))
            } else {
                let attrs = v2::attrs_json(&meta.attributes, meta.dimension_names.as_deref())?;
                store.set(&join(path, ZATTRS), attrs.as_bytes())
            }
        }
    }
}

/// Writes group metadata for `path` in the requested format.
pub fn write_group(
    store: &dyn Store,
    path: &str,
    format: ZarrFormat,
    group: &GroupMetadata,
) -> Result<()> {
    match format {
        ZarrFormat::V3 => {
            let doc = v3::group_json(&GroupMetadata {
                format: Some(ZarrFormat::V3),
                attributes: group.attributes.clone(),
            })?;
            store.set(&join(path, ZARR_JSON), doc.as_bytes())
        }
        ZarrFormat::V2 => {
            store.set(&join(path, ZGROUP), v2::group_json().as_bytes())?;
            if group.attributes.is_empty() {
                store.delete(&join(path, ZATTRS))
            } else {
                let attrs = v2::attrs_json(&group.attributes, None)?;
                store.set(&join(path, ZATTRS), attrs.as_bytes())
            }
        }
    }
}

/// Lists every array and group below `root` (use `""` for the whole store),
/// sorted by path. The root itself is included when it is a node.
pub fn list_nodes(store: &dyn Store, root: &str) -> Result<Vec<NodeEntry>> {
    let root = root.trim_matches('/');
    let prefix = if root.is_empty() {
        String::new()
    } else {
        alloc::format!("{root}/")
    };
    let mut paths = BTreeSet::new();
    for key in store.list(&prefix)? {
        let Some((parent, name)) = split_last(&key) else {
            continue;
        };
        if matches!(name, ZARR_JSON | ZARRAY | ZGROUP) {
            paths.insert(parent.to_string());
        }
    }
    let mut out = Vec::new();
    for path in paths {
        match read_node(store, &path)? {
            Some(NodeMetadata::Array(m)) => {
                out.push(NodeEntry {
                    path,
                    format: m.format,
                    is_array: true,
                });
            }
            Some(NodeMetadata::Group(g)) => {
                let format = g.format.unwrap_or(ZarrFormat::V2);
                out.push(NodeEntry {
                    path,
                    format,
                    is_array: false,
                });
            }
            None => {}
        }
    }
    Ok(out)
}

/// Splits `a/b/c` into (`a/b`, `c`); a bare name gives (`""`, `name`).
fn split_last(key: &str) -> Option<(&str, &str)> {
    match key.rfind('/') {
        Some(i) => Some((&key[..i], &key[i + 1..])),
        None => Some(("", key)),
    }
}

/// Writes a consolidated `.zmetadata` document at `root` that contains every
/// v2 `.zarray`, `.zgroup` and `.zattrs` document below it. Consolidation is a
/// snapshot: later metadata changes require calling this again.
pub fn consolidate_v2(store: &dyn Store, root: &str) -> Result<()> {
    let root = root.trim_matches('/');
    let prefix = if root.is_empty() {
        String::new()
    } else {
        alloc::format!("{root}/")
    };
    let mut entries: Vec<(String, Value)> = Vec::new();
    for key in store.list(&prefix)? {
        let rel = &key[prefix.len()..];
        let name = rel.rsplit('/').next().unwrap_or(rel);
        if !matches!(name, ZARRAY | ZGROUP | ZATTRS) {
            continue;
        }
        let bytes = store
            .get(&key)?
            .ok_or_else(|| Error::NotFound(key.clone()))?;
        let value: Value = serde_json::from_slice(&bytes)
            .map_err(|e| Error::InvalidMetadata(alloc::format!("{key}: {e}")))?;
        entries.push((rel.to_string(), value));
    }
    let doc = v2::consolidated_json(entries)?;
    store.set(&join(root, ZMETADATA), doc.as_bytes())
}

/// Reads the `.zmetadata` document at `root` into a map from relative key to
/// document. Returns `None` if the store has no consolidated metadata.
pub fn read_consolidated(store: &dyn Store, root: &str) -> Result<Option<Map<String, Value>>> {
    let Some(bytes) = store.get(&join(root, ZMETADATA))? else {
        return Ok(None);
    };
    Ok(Some(v2::parse_consolidated(&bytes)?.into_iter().collect()))
}

#[cfg(all(test, feature = "std"))]
mod tests {
    use super::*;
    use crate::dtype::DType;
    use crate::store::MemoryStore;

    #[test]
    fn lists_mixed_v2_and_v3_hierarchy() {
        let s = MemoryStore::new();
        write_group(&s, "", ZarrFormat::V2, &GroupMetadata::default()).unwrap();
        let arr = ArrayMetadata::new(ZarrFormat::V2, vec![4], vec![2], DType::INT32).unwrap();
        write_array(&s, "data/x", &arr).unwrap();
        let v3arr = ArrayMetadata::new(ZarrFormat::V3, vec![4], vec![2], DType::INT32).unwrap();
        write_array(&s, "v3/y", &v3arr).unwrap();
        write_group(&s, "v3", ZarrFormat::V3, &GroupMetadata::default()).unwrap();

        let nodes = list_nodes(&s, "").unwrap();
        let paths: Vec<(&str, bool)> = nodes
            .iter()
            .map(|n| (n.path.as_str(), n.is_array))
            .collect();
        assert_eq!(
            paths,
            vec![("", false), ("data/x", true), ("v3", false), ("v3/y", true)]
        );
    }

    #[test]
    fn consolidation_round_trip() {
        let s = MemoryStore::new();
        write_group(&s, "", ZarrFormat::V2, &GroupMetadata::default()).unwrap();
        let arr = ArrayMetadata::new(ZarrFormat::V2, vec![4], vec![2], DType::INT32).unwrap();
        write_array(&s, "a", &arr).unwrap();
        consolidate_v2(&s, "").unwrap();
        let map = read_consolidated(&s, "").unwrap().unwrap();
        assert!(map.contains_key("a/.zarray"));
        assert!(map.contains_key(".zgroup"));
    }
}
