//! Chunk-aligned region planning: which chunks a selection touches, and which
//! part of each chunk and of the output buffer each one contributes.

use alloc::vec::Vec;

use crate::selection::ResolvedDim;

/// The portion of one chunk that a selection contributes along one dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DimPiece {
    /// Chunk index along this dimension.
    pub chunk: u64,
    /// Offset within the chunk of the first selected element.
    pub local_start: u64,
    /// Distance between selected elements within the chunk.
    pub local_step: u64,
    /// Number of selected elements in this chunk along this dimension.
    pub count: u64,
    /// Position, along the selection (before dropping integer dimensions),
    /// of the first element in this piece.
    pub out_start: u64,
}

/// One chunk touched by a selection, together with the per-dimension pieces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkTask {
    /// Chunk coordinate in the chunk grid.
    pub coord: Vec<u64>,
    /// One piece per array dimension.
    pub pieces: Vec<DimPiece>,
}

/// The chunks touched by a resolved selection, in C order over chunk
/// coordinates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegionPlan {
    per_dim: Vec<Vec<DimPiece>>,
}

impl RegionPlan {
    /// Plans the chunks touched by `resolved` on a grid with `chunk_shape`.
    pub fn new(resolved: &[ResolvedDim], chunk_shape: &[u64]) -> Self {
        let per_dim = resolved
            .iter()
            .zip(chunk_shape)
            .map(|(r, &cs)| dim_pieces(r, cs))
            .collect();
        Self { per_dim }
    }

    /// Number of chunks the selection touches.
    pub fn num_chunks(&self) -> u64 {
        self.per_dim.iter().map(|p| p.len() as u64).product()
    }

    /// True if the selection contains no elements.
    pub fn is_empty(&self) -> bool {
        self.per_dim.iter().any(|p| p.is_empty())
    }

    /// Iterates over the touched chunks.
    pub fn tasks(&self) -> TaskIter<'_> {
        TaskIter {
            plan: self,
            idx: alloc::vec![0; self.per_dim.len()],
            done: self.is_empty(),
        }
    }
}

/// Iterator over [`ChunkTask`]s produced by [`RegionPlan::tasks`].
#[derive(Debug)]
pub struct TaskIter<'a> {
    plan: &'a RegionPlan,
    idx: Vec<usize>,
    done: bool,
}

impl Iterator for TaskIter<'_> {
    type Item = ChunkTask;

    fn next(&mut self) -> Option<ChunkTask> {
        if self.done {
            return None;
        }
        let pieces: Vec<DimPiece> = self
            .idx
            .iter()
            .zip(&self.plan.per_dim)
            .map(|(&i, p)| p[i])
            .collect();
        let coord = pieces.iter().map(|p| p.chunk).collect();
        let task = ChunkTask { coord, pieces };
        // Advance the odometer, last dimension fastest.
        let mut d = self.idx.len();
        loop {
            if d == 0 {
                self.done = true;
                break;
            }
            d -= 1;
            self.idx[d] += 1;
            if self.idx[d] < self.plan.per_dim[d].len() {
                break;
            }
            self.idx[d] = 0;
        }
        Some(task)
    }
}

fn dim_pieces(r: &ResolvedDim, chunk: u64) -> Vec<DimPiece> {
    let mut out = Vec::new();
    if r.count == 0 {
        return out;
    }
    let last_index = r.start + (r.count - 1) * r.step;
    let first_chunk = r.start / chunk;
    let last_chunk = last_index / chunk;
    for c in first_chunk..=last_chunk {
        let lo = c * chunk;
        let hi = lo + chunk;
        // First selected k whose index is >= lo, and first k whose index is >= hi.
        let k_lo = if r.start >= lo {
            0
        } else {
            (lo - r.start).div_ceil(r.step)
        };
        let k_hi = if r.start >= hi {
            0
        } else {
            (hi - r.start).div_ceil(r.step)
        };
        let k_hi = k_hi.min(r.count);
        if k_hi <= k_lo {
            continue;
        }
        let first_index = r.start + k_lo * r.step;
        out.push(DimPiece {
            chunk: c,
            local_start: first_index - lo,
            local_step: r.step,
            count: k_hi - k_lo,
            out_start: k_lo,
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dim(start: u64, step: u64, count: u64) -> ResolvedDim {
        ResolvedDim {
            start,
            step,
            count,
            kept: true,
        }
    }

    #[test]
    fn contiguous_range_spans_chunks() {
        // indices 3..9 with chunk size 4 -> chunks 0 (3), 1 (4), 2 (1)
        let plan = RegionPlan::new(&[dim(3, 1, 6)], &[4]);
        let pieces = &plan.per_dim[0];
        assert_eq!(pieces.len(), 3);
        assert_eq!(
            pieces[0],
            DimPiece {
                chunk: 0,
                local_start: 3,
                local_step: 1,
                count: 1,
                out_start: 0
            }
        );
        assert_eq!(
            pieces[1],
            DimPiece {
                chunk: 1,
                local_start: 0,
                local_step: 1,
                count: 4,
                out_start: 1
            }
        );
        assert_eq!(
            pieces[2],
            DimPiece {
                chunk: 2,
                local_start: 0,
                local_step: 1,
                count: 1,
                out_start: 5
            }
        );
    }

    #[test]
    fn strided_selection_counts_per_chunk() {
        // indices 1,4,7,10 with chunk size 5 -> chunk 0: {1,4}; chunk 1: {7}; chunk 2: {10}
        let plan = RegionPlan::new(&[dim(1, 3, 4)], &[5]);
        let c: Vec<(u64, u64, u64)> = plan.per_dim[0]
            .iter()
            .map(|p| (p.chunk, p.local_start, p.count))
            .collect();
        assert_eq!(c, vec![(0, 1, 2), (1, 2, 1), (2, 0, 1)]);
        assert_eq!(plan.per_dim[0][1].out_start, 2);
    }

    #[test]
    fn two_d_tasks_cover_grid_in_c_order() {
        let plan = RegionPlan::new(&[dim(0, 1, 5), dim(0, 1, 2)], &[4, 4]);
        assert_eq!(plan.num_chunks(), 2);
        let coords: Vec<Vec<u64>> = plan.tasks().map(|t| t.coord).collect();
        assert_eq!(coords, vec![vec![0, 0], vec![1, 0]]);
    }

    #[test]
    fn empty_selection_has_no_tasks() {
        let plan = RegionPlan::new(&[dim(0, 1, 0)], &[4]);
        assert!(plan.is_empty());
        assert_eq!(plan.tasks().count(), 0);
    }
}
