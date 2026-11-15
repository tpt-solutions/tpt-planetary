# ADR-003: Lazy evaluation for N-D arrays

**Status:** Accepted

## Decision

Mathematical operations on N-D arrays will be evaluated lazily.

## Rationale

Climate models produce terabytes of output, and eager evaluation would run out
of memory. A lazy execution graph lets the engine fuse operations (for example
`array * 2 + 1`) and stream only the chunks it needs from object storage.
