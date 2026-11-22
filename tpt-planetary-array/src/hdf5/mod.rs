//! Read-only HDF5 subset: groups, datasets and attributes in files written by
//! the HDF5 library with its default ("earliest") format settings, as h5py and
//! netCDF-4 write them.
//!
//! Supported: superblock versions 0 and 2 (and version 3 by the same layout);
//! object headers version 1 and version 2, with continuation blocks; old-style
//! groups (symbol table messages with version 1 group B-trees and local heaps)
//! and new-style groups with hard-link messages; dataspaces versions 1 and 2;
//! datatypes of class integer, float and string; data layout version 3 with
//! compact, contiguous and chunked storage (chunk index: version 1 B-tree);
//! filter pipelines with deflate, shuffle and fletcher32; fill values; and
//! attributes of versions 1 to 3.
//!
//! Not supported, and reported as [`Error::Unsupported`]: dense (fractal-heap)
//! groups and attributes, the newer chunk indexes (fixed, extensible and
//! implicit arrays, version 4 layouts), soft and external links, virtual and
//! compound datasets, and filters other than deflate, shuffle and fletcher32.
//! Numeric values are converted to `f64`; 64-bit integers above 2^53 lose
//! precision in that conversion.

use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec;
use alloc::vec::Vec;

use crate::codec::Codec;
use crate::codec::compress::ZlibCodec;
use crate::codec::filters::ShuffleCodec;
use crate::error::{Error, Result};

const SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n'];

/// Message type numbers used by the reader.
const MSG_DATASPACE: u16 = 0x0001;
const MSG_LINK_INFO: u16 = 0x0002;
const MSG_DATATYPE: u16 = 0x0003;
const MSG_FILL_VALUE: u16 = 0x0005;
const MSG_LINK: u16 = 0x0006;
const MSG_DATA_LAYOUT: u16 = 0x0008;
const MSG_FILTER_PIPELINE: u16 = 0x000B;
const MSG_ATTRIBUTE: u16 = 0x000C;
const MSG_CONTINUATION: u16 = 0x0010;
const MSG_SYMBOL_TABLE: u16 = 0x0011;

/// Kind of an object in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObjectKind {
    /// A group (it holds links to other objects).
    Group,
    /// A dataset (it has a data layout message).
    Dataset,
    /// Any other object, such as a committed datatype.
    Other,
}

/// Element class and size of a datatype, the parts the reader uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Fixed-point integer.
    Integer {
        /// True for two's-complement signed integers.
        signed: bool,
    },
    /// IEEE floating point (4 or 8 bytes).
    Float,
    /// Fixed-length string (bytes, NUL-padded).
    String,
    /// Any other class (compound, enumeration, reference, variable length, ...).
    Other,
}

/// A datatype: class, size in bytes and byte order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DataType {
    /// Element class.
    pub kind: Kind,
    /// Element size in bytes.
    pub size: usize,
    /// True for big-endian storage.
    pub big_endian: bool,
}

/// A parsed HDF5 file over a byte slice (the whole file is in memory).
#[derive(Debug, Clone, Copy)]
pub struct Hdf5File<'a> {
    data: &'a [u8],
    offset_size: usize,
    length_size: usize,
    root: u64,
}

/// A dataset of a file, with its metadata and a way to read its values.
#[derive(Debug, Clone)]
pub struct Dataset<'f, 'a> {
    file: &'f Hdf5File<'a>,
    shape: Vec<u64>,
    dtype: DataType,
    layout: Layout,
    fill: Option<Vec<u8>>,
    filters: Vec<Filter>,
}

/// An attribute of a group or dataset.
#[derive(Debug, Clone, PartialEq)]
pub struct Attribute {
    /// Attribute name.
    pub name: String,
    /// Element datatype.
    pub dtype: DataType,
    /// Shape (empty for a scalar).
    pub shape: Vec<u64>,
    raw: Vec<u8>,
}

#[derive(Debug, Clone)]
enum Layout {
    Compact(Vec<u8>),
    Contiguous { address: Option<u64>, size: u64 },
    Chunked { index: u64, chunk: Vec<u64> },
}

#[derive(Debug, Clone, Copy)]
struct Filter {
    id: u16,
    client0: Option<u32>,
}

