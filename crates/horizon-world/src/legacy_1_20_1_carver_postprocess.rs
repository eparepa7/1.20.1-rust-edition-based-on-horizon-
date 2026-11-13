//! Chunk-carving effects for the independent 1.20.1 pipeline.
//!
//! The caller supplies 1.20.1 `CarvingContext.topMaterial` and the owning
//! proto-chunk's postprocessing marker. This module does not construct or
//! use 26.3 surface rules, schedule live fluid ticks, or load game assets.
//! `after_write` executes in Java's write -> fluid marker -> dirt restoration
//! order. Writes and the queue are not rolled back when a caller callback
//! fails; the failure propagates instead of claiming chunk parity.

use crate::{
    compression::invalid,
    legacy_1_20_1_carver_policy::{
        Legacy1201AquiferSample, Legacy1201CarveWrite, Legacy1201CarverPolicy,
    },
    legacy_1_20_1_carver_world::{Legacy1201CarverColumn, Legacy1201CarverWorldOutput},
    legacy_1_20_1_world::{Legacy1201Level, LEGACY_1_20_1_DATA_VERSION},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    carving::CarvingMask,
    position::{BlockPos, ChunkPos},
};
use std::{
    collections::{HashSet, VecDeque},
    io, ptr,
};

fn has_fluid(registry: &BlockRegistry, state: BlockStateId) -> io::Result<bool> {
    let block = registry
        .state(state)
        .ok_or_else(|| invalid("1.20.1 fluid state outside registry"))?;
    Ok(matches!(block.name.as_str(), "minecraft:water" | "minecraft:lava")
        || block
            .properties
            .get("waterlogged")
            .is_some_and(|v| v == "true"))
}

/// An ordered, duplicate-free list of positions to mark for proto-chunk
/// fluid postprocessing (NOT live-world fluid ticks). Persist this for the
/// entire CARVERS step of one target proto-chunk; never mix chunks or versions.
#[derive(Debug)]
pub struct Legacy1201CarverPostprocess {
    target: ChunkPos,
    pending: VecDeque<BlockPos>,
    pending_set: HashSet<BlockPos>,
}

impl Legacy1201CarverPostprocess {
    pub fn new(target: ChunkPos) -> Self {
        Self {
            target,
            pending: VecDeque::new(),
            pending_set: HashSet::new(),
        }
    }

    pub fn target(&self) -> ChunkPos {
        self.target
    }

    pub fn pending(&self) -> usize {
        self.pending.len()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_empty()
    }

    pub fn mark(&mut self, pos: BlockPos) -> io::Result<()> {
        if pos.chunk() != self.target {
            return Err(invalid("1.20.1 postprocess position outside owning proto-chunk"));
        }
        if self.pending_set.insert(pos) {
            self.pending.push_back(pos);
        }
        Ok(())
    }

    /// Mark successful proto-chunk positions. If marking fails, retain the
    /// failing position and all later entries for retry, in the same order.
    /// Positions marked before the error were already committed to the owner.
    pub fn flush(
        &mut self,
        mut mark_owner: impl FnMut(BlockPos) -> io::Result<()>,
    ) -> io::Result<usize> {
        let mut committed = 0;
        while let Some(&pos) = self.pending.front() {
            mark_owner(pos)?;
            self.pending.pop_front();
            self.pending_set.remove(&pos);
            committed += 1;
        }
        Ok(committed)
    }

    /// The 1.20.1 `WorldCarver.carveBlock` side effects after `setBlockState`:
    /// first mark the carved fluid (if requested by aquifer), then if any
    /// grass/mycelium was encountered in this ellipsoid column, replace a dirt
    /// block directly below with 1.20.1 `CarvingContext.topMaterial`.
    ///
    /// `top_material` must query the *legacy* biome/surface rules and may
    /// return None, in which case the dirt remains untouched. No 26.3 fallback
    /// is possible. The owning proto-chunk must update its heightmaps after
    /// raw storage writes; this function does not silently use 26.3 heightmaps.
    pub fn after_write<L, T>(
        &mut self,
        level: &mut L,
        registry: &BlockRegistry,
        write: Legacy1201CarveWrite,
        top_material: &mut T,
    ) -> io::Result<()>
    where
        L: Legacy1201Level,
        T: FnMut(&L, BlockPos) -> io::Result<Option<BlockStateId>>,
    {
        if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
            || !ptr::eq(level.collision_block_registry(), registry)
            || registry.len() != 24_135
        {
            return Err(invalid("1.20.1 carver postprocess registry/version mismatch"));
        }
        if write.position.chunk() != self.target
            || !level.is_loaded(write.position)
            || level.block_state(write.position) != write.replacement
        {
            return Err(invalid("1.20.1 postprocess requires completed carving write"));
        }
        if write.fluid_postprocessing {
            if !has_fluid(registry, write.replacement)? {
                return Err(invalid("1.20.1 carver requested postprocessing for non-fluid state"));
            }
            self.mark(write.position)?;
        }
        if !write.found_surface || write.position.y <= level.min_y() {
            return Ok(());
        }
        let below = BlockPos::new(write.position.x, write.position.y - 1, write.position.z);
        if !level.is_loaded(below) {
            return Err(invalid("1.20.1 carver surface restoration below unloaded chunk"));
        }
        let existing = level.block_state(below);
        let existing_name = &registry
            .state(existing)
            .ok_or_else(|| invalid("1.20.1 surface block outside registry"))?
            .name;
        if existing_name != "minecraft:dirt" {
            return Ok(());
        }
        let Some(top) = top_material(level, below)? else {
            return Ok(());
        };
        let top_has_fluid = has_fluid(registry, top)?;
        let before = level.set_raw(below, top)
            .ok_or_else(|| invalid("1.20.1 surface restoration write rejected"))?;
        if before != existing {
            return Err(invalid("1.20.1 surface restoration block changed before write"));
        }
        if top_has_fluid {
            self.mark(below)?;
        }
        Ok(())
    }
}

