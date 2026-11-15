# ADR-001: Zarr and cloud-native I/O over legacy HDF5 C libraries

**Status:** Accepted

## Decision

We will build a native chunked-array engine focused on the Zarr format and
cloud object storage, rather than wrapping the HDF5 C library.

## Rationale

The HDF5 C library is hard to cross-compile, relies on POSIX file locking, and
is designed for local monolithic files. Zarr is the modern, cloud-native
standard for planetary-scale data. Implementing Zarr natively in Rust lets us
stream chunks from object storage without C FFI.