/// One chunk of a chunked dataset, from the chunk index.
struct ChunkRecord {
    origin: Vec<u64>,
    stored_size: u64,
    filter_mask: u32,
    address: u64,
}

/// Little-endian unsigned integer of `n` bytes (1 to 8) at `pos`.
fn read_uint(data: &[u8], pos: usize, n: usize) -> Result<u64> {
    if n == 0 || n > 8 {
        return Err(Error::Unsupported(format!("HDF5 integer of {n} bytes")));
    }
    let bytes = data.get(pos..pos + n).ok_or_else(|| {
        Error::InvalidMetadata("HDF5 structure runs past the end of the file".into())
    })?;
    Ok(bytes
        .iter()
        .rev()
        .fold(0u64, |acc, &b| (acc << 8) | u64::from(b)))
}

/// True if an address of `n` bytes is the undefined address (all ones).
fn is_undefined(value: u64, n: usize) -> bool {
    n == 8 && value == u64::MAX || n < 8 && value == (1u64 << (8 * n)) - 1
}

fn align8(n: usize) -> usize {
    (n + 7) & !7
}

impl<'a> Hdf5File<'a> {
    /// Opens an HDF5 file held in memory. The superblock is searched at the
    /// standard offsets (0, 512, 1024, ...).
    pub fn open(data: &'a [u8]) -> Result<Self> {
        let mut base = 0usize;
        loop {
            if data.get(base..base + 8) == Some(&SIGNATURE[..]) {
                break;
            }
            base = if base == 0 { 512 } else { base * 2 };
            if base >= data.len() {
                return Err(Error::InvalidMetadata(
                    "not an HDF5 file (no superblock)".into(),
                ));
            }
        }
        let version = *data
            .get(base + 8)
            .ok_or_else(|| Error::InvalidMetadata("truncated HDF5 superblock".into()))?;
        match version {
            0 | 1 => {
                let offset_size = usize::from(data[base + 13]);
                let length_size = usize::from(data[base + 14]);
                let fields = base + if version == 0 { 24 } else { 28 };
                // Base, free-space, end-of-file and driver addresses come first;
                // the root symbol table entry follows.
                let ste = fields + 4 * offset_size;
                let root = read_uint(data, ste + offset_size, offset_size)?;
                Ok(Self {
                    data,
                    offset_size,
                    length_size,
                    root,
                })
            }
            2 | 3 => {
                let offset_size = usize::from(data[base + 9]);
                let length_size = usize::from(data[base + 10]);
                let root = read_uint(data, base + 12 + 3 * offset_size, offset_size)?;
                Ok(Self {
                    data,
                    offset_size,
                    length_size,
                    root,
                })
            }
            other => Err(Error::Unsupported(format!(
                "HDF5 superblock version {other}"
            ))),
        }
    }