/// Wire one 1.20.1 carver to the owning proto-chunk's deferred fluid marks
/// and legacy top-material callback. No postprocessing or surface hook is
/// optional in this constructor. `effects.flush` is mandatory after carving.
pub fn output_with_legacy1201_postprocessing<'a, L, A, T>(
    level: &'a mut L,
    policy: &'a Legacy1201CarverPolicy<'_, '_>,
    mask: &'a mut CarvingMask,
    seen: &'a mut HashSet<(i32, i32, i32)>,
    target: ChunkPos,
    aquifer: &'a mut A,
    effects: &'a mut Legacy1201CarverPostprocess,
    top_material: &'a mut T,
) -> io::Result<
    Legacy1201CarverWorldOutput<
        'a,
        L,
        impl FnMut(&mut L, BlockPos, &mut Legacy1201CarverColumn) -> io::Result<bool> + 'a,
    >,
>
where
    L: Legacy1201Level + 'a,
    A: FnMut(BlockPos) -> io::Result<Legacy1201AquiferSample> + 'a,
    T: FnMut(&L, BlockPos) -> io::Result<Option<BlockStateId>> + 'a,
{
    if effects.target() != target {
        return Err(invalid("1.20.1 postprocessing queue has wrong target chunk"));
    }
    let registry = policy.registry();
    Legacy1201CarverWorldOutput::new(
        level,
        registry,
        mask,
        seen,
        target,
        policy.debug(),
        move |level: &mut L, position, column| {
            let mut after_write = |level: &mut L, write| {
                effects.after_write(level, registry, write, top_material)
            };
            policy.apply(level, position, column, aquifer, &mut after_write)
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_queue_keeps_first_write_order_and_deduplicates() {
        let mut queue = Legacy1201CarverPostprocess::new(ChunkPos::new(-1, -1));
        let a = BlockPos::new(-16, 20, -1);
        let b = BlockPos::new(-16, 21, -1);
        queue.mark(a).unwrap();
        queue.mark(b).unwrap();
        queue.mark(a).unwrap();
        assert_eq!(queue.pending(), 2);
        let mut received = Vec::new();
        assert_eq!(
            queue.flush(|pos| {
                received.push(pos);
                Ok(())
            }).unwrap(),
            2
        );
        assert_eq!(received, vec![a, b]);
        assert!(queue.is_empty());
    }

    #[test]
    fn failed_protochunk_marker_keeps_remaining_positions() {
        let mut queue = Legacy1201CarverPostprocess::new(ChunkPos::new(0, 0));
        let positions = [
            BlockPos::new(0, -54, 0),
            BlockPos::new(0, -53, 0),
            BlockPos::new(0, -52, 0),
        ];
        for pos in positions {
            queue.mark(pos).unwrap();
        }
        let mut received = Vec::new();
        let result = queue.flush(|pos| {
            if pos == positions[1] {
                return Err(invalid("test marker failure"));
            }
            received.push(pos);
            Ok(())
        });
        assert!(result.is_err());
        assert_eq!(received, vec![positions[0]]);
        assert_eq!(queue.pending(), 2);
        assert_eq!(
            queue.flush(|pos| {
                received.push(pos);
                Ok(())
            }).unwrap(),
            2
        );
        assert_eq!(received, positions.to_vec());
    }

    #[test]
    fn cross_chunk_fluid_update_is_rejected() {
        let mut queue = Legacy1201CarverPostprocess::new(ChunkPos::new(-1, 0));
        assert!(queue.mark(BlockPos::new(0, 64, 0)).is_err());
        assert_eq!(queue.pending(), 0);
        queue.mark(BlockPos::new(-1, 64, 0)).unwrap();
        assert_eq!(queue.pending(), 1);
    }

    #[test]
    fn requeue_after_success_can_mark_again_in_later_phase() {
        let mut queue = Legacy1201CarverPostprocess::new(ChunkPos::new(1, 1));
        let at = BlockPos::new(16, 64, 16);
        queue.mark(at).unwrap();
        assert_eq!(queue.flush(|_| Ok(())).unwrap(), 1);
        queue.mark(at).unwrap();
        assert_eq!(queue.pending(), 1);
    }
}
