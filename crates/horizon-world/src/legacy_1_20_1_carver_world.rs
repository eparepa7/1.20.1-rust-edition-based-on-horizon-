//! A deliberately versioned bridge from 1.20.1 carver geometry to live chunks.
//!
//! This does not choose AIR/LAVA, perform aquifer sampling, restore surface
//! material, or schedule fluid updates. A *required* caller-supplied policy
//! must do those things according to the configured 1.20.1 carver. There is
//! no 26.3 fallback. The geometry emits *local* X/Z and absolute Y.
//!
//! Keep the mask and `seen` set across all carvers of one carving step for
//! the same target chunk. Create a fresh pair for another step/chunk. Debug
//! mode still calls the policy on repeated candidates, matching the original
//! geometry probe's repeat handling.

use crate::{
    compression::invalid,
    legacy_1_20_1_world::{Legacy1201Level, LEGACY_1_20_1_DATA_VERSION},
    registry::BlockRegistry,
};
use horizon_core::{
    carving::{CarverOutput, CarvingMask},
    position::{BlockPos, ChunkPos},
};
use std::{collections::HashSet, io, ptr};

/// Per-ellipsoid-column Java MutableBoolean, reset before descending Y.
#[derive(Default, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Legacy1201CarverColumn {
    found_surface: bool,
}

impl Legacy1201CarverColumn {
    pub fn observed(&mut self, was_grassy: bool) {
        self.found_surface |= was_grassy;
    }

    pub fn found_surface(&self) -> bool {
        self.found_surface
    }
}

/// Convert a target-chunk-local X/Z into block coordinates without wraparound.
fn world_cell(chunk: ChunkPos, x: i32, y: i32, z: i32) -> Option<BlockPos> {
    if !(0..16).contains(&x) || !(0..16).contains(&z) {
        return None;
    }
    Some(BlockPos::new(
        chunk.x.checked_mul(16)?.checked_add(x)?,
        y,
        chunk.z.checked_mul(16)?.checked_add(z)?,
    ))
}

/// Output for `Legacy1201Carver::carve_geometry`. The policy receives the
/// *current* `LevelAccess` at each visited block, so it can use its own
/// replaceable set, chosen aquifer and liquid/surface postprocessing.
/// The library never picks a replacement state on the caller's behalf.
pub struct Legacy1201CarverWorldOutput<'a, L, F> {
    level: &'a mut L,
    mask: &'a mut CarvingMask,
    seen: &'a mut HashSet<(i32, i32, i32)>,
    target: ChunkPos,
    debug: bool,
    apply: F,
    column: Legacy1201CarverColumn,
    visited: usize,
    applied: usize,
}

impl<'a, L, F> Legacy1201CarverWorldOutput<'a, L, F>
where
    L: Legacy1201Level,
    F: FnMut(&mut L, BlockPos, &mut Legacy1201CarverColumn) -> io::Result<bool>,
{
    pub fn new(
        level: &'a mut L,
        registry: &BlockRegistry,
        mask: &'a mut CarvingMask,
        seen: &'a mut HashSet<(i32, i32, i32)>,
        target: ChunkPos,
        debug: bool,
        apply: F,
    ) -> io::Result<Self> {
        if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
            || !ptr::eq(level.collision_block_registry(), registry)
            || registry.len() != 24_135
        {
            return Err(invalid("1.20.1 carver world and block registry do not match"));
        }
        // Geometry uses a lower-exclusive and upper-inclusive clamp. They
        // must lie inside the actual dimension, not 26.3's fixed bounds.
        if mask.min_y() >= mask.max_y()
            || mask.min_y() < level.min_y()
            || mask.max_y() >= level.max_y()
        {
            return Err(invalid("invalid 1.20.1 carver geometry limits"));
        }
        // Both the first and last block must fit in BlockPos. Otherwise a
        // wrapping chunk coordinate could write into a *different* chunk.
        let start = world_cell(target, 0, mask.min_y(), 0)
            .ok_or_else(|| invalid("1.20.1 carver target outside block coordinates"))?;
        world_cell(target, 15, mask.min_y(), 15)
            .ok_or_else(|| invalid("1.20.1 carver target outside block coordinates"))?;
        if !level.is_loaded(start) {
            return Err(invalid("1.20.1 carver target chunk is not loaded"));
        }
        Ok(Self {
            level,
            mask,
            seen,
            target,
            debug,
            apply,
            column: Legacy1201CarverColumn::default(),
            visited: 0,
            applied: 0,
        })
    }

    /// Number of geometry candidates received, including debug repeats.
    pub fn visited(&self) -> usize {
        self.visited
    }

    /// Number of positions for which the caller's 1.20.1 policy made a write.
    pub fn applied(&self) -> usize {
        self.applied
    }
}