    fn slice(&self, pos: usize, n: usize) -> Result<&'a [u8]> {
        self.data.get(pos..pos + n).ok_or_else(|| {
            Error::InvalidMetadata("HDF5 structure runs past the end of the file".into())
        })
    }

    fn uint(&self, pos: usize, n: usize) -> Result<u64> {
        read_uint(self.data, pos, n)
    }

    /// Messages of the object header at `address`, as `(type, data)` pairs.
    fn messages(&self, address: u64) -> Result<Vec<(u16, Vec<u8>)>> {
        let start = address as usize;
        let mut out = Vec::new();
        // Pending message areas: (start, end, version-2 header, creation order).
        let mut pending: Vec<(usize, usize, bool, bool)> = Vec::new();
        if self.slice(start, 4)? == b"OHDR" {
            let flags = self.slice(start + 5, 1)?[0];
            let mut p = start + 6;
            if flags & 0x20 != 0 {
                p += 16;
            }
            if flags & 0x10 != 0 {
                p += 4;
            }
            let size_bytes = 1usize << (flags & 3);
            let chunk = self.uint(p, size_bytes)? as usize;
            p += size_bytes;
            // The chunk size excludes the trailing checksum of chunk 0.
            pending.push((p, p + chunk, true, flags & 0x04 != 0));
        } else {
            if self.slice(start, 1)?[0] != 1 {
                return Err(Error::Unsupported("HDF5 object header version".into()));
            }
            let size = self.uint(start + 8, 4)? as usize;
            pending.push((start + 16, start + 16 + size, false, false));
        }
        while let Some((from, to, v2, order)) = pending.pop() {
            let mut p = from;
            while p < to {
                let (ty, size, data_at) = if v2 {
                    let hdr = if order { 6 } else { 4 };
                    if p + hdr > to {
                        break;
                    }
                    let ty = u16::from(self.slice(p, 1)?[0]);
                    let size = self.uint(p + 1, 2)? as usize;
                    (ty, size, p + hdr)
                } else {
                    if p + 8 > to {
                        break;
                    }
                    let ty = self.uint(p, 2)? as u16;
                    let size = self.uint(p + 2, 2)? as usize;
                    (ty, size, p + 8)
                };
                let data = self.slice(data_at, size)?;
                if ty == MSG_CONTINUATION {
                    let at = read_uint(data, 0, self.offset_size)?;
                    let len = read_uint(data, self.offset_size, self.length_size)? as usize;
                    let at = at as usize;
                    if v2 {
                        // Continuation chunks begin with "OCHK" and end with a checksum.
                        pending.push((at + 4, at + len - 4, true, order));
                    } else {
                        pending.push((at, at + len, false, false));
                    }
                } else if ty != 0 {
                    out.push((ty, data.to_vec()));
                }
                p = if v2 {
                    data_at + size
                } else {
                    data_at + align8(size)
                };
            }
        }
        Ok(out)
    }

    /// The kind of the object at `address`.
    fn kind_at(&self, address: u64) -> Result<ObjectKind> {
        let msgs = self.messages(address)?;
        if msgs.iter().any(|(t, _)| *t == MSG_DATA_LAYOUT) {
            Ok(ObjectKind::Dataset)
        } else if msgs
            .iter()
            .any(|(t, _)| *t == MSG_SYMBOL_TABLE || *t == MSG_LINK || *t == MSG_LINK_INFO)
            || !msgs.iter().any(|(t, _)| *t == MSG_DATATYPE)
        {
            Ok(ObjectKind::Group)
        } else {
            Ok(ObjectKind::Other)
        }
    }

    /// Children of the group at `address`: `(name, object address)`.
    fn children(&self, address: u64) -> Result<Vec<(String, u64)>> {
        let mut out = Vec::new();
        for (ty, data) in self.messages(address)? {
            match ty {
                MSG_SYMBOL_TABLE => {
                    let btree = read_uint(&data, 0, self.offset_size)?;
                    let heap = read_uint(&data, self.offset_size, self.offset_size)?;
                    let heap_data = self.heap_data(heap)?;
                    self.group_btree(btree, heap_data, &mut out)?;
                }
                MSG_LINK => {
                    if let Some(link) = self.hard_link(&data)? {
                        out.push(link);
                    }
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Data address of a local heap at `heap`.
    fn heap_data(&self, heap: u64) -> Result<usize> {
        let h = heap as usize;
        if self.slice(h, 4)? != b"HEAP" {
            return Err(Error::InvalidMetadata(
                "local heap signature missing".into(),
            ));
        }
        Ok(read_uint(self.data, h + 8 + 2 * self.length_size, self.offset_size)? as usize)
    }

    fn heap_name(&self, heap_data: usize, offset: u64) -> Result<String> {
        let start = heap_data + offset as usize;
        let mut end = start;
        while *self
            .data
            .get(end)
            .ok_or_else(|| Error::InvalidMetadata("heap name runs past the file".into()))?
            != 0
        {
            end += 1;
        }
        Ok(String::from_utf8_lossy(&self.data[start..end]).to_string())
    }

    /// Collects the entries of a version-1 group B-tree (and its symbol nodes).
    fn group_btree(
        &self,
        address: u64,
        heap_data: usize,
        out: &mut Vec<(String, u64)>,
    ) -> Result<()> {
        let at = address as usize;
        if self.slice(at, 4)? != b"TREE" {
            return Err(Error::InvalidMetadata(
                "group B-tree signature missing".into(),
            ));
        }
        let node_type = self.slice(at + 4, 1)?[0];
        let level = self.slice(at + 5, 1)?[0];
        let used = self.uint(at + 6, 2)? as usize;
        if node_type != 0 {
            return Err(Error::InvalidMetadata(
                "expected a group B-tree node".into(),
            ));
        }
        let keys = at + 8 + 2 * self.offset_size;
        for i in 0..used {
            let child = self.uint(
                keys + i * (self.length_size + self.offset_size) + self.length_size,
                self.offset_size,
            )?;
            if level == 0 {
                self.symbol_node(child, heap_data, out)?;
            } else {
                self.group_btree(child, heap_data, out)?;
            }
        }
        Ok(())
    }

    /// Entries of one symbol table node ("SNOD").
    fn symbol_node(
        &self,
        address: u64,
        heap_data: usize,
        out: &mut Vec<(String, u64)>,
    ) -> Result<()> {
        let at = address as usize;
        if self.slice(at, 4)? != b"SNOD" {
            return Err(Error::InvalidMetadata(
                "symbol table node signature missing".into(),
            ));
        }
        let count = self.uint(at + 6, 2)? as usize;
        let entry = 2 * self.offset_size + 24;
        for i in 0..count {
            let p = at + 8 + i * entry;
            let name_offset = self.uint(p, self.offset_size)?;
            let object = self.uint(p + self.offset_size, self.offset_size)?;
            out.push((self.heap_name(heap_data, name_offset)?, object));
        }
        Ok(())
    }

    /// A hard link message: `(name, address)`. Other link types are skipped.
    fn hard_link(&self, data: &[u8]) -> Result<Option<(String, u64)>> {
        if data.len() < 2 || data[0] != 1 {
            return Err(Error::Unsupported("HDF5 link message version".into()));
        }
        let flags = data[1];
        let mut p = 2usize;
        let mut link_type = 0u8;
        if flags & 0x08 != 0 {
            link_type = data[p];
            p += 1;
        }
        if flags & 0x04 != 0 {
            p += 8;
        }
        if flags & 0x10 != 0 {
            p += 1;
        }
        let name_len_bytes = 1usize << (flags & 3);
        let name_len = read_uint(data, p, name_len_bytes)? as usize;
        p += name_len_bytes;
        let name = String::from_utf8_lossy(
            data.get(p..p + name_len)
                .ok_or_else(|| Error::InvalidMetadata("link name runs past the message".into()))?,
        )
        .to_string();
        p += name_len;
        if link_type != 0 {
            return Ok(None);
        }
        Ok(Some((name, read_uint(data, p, self.offset_size)?)))
    }

    /// Kind of the object at `path` (`"/"` or `""` is the root group).
    pub fn kind(&self, path: &str) -> Result<ObjectKind> {
        self.kind_at(self.resolve(path)?)
    }

    /// Names of the children of the group at `path`, in the order the file lists them.
    pub fn list(&self, path: &str) -> Result<Vec<String>> {
        let address = self.resolve(path)?;
        if self.kind_at(address)? != ObjectKind::Group {
            return Err(Error::InvalidSelection(format!("'{path}' is not a group")));
        }
        Ok(self
            .children(address)?
            .into_iter()
            .map(|(n, _)| n)
            .collect())
    }

    fn resolve(&self, path: &str) -> Result<u64> {
        let mut address = self.root;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            address = self
                .children(address)?
                .into_iter()
                .find(|(n, _)| n == part)
                .map(|(_, a)| a)
                .ok_or_else(|| Error::NotFound(format!("HDF5 object '{path}'")))?;
        }
        Ok(address)
    }

    /// The dataset at `path`.
    pub fn dataset(&self, path: &str) -> Result<Dataset<'_, 'a>> {
        let address = self.resolve(path)?;
        let msgs = self.messages(address)?;
        let find = |t: u16| {
            msgs.iter()
                .find(|(ty, _)| *ty == t)
                .map(|(_, d)| d.as_slice())
        };
        let layout = find(MSG_DATA_LAYOUT)
            .ok_or_else(|| Error::InvalidSelection(format!("'{path}' is not a dataset")))?;
        let shape = self.dataspace(find(MSG_DATASPACE).unwrap_or(&[]))?;
        let dtype = parse_datatype(
            find(MSG_DATATYPE)
                .ok_or_else(|| Error::InvalidMetadata("dataset without datatype".into()))?,
        )?;
        let layout = self.layout(layout)?;
        let filters = match find(MSG_FILTER_PIPELINE) {
            Some(d) => self.filters(d)?,
            None => Vec::new(),
        };
        let fill = match find(MSG_FILL_VALUE) {
            Some(d) => fill_value(d)?,
            None => None,
        };
        Ok(Dataset {
            file: self,
            shape,
            dtype,
            layout,
            fill,
            filters,
        })
    }

    /// Attributes of the group or dataset at `path`.
    pub fn attributes(&self, path: &str) -> Result<Vec<Attribute>> {
        let address = self.resolve(path)?;
        let mut out = Vec::new();
        for (ty, data) in self.messages(address)? {
            if ty == MSG_ATTRIBUTE {
                out.push(self.attribute(&data)?);
            }
        }
        Ok(out)
    }

    fn dataspace(&self, d: &[u8]) -> Result<Vec<u64>> {
        if d.is_empty() {
            return Ok(Vec::new());
        }
        let (rank, dims_at) = match d[0] {
            1 => (usize::from(d[1]), 8),
            2 => {
                if d[3] == 2 {
                    return Ok(vec![0]); // null dataspace: no elements
                }
                (usize::from(d[1]), 4)
            }
            v => return Err(Error::Unsupported(format!("HDF5 dataspace version {v}"))),
        };
        (0..rank)
            .map(|i| read_uint(d, dims_at + i * self.length_size, self.length_size))
            .collect()
    }

    fn layout(&self, d: &[u8]) -> Result<Layout> {
        if d.len() < 2 || d[0] != 3 {
            return Err(Error::Unsupported(format!(
                "HDF5 data layout version {}",
                d.first().copied().unwrap_or(0)
            )));
        }
        match d[1] {
            0 => {
                let size = read_uint(d, 2, 2)? as usize;
                Ok(Layout::Compact(
                    d.get(4..4 + size)
                        .ok_or_else(|| {
                            Error::InvalidMetadata("compact data runs past the message".into())
                        })?
                        .to_vec(),
                ))
            }
            1 => Ok(Layout::Contiguous {
                address: {
                    let a = read_uint(d, 2, self.offset_size)?;
                    if is_undefined(a, self.offset_size) {
                        None
                    } else {
                        Some(a)
                    }
                },
                size: read_uint(d, 2 + self.offset_size, self.length_size)?,
            }),
            2 => {
                let dims = usize::from(d[2]);
                let index = read_uint(d, 3, self.offset_size)?;
                let mut chunk = Vec::with_capacity(dims);
                for i in 0..dims {
                    chunk.push(read_uint(d, 3 + self.offset_size + 4 * i, 4)?);
                }
                // The last entry is the element size, not a dimension.
                chunk.pop();
                Ok(Layout::Chunked { index, chunk })
            }
            c => Err(Error::Unsupported(format!("HDF5 data layout class {c}"))),
        }
    }

    fn filters(&self, d: &[u8]) -> Result<Vec<Filter>> {
        let version = d[0];
        let count = usize::from(d[1]);
        let mut p = if version == 1 { 8 } else { 2 };
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let id = read_uint(d, p, 2)? as u16;
            p += 2;
            let mut name_len = 0usize;
            if version == 1 || id >= 256 {
                name_len = read_uint(d, p, 2)? as usize;
                p += 2;
            }
            p += 2; // flags
            let ncd = read_uint(d, p, 2)? as usize;
            p += 2;
            if version == 1 {
                p += align8(name_len);
            } else {
                p += name_len;
            }
            let mut client0 = None;
            for i in 0..ncd {
                let v = read_uint(d, p + 4 * i, 4)? as u32;
                if i == 0 {
                    client0 = Some(v);
                }
            }
            p += 4 * ncd;
            if version == 1 && ncd % 2 == 1 {
                p += 4;
            }
            out.push(Filter { id, client0 });
        }
        Ok(out)
    }

    fn attribute(&self, d: &[u8]) -> Result<Attribute> {
        let version = d[0];
        let name_size = read_uint(d, 2, 2)? as usize;
        let dt_size = read_uint(d, 4, 2)? as usize;
        let ds_size = read_uint(d, 6, 2)? as usize;
        let mut p = if version == 3 { 9 } else { 8 };
        let name_bytes = d
            .get(p..p + name_size)
            .ok_or_else(|| Error::InvalidMetadata("attribute name runs past the message".into()))?;
        let name = String::from_utf8_lossy(name_bytes.split(|&b| b == 0).next().unwrap_or(&[]))
            .to_string();
        p += if version == 1 {
            align8(name_size)
        } else {
            name_size
        };
        let dtype = parse_datatype(d.get(p..p + dt_size).ok_or_else(|| {
            Error::InvalidMetadata("attribute datatype runs past the message".into())
        })?)?;
        p += if version == 1 {
            align8(dt_size)
        } else {
            dt_size
        };
        let shape = self.dataspace(d.get(p..p + ds_size).ok_or_else(|| {
            Error::InvalidMetadata("attribute dataspace runs past the message".into())
        })?)?;
        p += if version == 1 {
            align8(ds_size)
        } else {
            ds_size
        };
        let count: u64 = shape.iter().product();
        let n = count as usize * dtype.size;
        let raw = d
            .get(p..p + n)
            .ok_or_else(|| Error::InvalidMetadata("attribute data runs past the message".into()))?
            .to_vec();
        Ok(Attribute {
            name,
            dtype,
            shape,
            raw,
        })
    }

    /// Reads a chunk index (version 1 B-tree of chunk records) below `address`.
    fn chunk_records(&self, address: u64, rank: usize, out: &mut Vec<ChunkRecord>) -> Result<()> {
        let at = address as usize;
        if self.slice(at, 4)? != b"TREE" {
            return Err(Error::InvalidMetadata(
                "chunk B-tree signature missing".into(),
            ));
        }
        if self.slice(at + 4, 1)?[0] != 1 {
            return Err(Error::InvalidMetadata(
                "expected a chunk B-tree node".into(),
            ));
        }
        let level = self.slice(at + 5, 1)?[0];
        let used = self.uint(at + 6, 2)? as usize;
        let key_size = 8 + 8 * (rank + 1);
        let keys = at + 8 + 2 * self.offset_size;
        for i in 0..used {
            let p = keys + i * (key_size + self.offset_size);
            let stored_size = self.uint(p, 4)?;
            let filter_mask = self.uint(p + 4, 4)? as u32;
            let mut origin = Vec::with_capacity(rank);
            for d in 0..rank {
                origin.push(self.uint(p + 8 + 8 * d, 8)?);
            }
            let child = self.uint(p + key_size, self.offset_size)?;
            if level == 0 {
                out.push(ChunkRecord {
                    origin,
                    stored_size,
                    filter_mask,
                    address: child,
                });
            } else {
                self.chunk_records(child, rank, out)?;
            }
        }
        Ok(())
    }
}

/// Parses a datatype message.
fn parse_datatype(d: &[u8]) -> Result<DataType> {
    if d.len() < 8 {
        return Err(Error::InvalidMetadata(
            "datatype message is too short".into(),
        ));
    }
    let class = d[0] & 0x0F;
    let bits = d[1];
    let size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
    let big_endian = bits & 0x01 != 0;
    let kind = match class {
        0 => Kind::Integer {
            signed: bits & 0x08 != 0,
        },
        1 => Kind::Float,
        3 => Kind::String,
        _ => Kind::Other,
    };
    Ok(DataType {
        kind,
        size,
        big_endian,
    })
}

/// The fill value of a fill-value message, if one is defined.
fn fill_value(d: &[u8]) -> Result<Option<Vec<u8>>> {
    match d.first().copied() {
        Some(1) | Some(2) => {
            if d.get(3).copied().unwrap_or(0) == 0 || d.len() < 8 {
                return Ok(None);
            }
            let size = u32::from_le_bytes([d[4], d[5], d[6], d[7]]) as usize;
            Ok(d.get(8..8 + size).map(<[u8]>::to_vec))
        }
        Some(3) => {
            let flags = d.get(1).copied().unwrap_or(0);
            if flags & 0x20 == 0 || d.len() < 6 {
                return Ok(None);
            }
            let size = u32::from_le_bytes([d[2], d[3], d[4], d[5]]) as usize;
            Ok(d.get(6..6 + size).map(<[u8]>::to_vec))
        }
        _ => Err(Error::Unsupported("HDF5 fill value message version".into())),
    }
}

impl DataType {
    /// Decodes raw element bytes of this type to `f64`.
    pub fn decode_f64(&self, raw: &[u8]) -> Result<Vec<f64>> {
        let n = self.size;
        if n == 0 || !raw.len().is_multiple_of(n) {
            return Err(Error::SizeMismatch {
                expected: n,
                actual: raw.len(),
            });
        }
        let mut out = Vec::with_capacity(raw.len() / n);
        for chunk in raw.chunks_exact(n) {
            let mut bytes = chunk.to_vec();
            if self.big_endian {
                bytes.reverse();
            }
            let value = match self.kind {
                Kind::Integer { signed } => {
                    let unsigned = read_uint(&bytes, 0, n)?;
                    if signed {
                        let shift = 64 - 8 * n as u32;
                        (((unsigned << shift) as i64) >> shift) as f64
                    } else {
                        unsigned as f64
                    }
                }
                Kind::Float => match n {
                    4 => f64::from(f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]])),
                    8 => f64::from_le_bytes([
                        bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5], bytes[6],
                        bytes[7],
                    ]),
                    _ => return Err(Error::Unsupported(format!("HDF5 float of {n} bytes"))),
                },
                _ => return Err(Error::Unsupported("HDF5 datatype is not numeric".into())),
            };
            out.push(value);
        }
        Ok(out)
    }
}

