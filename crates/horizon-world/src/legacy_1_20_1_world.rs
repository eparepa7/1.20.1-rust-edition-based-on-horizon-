//! Bind the separately versioned Java 1.20.1 collision data to a live level.
//!
//! Block states come from `LevelAccess`, not a stand-alone NBT fixture. Dynamic
//! block entities and entity-dependent collision decisions come from the same
//! live world through the existing `DynamicCollisionWorld`,
//! `LiquidCollisionWorld`, and `PowderSnowWorld` contracts.
//!
//! This is a library adapter, not a game executable. The client/game owns
//! chunk storage, loaded-chunk tracking and live entity/block-entity queries.
//!
//! Example (with an already prepared 1.20.1 `BlockCollisionRegistry`):
//! ```ignore
//! let mut live_collision = Legacy1201CollisionWorld::new(&collisions, &mut game_level)?;
//! let scan = BlockCollisionScan::new(
//!     &mut live_collision, context, player_bounds, false, candidate_budget,
//! )?;
//! for hit in scan {
//!     let hit = hit?;
//!     // Resolve movement using hit.position and hit.shape.
//! }
//! ```

use crate::{
    block_collision::{BlockCollisionRegistry, CollisionQueryError, DynamicCollisionWorld},
    block_collision_scan::{BlockCollisionWorld, CollisionBlock},
    collision_context::CollisionContext,
    compression::invalid,
    level::LevelAccess,
    liquid_collision::LiquidCollisionWorld,
    powder_snow_collision::PowderSnowWorld,
    registry::{BlockRegistry, BlockStateId},
    suffocation::SuffocationRegistry,
};
use horizon_core::{
    geometry::VoxelShape,
    position::{BlockPos, ChunkPos},
};
use std::{io, ptr};

pub const LEGACY_1_20_1_DATA_VERSION: i32 = 3465;

/// Probes the chunk origin without overflowing `BlockPos`'s i32 coordinates.
/// Impossible chunk origins are not allowed to masquerade as loaded chunks.
fn chunk_probe(position: ChunkPos, min_y: i32) -> Option<BlockPos> {
    Some(BlockPos::new(
        position.x.checked_mul(16)?,
        min_y,
        position.z.checked_mul(16)?,
    ))
}


/// Identity of a live 1.20.1 world. The state IDs stored in `LevelAccess`
/// must come from *the exact registry instance* used for collision data.
/// Without this check, state IDs from 26.3 can silently select unrelated
/// 1.20.1 blocks even when both integer IDs are valid.
pub trait Legacy1201Level: LevelAccess {
    fn collision_data_version(&self) -> i32;
    fn collision_block_registry(&self) -> &BlockRegistry;
}

/// 1.20.1 world collision provider, independent of the active 26.3 path.
///
/// Construct once per query or borrow it for a series of scans. The provider
/// holds no cached block states, so changes made to `LevelAccess` between
/// scans are observed the next time a block is queried.
pub struct Legacy1201CollisionWorld<'w, 'r, L> {
    collisions: &'w BlockCollisionRegistry<'r>,
    suffocation: Option<&'w SuffocationRegistry<'r>>,
    level: &'w mut L,
}

impl<'w, 'r, L: Legacy1201Level> Legacy1201CollisionWorld<'w, 'r, L> {
    pub fn new(collisions: &'w BlockCollisionRegistry<'r>, level: &'w mut L) -> io::Result<Self> {
        let collision_data = collisions.cached_shapes();
        if collision_data.data_version() != LEGACY_1_20_1_DATA_VERSION
            || level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
        {
            return Err(invalid("1.20.1 world collision DataVersion mismatch"));
        }
        if !ptr::eq(level.collision_block_registry(), collisions.registry()) {
            return Err(invalid("1.20.1 world uses a different block-state registry"));
        }
        if !collision_data.has_canonical_identities() {
            return Err(invalid("1.20.1 collision scan needs canonical shape identities"));
        }
        if collisions.registry().len() != 24_135 {
            return Err(invalid("1.20.1 vanilla collision registry must have 24135 states"));
        }
        // Vanilla 1.20.1 dimensions may have different bounds (the Overworld
        // defaults to -64..320, Nether/End to 0..256). Reject invalid storage,
        // not legitimate non-Overworld dimensions.
        if level.max_y() <= level.min_y()
            || level.min_y().rem_euclid(16) != 0
            || level.max_y().rem_euclid(16) != 0
        {
            return Err(invalid("invalid 1.20.1 world build bounds"));
        }
        Ok(Self {
            collisions,
            suffocation: None,
            level,
        })
    }

    /// Enable the `only_suffocating` scan mode using the vanilla rule table.
    /// Ordinary collision scans need no separate suffocation export.
    pub fn with_suffocation(mut self, table: &'w SuffocationRegistry<'r>) -> io::Result<Self> {
        if !ptr::eq(table.collisions(), self.collisions) {
            return Err(invalid("1.20.1 suffocation rules use another collision registry"));
        }
        self.suffocation = Some(table);
        Ok(self)
    }