impl<L, F> CarverOutput for Legacy1201CarverWorldOutput<'_, L, F>
where
    L: Legacy1201Level,
    F: FnMut(&mut L, BlockPos, &mut Legacy1201CarverColumn) -> io::Result<bool>,
{
    type Error = io::Error;

    fn min_y(&self) -> i32 {
        self.mask.min_y()
    }

    fn max_y(&self) -> i32 {
        self.mask.max_y()
    }

    fn begin_column(&mut self, _x: i32, _z: i32) -> io::Result<()> {
        self.column = Legacy1201CarverColumn::default();
        Ok(())
    }

    fn carve(&mut self, x: i32, y: i32, z: i32) -> io::Result<()> {
        let position = world_cell(self.target, x, y, z)
            .ok_or_else(|| invalid("1.20.1 carver candidate outside target chunk"))?;
        if y <= self.mask.min_y() || y > self.mask.max_y() {
            return Err(invalid("1.20.1 carver candidate outside height clamps"));
        }
        if !self.level.is_loaded(position) {
            return Err(invalid("1.20.1 carver candidate chunk unloaded"));
        }
        self.visited += 1;
        // Vanilla marks an attempted cell before deciding whether a block
        // can actually be replaced. Debug carving replays repeat visits.
        if !self.seen.insert((x, y, z)) && !self.debug {
            return Ok(());
        }
        self.mask.carve(x, y, z).map_err(io::Error::other)?;
        if (self.apply)(self.level, position, &mut self.column)? {
            self.applied += 1;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_chunk_local_to_absolute() {
        assert_eq!(
            world_cell(ChunkPos::new(-2, -1), 15, -45, 0),
            Some(BlockPos::new(-17, -45, -16)),
        );
        assert_eq!(
            world_cell(ChunkPos::new(0, 0), 0, 64, 15),
            Some(BlockPos::new(0, 64, 15)),
        );
    }

    #[test]
    fn local_coordinates_cannot_cross_chunk_border() {
        for x in [-1, 16, i32::MIN, i32::MAX] {
            assert_eq!(world_cell(ChunkPos::new(0, 0), x, 0, 0), None);
            assert_eq!(world_cell(ChunkPos::new(0, 0), 0, 0, x), None);
        }
    }

    #[test]
    fn out_of_block_range_target_rejected() {
        let max_chunk = i32::MAX / 16;
        assert_eq!(
            world_cell(ChunkPos::new(max_chunk, max_chunk), 15, 0, 15),
            Some(BlockPos::new(i32::MAX, 0, i32::MAX)),
        );
        assert_eq!(
            world_cell(ChunkPos::new(max_chunk + 1, 0), 0, 0, 0),
            None,
        );
        assert_eq!(
            world_cell(ChunkPos::new(i32::MIN, 0), 0, 0, 0),
            None,
        );
    }
}

#[cfg(test)]
mod column_tests {
    use super::Legacy1201CarverColumn;

    #[test]
    fn surface_flag_is_sticky_within_one_column() {
        let mut column = Legacy1201CarverColumn::default();
        column.observed(false);
        assert!(!column.found_surface());
        column.observed(true);
        column.observed(false);
        assert!(column.found_surface());
    }

    #[test]
    fn subsequent_column_does_not_inherit_surface_flag() {
        let mut column = Legacy1201CarverColumn::default();
        column.observed(true);
        column = Legacy1201CarverColumn::default();
        assert!(!column.found_surface());
    }
}