impl Attribute {
    /// Raw element bytes, in the file's byte order.
    pub fn raw(&self) -> &[u8] {
        &self.raw
    }

    /// Numeric values as `f64` (one for a scalar).
    pub fn values_f64(&self) -> Result<Vec<f64>> {
        self.dtype.decode_f64(&self.raw)
    }

    /// The value of a string attribute, without trailing NUL bytes.
    pub fn as_str(&self) -> Result<String> {
        if self.dtype.kind != Kind::String {
            return Err(Error::InvalidSelection(format!(
                "attribute '{}' is not a string",
                self.name
            )));
        }
        let end = self
            .raw
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.raw.len());
        Ok(String::from_utf8_lossy(&self.raw[..end]).to_string())
    }
}

impl<'f, 'a> Dataset<'f, 'a> {
    /// Shape of the dataset, in C order.
    pub fn shape(&self) -> &[u64] {
        &self.shape
    }

    /// Element datatype.
    pub fn dtype(&self) -> DataType {
        self.dtype
    }

    /// Reads all elements as `f64`, in C order.
    pub fn read_f64(&self) -> Result<Vec<f64>> {
        let raw = self.read_bytes()?;
        self.dtype.decode_f64(&raw)
    }

    /// Reads all elements as stored (file byte order), in C order.
    pub fn read_bytes(&self) -> Result<Vec<u8>> {
        let elem = self.dtype.size;
        let count: u64 = self.shape.iter().product();
        let total = count as usize * elem;
        let fill = match &self.fill {
            Some(v) if v.len() == elem => v.repeat(count as usize),
            _ => vec![0u8; total],
        };
        match &self.layout {
            Layout::Compact(bytes) => {
                if bytes.len() != total {
                    return Err(Error::SizeMismatch {
                        expected: total,
                        actual: bytes.len(),
                    });
                }
                Ok(bytes.clone())
            }
            Layout::Contiguous { address, size } => match address {
                None => Ok(fill),
                Some(a) => {
                    if *size as usize != total {
                        return Err(Error::SizeMismatch {
                            expected: total,
                            actual: *size as usize,
                        });
                    }
                    Ok(self.file.slice(*a as usize, total)?.to_vec())
                }
            },
            Layout::Chunked { index, chunk } => {
                if chunk.len() != self.shape.len() {
                    return Err(Error::InvalidMetadata(
                        "chunk rank differs from the dataset rank".into(),
                    ));
                }
                let mut out = fill;
                let mut records = Vec::new();
                self.file
                    .chunk_records(*index, self.shape.len(), &mut records)?;
                for rec in records {
                    let stored = self
                        .file
                        .slice(rec.address as usize, rec.stored_size as usize)?;
                    let raw = self.unfilter(stored.to_vec(), rec.filter_mask)?;
                    let expected: usize = chunk.iter().product::<u64>() as usize * elem;
                    if raw.len() != expected {
                        return Err(Error::SizeMismatch {
                            expected,
                            actual: raw.len(),
                        });
                    }
                    copy_chunk(&mut out, &self.shape, &raw, chunk, &rec.origin, elem);
                }
                Ok(out)
            }
        }
    }