    /// Query a block's collision using its *current* world state and entity
    /// context. Unloaded chunks are errors, not silently treated as air.
    pub fn collision_shape<E>(
        &mut self,
        position: BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, CollisionQueryError<E>>
    where
        L: DynamicCollisionWorld<Error = E>
            + LiquidCollisionWorld<Error = E>
            + PowderSnowWorld<Error = E>,
    {
        if !self.is_chunk_loaded(position.chunk()) {
            return Err(CollisionQueryError::Data(invalid(
                "1.20.1 collision query for an unloaded chunk",
            )));
        }
        let state = self.level.block_state(position);
        self.collisions
            .collision_shape_with_context(self.level, state, position, context)
    }

    /// A block-space probe for a loaded chunk. `ChunkPos` can represent more
    /// chunk coordinates than `BlockPos` can represent blocks; wrapping the
    /// multiplication would alias an out-of-range chunk to an unrelated one.
    /// Negative coordinates deliberately retain Java's floor-division layout.
    fn is_chunk_loaded(&self, position: ChunkPos) -> bool {
        chunk_probe(position, self.level.min_y())
            .is_some_and(|probe| self.level.is_loaded(probe))
    }

    /// A scan first reads the state and then calculates its collision. Reject
    /// a mid-scan block update rather than calculating a dynamic shape for a
    /// state that no longer exists at this position.
    fn check_snapshot<E>(
        &self,
        position: BlockPos,
        expected: BlockStateId,
    ) -> Result<(), CollisionQueryError<E>> {
        if self.level.block_state(position) != expected {
            return Err(CollisionQueryError::Data(invalid(
                "1.20.1 collision state changed during scan",
            )));
        }
        Ok(())
    }
}

impl<L, E> BlockCollisionWorld for Legacy1201CollisionWorld<'_, '_, L>
where
    L: Legacy1201Level
        + DynamicCollisionWorld<Error = E>
        + LiquidCollisionWorld<Error = E>
        + PowderSnowWorld<Error = E>,
{
    type Error = CollisionQueryError<E>;
    type Chunk = ChunkPos;

    fn collision_chunk(&mut self, position: ChunkPos) -> Result<Option<Self::Chunk>, Self::Error> {
        Ok(self.is_chunk_loaded(position).then_some(position))
    }

    fn collision_block(
        &mut self,
        chunk: Self::Chunk,
        position: BlockPos,
    ) -> Result<CollisionBlock, Self::Error> {
        if position.chunk() != chunk || !self.is_chunk_loaded(chunk) {
            return Err(CollisionQueryError::Data(invalid(
                "1.20.1 collision scan accessed a missing or wrong chunk",
            )));
        }
        let state = self.level.block_state(position);
        self.collisions
            .collision_block(state)
            .map_err(CollisionQueryError::Data)
    }

    fn is_suffocating(
        &mut self,
        chunk: Self::Chunk,
        block: CollisionBlock,
        position: BlockPos,
    ) -> Result<bool, Self::Error> {
        if position.chunk() != chunk || !self.is_chunk_loaded(chunk) {
            return Err(CollisionQueryError::Data(invalid(
                "1.20.1 suffocation query accessed a missing or wrong chunk",
            )));
        }
        self.check_snapshot(position, block.state)?;
        let table = self.suffocation.ok_or_else(|| {
            CollisionQueryError::Data(invalid(
                "1.20.1 suffocation query requires a matching suffocation table",
            ))
        })?;
        table.is_suffocating(self.level, block.state, [position.x, position.y, position.z])
    }

    fn context_shape(
        &mut self,
        block: CollisionBlock,
        position: BlockPos,
        context: &CollisionContext<'_>,
    ) -> Result<VoxelShape, Self::Error> {
        if !self.is_chunk_loaded(position.chunk()) {
            return Err(CollisionQueryError::Data(invalid(
                "1.20.1 contextual collision query accessed an unloaded chunk",
            )));
        }
        self.check_snapshot(position, block.state)?;
        self.collisions.collision_shape_with_context(
            self.level,
            block.state,
            position,
            context,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_chunk_origin_uses_floor_coordinates() {
        assert_eq!(
            chunk_probe(ChunkPos::new(-1, -2), -64),
            Some(BlockPos::new(-16, -64, -32)),
        );
        assert_eq!(
            chunk_probe(ChunkPos::new(0, 0), 0),
            Some(BlockPos::new(0, 0, 0)),
        );
    }

    #[test]
    fn last_representable_chunk_origins_do_not_wrap() {
        let smallest = i32::MIN / 16;
        let largest = i32::MAX / 16;
        assert_eq!(
            chunk_probe(
                ChunkPos::new(smallest, largest), -64,
            ),
            Some(BlockPos::new(i32::MIN, -64, largest * 16)),
        );
        assert_eq!(
            chunk_probe(
                ChunkPos::new(largest, smallest), 0,
            ),
            Some(BlockPos::new(largest * 16, 0, i32::MIN)),
        );
    }

    #[test]
    fn out_of_block_range_chunks_cannot_alias_other_chunks() {
        let before_min = i32::MIN / 16 - 1;
        let after_max = i32::MAX / 16 + 1;
        for at in [
            ChunkPos::new(before_min, 0),
            ChunkPos::new(0, before_min),
            ChunkPos::new(after_max, 0),
            ChunkPos::new(0, after_max),
            ChunkPos::new(i32::MIN, i32::MAX),
        ] {
            assert_eq!(chunk_probe(at, -64), None);
        }
    }

    #[test]
    fn dimension_probe_preserves_given_minimum_height() {
        for y in [-64, 0, 64, 256] {
            let position = chunk_probe(
                ChunkPos::new(-31, 17), y,
            );
            assert_eq!(position, Some(BlockPos::new(-496, y, 272)));
        }
    }
}