    /// Undoes the filter pipeline on one stored chunk. Filters whose bit is set
    /// in `mask` were not applied when the chunk was written.
    fn unfilter(&self, mut data: Vec<u8>, mask: u32) -> Result<Vec<u8>> {
        for (i, filter) in self.filters.iter().enumerate().rev() {
            if mask & (1 << i) != 0 {
                continue;
            }
            data = match filter.id {
                1 => ZlibCodec::new(0)
                    .decode(&data)
                    .map_err(|e| Error::Codec(format!("HDF5 deflate: {e}")))?,
                2 => {
                    let size = filter.client0.map_or(self.dtype.size, |v| v as usize);
                    ShuffleCodec::new(size)?.decode(&data)?
                }
                3 => {
                    // Fletcher-32: the stored chunk ends with a 4-byte checksum.
                    if data.len() < 4 {
                        return Err(Error::Codec("HDF5 fletcher32 chunk too short".into()));
                    }
                    data.truncate(data.len() - 4);
                    data
                }
                id => return Err(Error::Unsupported(format!("HDF5 filter {id}"))),
            };
        }
        Ok(data)
    }
}

/// Copies one decoded chunk into the dataset buffer, clipping at the edges.
fn copy_chunk(
    out: &mut [u8],
    dims: &[u64],
    chunk: &[u8],
    chunk_dims: &[u64],
    origin: &[u64],
    elem: usize,
) {
    let rank = dims.len();
    let n: usize = chunk_dims.iter().product::<u64>() as usize;
    let mut idx = vec![0u64; rank];
    for linear in 0..n {
        // Unravel `linear` into the chunk's multi-index (C order).
        let mut rest = linear as u64;
        for d in (0..rank).rev() {
            idx[d] = rest % chunk_dims[d];
            rest /= chunk_dims[d];
        }
        let mut dst = 0u64;
        let mut inside = true;
        for d in 0..rank {
            let c = origin[d] + idx[d];
            if c >= dims[d] {
                inside = false;
                break;
            }
            dst = dst * dims[d] + c;
        }
        if inside {
            let s = linear * elem;
            let t = dst as usize * elem;
            out[t..t + elem].copy_from_slice(&chunk[s..s + elem]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datatype_decodes_signed_and_unsigned_integers() {
        let i16 = DataType {
            kind: Kind::Integer { signed: true },
            size: 2,
            big_endian: false,
        };
        assert_eq!(
            i16.decode_f64(&[0xFF, 0xFF, 0x00, 0x80]).unwrap(),
            vec![-1.0, -32768.0]
        );
        let u8 = DataType {
            kind: Kind::Integer { signed: false },
            size: 1,
            big_endian: false,
        };
        assert_eq!(u8.decode_f64(&[200]).unwrap(), vec![200.0]);
        let be = DataType {
            kind: Kind::Integer { signed: true },
            size: 2,
            big_endian: true,
        };
        assert_eq!(be.decode_f64(&[0x80, 0x00]).unwrap(), vec![-32768.0]);
    }

    #[test]
    fn datatype_decodes_floats() {
        let f = DataType {
            kind: Kind::Float,
            size: 8,
            big_endian: false,
        };
        assert_eq!(f.decode_f64(&2.5f64.to_le_bytes()).unwrap(), vec![2.5]);
        let f4 = DataType {
            kind: Kind::Float,
            size: 4,
            big_endian: false,
        };
        assert_eq!(f4.decode_f64(&(-0.5f32).to_le_bytes()).unwrap(), vec![-0.5]);
    }

    #[test]
    fn chunk_copy_clips_at_the_dataset_edges() {
        // A 3x3 dataset with 2x2 chunks: the chunk at (2,2) has one valid element.
        let mut out = vec![0u8; 9];
        let chunk = [1u8, 2, 3, 4];
        copy_chunk(&mut out, &[3, 3], &chunk, &[2, 2], &[2, 2], 1);
        assert_eq!(out, vec![0, 0, 0, 0, 0, 0, 0, 0, 1]);
        let mut out = vec![0u8; 9];
        copy_chunk(&mut out, &[3, 3], &chunk, &[2, 2], &[0, 0], 1);
        assert_eq!(out, vec![1, 2, 0, 3, 4, 0, 0, 0, 0]);
    }

    #[test]
    fn rejects_files_without_a_superblock() {
        assert!(matches!(
            Hdf5File::open(b"not hdf5"),
            Err(Error::InvalidMetadata(_))
        ));
    }
}
