//! Stage 2.2 — Minecraft Java 1.20.1 (`DataVersion = 3465`) `CollectingNeighborUpdater`
//! (`dye.class`), `NeighborUpdater` (`dyg.class`), `LevelTicks` (`65536` block ticks then
//! `65536` fluid ticks), and `ServerLevel.tickChunk` LCG random tick engine.
//!
//! Preserves Horizon 26.3 performance optimizations:
//! - Reusable flat `stack` and `added` vectors (`Vec::try_reserve`) with in-place cursor
//!   advancement for `MultiNeighborUpdate` (zero per-update heap allocation in steady state).
//! - Deterministic `(trigger_tick, priority, sub_tick_order)` scheduling with `HashSet`
//!   deduplication matching vanilla `LevelChunkTicks`.

use crate::{
    compression::invalid,
    fluid::FluidKind,
    legacy_1_20_1_world::{LEGACY_1_20_1_DATA_VERSION, Legacy1201Level},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{geometry::Direction, position::BlockPos};
use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashSet, VecDeque},
    io, ptr,
};

/// 1.20.1 `NeighborUpdater.UPDATE_ORDER` (`dyg.a`):
/// `[WEST, EAST, DOWN, UP, NORTH, SOUTH]`.
pub const LEGACY1201_NEIGHBOR_UPDATE_ORDER: [Direction; 6] = [
    Direction::West,
    Direction::East,
    Direction::Down,
    Direction::Up,
    Direction::North,
    Direction::South,
];

/// 1.20.1 `BlockBehaviour.UPDATE_SHAPE_ORDER` (`dca.aO`):
/// `[DOWN, UP, NORTH, SOUTH, WEST, EAST]`.
pub const LEGACY1201_UPDATE_SHAPE_ORDER: [Direction; 6] = [
    Direction::Down,
    Direction::Up,
    Direction::North,
    Direction::South,
    Direction::West,
    Direction::East,
];

/// Default `maxChainedNeighborUpdates` in 1.20.1 (`ServerLevel` game rule / server property).
pub const LEGACY1201_MAX_CHAINED_NEIGHBOR_UPDATES: usize = 1_000_000;

/// Default recursion limit `Block.UPDATE_LIMIT` (`512`).
pub const LEGACY1201_UPDATE_LIMIT: i32 = 512;

/// Maximum scheduled ticks drained per `LevelTicks.tick` step in 1.20.1 (`ServerLevel.tick`).
pub const LEGACY1201_MAX_TICKS_PER_STEP: usize = 65_536;

/// Linear Congruential Generator multiplier and increment in `Level.getBlockRandomPos` (`cmm`).
pub const LEGACY1201_RANDOM_TICK_LCG_MUL: i32 = 3;
pub const LEGACY1201_RANDOM_TICK_LCG_ADD: i32 = 1_013_904_223;

/// `Level.setBlock` flags in 1.20.1 (`cpn` / `cmm`).
pub mod legacy1201_set_block_flags {
    pub const UPDATE_NEIGHBORS: i32 = 1;
    pub const UPDATE_CLIENTS: i32 = 2;
    pub const UPDATE_ALL: i32 = 3;
    pub const UPDATE_INVISIBLE: i32 = 4;
    pub const UPDATE_IMMEDIATE: i32 = 8;
    pub const UPDATE_KNOWN_SHAPE: i32 = 16;
    pub const UPDATE_SUPPRESS_DROPS: i32 = 32;
    pub const UPDATE_MOVE_BY_PISTON: i32 = 64;
}

/// Entry on the 1.20.1 `CollectingNeighborUpdater` stack (`dye$c`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201NeighborUpdateEntry {
    /// `ShapeUpdate(Direction, BlockState, BlockPos, BlockPos, int, int)` (`dye$d`).
    ShapeUpdate {
        direction: Direction,
        state: BlockStateId,
        pos: BlockPos,
        neighbor_pos: BlockPos,
        update_flags: i32,
        update_limit: i32,
    },
    /// `SimpleNeighborUpdate(BlockPos, Block, BlockPos)` (`dye$e`).
    SimpleNeighborUpdate {
        pos: BlockPos,
        source_block: BlockStateId,
        source_pos: BlockPos,
    },
    /// `FullNeighborUpdate(BlockState, BlockPos, Block, BlockPos, boolean)` (`dye$a`).
    FullNeighborUpdate {
        state: BlockStateId,
        pos: BlockPos,
        source_block: BlockStateId,
        source_pos: BlockPos,
        moved_by_piston: bool,
    },
    /// `MultiNeighborUpdate(BlockPos, Block, @Nullable Direction)` (`dye$b`).
    MultiNeighborUpdate {
        source_pos: BlockPos,
        source_block: BlockStateId,
        skip_direction: Option<Direction>,
        next_index: usize,
    },
}

/// Observable step event emitted during `Legacy1201CollectingNeighborUpdater::drain`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201DispatchedUpdate {
    NeighborChanged {
        pos: BlockPos,
        state: BlockStateId,
        source_block: BlockStateId,
        source_pos: BlockPos,
        moved_by_piston: bool,
    },
    ShapeUpdate {
        direction: Direction,
        pos: BlockPos,
        neighbor_pos: BlockPos,
        neighbor_state: BlockStateId,
        update_flags: i32,
        update_limit: i32,
    },
}

/// 1.20.1 `CollectingNeighborUpdater` (`dye.class`) with Horizon 26.3 flat buffer reuse.
#[derive(Clone, Debug)]
pub struct Legacy1201CollectingNeighborUpdater {
    stack: Vec<Legacy1201NeighborUpdateEntry>,
    added_this_layer: Vec<Legacy1201NeighborUpdateEntry>,
    count: usize,
    max_chained_neighbor_updates: usize,
    dropped_over_limit: usize,
}

impl Default for Legacy1201CollectingNeighborUpdater {
    fn default() -> Self {
        Self::new(LEGACY1201_MAX_CHAINED_NEIGHBOR_UPDATES)
    }
}

impl Legacy1201CollectingNeighborUpdater {
    pub fn new(max_chained_neighbor_updates: usize) -> Self {
        Self {
            stack: Vec::new(),
            added_this_layer: Vec::new(),
            count: 0,
            max_chained_neighbor_updates,
            dropped_over_limit: 0,
        }
    }

    #[inline]
    pub fn count(&self) -> usize {
        self.count
    }

    #[inline]
    pub fn dropped_over_limit(&self) -> usize {
        self.dropped_over_limit
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.stack.is_empty() && self.added_this_layer.is_empty()
    }

    pub fn shape_update(
        &mut self,
        direction: Direction,
        state: BlockStateId,
        pos: BlockPos,
        neighbor_pos: BlockPos,
        update_flags: i32,
        update_limit: i32,
    ) {
        self.enqueue(Legacy1201NeighborUpdateEntry::ShapeUpdate {
            direction,
            state,
            pos,
            neighbor_pos,
            update_flags,
            update_limit,
        });
    }

    pub fn neighbor_changed(
        &mut self,
        pos: BlockPos,
        source_block: BlockStateId,
        source_pos: BlockPos,
    ) {
        self.enqueue(Legacy1201NeighborUpdateEntry::SimpleNeighborUpdate {
            pos,
            source_block,
            source_pos,
        });
    }

    pub fn neighbor_changed_state(
        &mut self,
        state: BlockStateId,
        pos: BlockPos,
        source_block: BlockStateId,
        source_pos: BlockPos,
        moved_by_piston: bool,
    ) {
        self.enqueue(Legacy1201NeighborUpdateEntry::FullNeighborUpdate {
            state,
            pos,
            source_block,
            source_pos,
            moved_by_piston,
        });
    }

    pub fn update_neighbors_at_except_from_facing(
        &mut self,
        source_pos: BlockPos,
        source_block: BlockStateId,
        skip_direction: Option<Direction>,
    ) {
        let mut next_index = 0;
        if Some(LEGACY1201_NEIGHBOR_UPDATE_ORDER[0]) == skip_direction {
            next_index = 1;
        }
        self.enqueue(Legacy1201NeighborUpdateEntry::MultiNeighborUpdate {
            source_pos,
            source_block,
            skip_direction,
            next_index,
        });
    }

    fn enqueue(&mut self, entry: Legacy1201NeighborUpdateEntry) {
        let already_running = !self.stack.is_empty() || !self.added_this_layer.is_empty();
        if self.count < self.max_chained_neighbor_updates {
            self.count += 1;
            if already_running {
                self.added_this_layer.push(entry);
            } else {
                self.stack.push(entry);
            }
        } else {
            self.dropped_over_limit += 1;
        }
    }

    /// Execute the `CollectingNeighborUpdater.runUpdates()` (`dye.a()`) loop against a live
    /// `Legacy1201Level`, invoking `handler` for each concrete neighbor or shape update.
    /// The `handler` may enqueue child updates on `self` (which land in `added_this_layer`
    /// and are pushed in reverse order onto `stack`, matching Java 1.20.1).
    pub fn run_updates<L, F>(&mut self, level: &mut L, mut handler: F) -> io::Result<usize>
    where
        L: Legacy1201Level + ?Sized,
        F: FnMut(&mut Self, &mut L, Legacy1201DispatchedUpdate) -> io::Result<()>,
    {
        if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
            || level.collision_block_registry().len() != 24_135
        {
            return Err(invalid("1.20.1 CollectingNeighborUpdater level version/registry mismatch"));
        }

        let mut executed = 0usize;
        while !self.stack.is_empty() || !self.added_this_layer.is_empty() {
            if !self.added_this_layer.is_empty() {
                for entry in self.added_this_layer.drain(..).rev() {
                    self.stack.push(entry);
                }
            }

            while self.added_this_layer.is_empty() {
                let Some(top) = self.stack.last().copied() else {
                    break;
                };
                let keep_on_stack = match top {
                    Legacy1201NeighborUpdateEntry::ShapeUpdate {
                        direction,
                        state,
                        pos,
                        neighbor_pos,
                        update_flags,
                        update_limit,
                    } => {
                        if update_limit > 0 {
                            executed += 1;
                            handler(
                                self,
                                level,
                                Legacy1201DispatchedUpdate::ShapeUpdate {
                                    direction,
                                    pos,
                                    neighbor_pos,
                                    neighbor_state: state,
                                    update_flags,
                                    update_limit,
                                },
                            )?;
                        }
                        false
                    }
                    Legacy1201NeighborUpdateEntry::SimpleNeighborUpdate {
                        pos,
                        source_block,
                        source_pos,
                    } => {
                        let state = level.block_state(pos);
                        executed += 1;
                        handler(
                            self,
                            level,
                            Legacy1201DispatchedUpdate::NeighborChanged {
                                pos,
                                state,
                                source_block,
                                source_pos,
                                moved_by_piston: false,
                            },
                        )?;
                        false
                    }
                    Legacy1201NeighborUpdateEntry::FullNeighborUpdate {
                        state,
                        pos,
                        source_block,
                        source_pos,
                        moved_by_piston,
                    } => {
                        executed += 1;
                        handler(
                            self,
                            level,
                            Legacy1201DispatchedUpdate::NeighborChanged {
                                pos,
                                state,
                                source_block,
                                source_pos,
                                moved_by_piston,
                            },
                        )?;
                        false
                    }
                    Legacy1201NeighborUpdateEntry::MultiNeighborUpdate {
                        source_pos,
                        source_block,
                        skip_direction,
                        next_index,
                    } => {
                        let dir = LEGACY1201_NEIGHBOR_UPDATE_ORDER[next_index];
                        let [dx, dy, dz] = dir.normal();
                        let neighbor_pos = source_pos.offset(dx, dy, dz);
                        let state = level.block_state(neighbor_pos);

                        let mut idx = next_index + 1;
                        if idx < LEGACY1201_NEIGHBOR_UPDATE_ORDER.len()
                            && Some(LEGACY1201_NEIGHBOR_UPDATE_ORDER[idx]) == skip_direction
                        {
                            idx += 1;
                        }
                        if let Some(slot) = self.stack.last_mut() {
                            *slot = Legacy1201NeighborUpdateEntry::MultiNeighborUpdate {
                                source_pos,
                                source_block,
                                skip_direction,
                                next_index: idx,
                            };
                        }

                        executed += 1;
                        handler(
                            self,
                            level,
                            Legacy1201DispatchedUpdate::NeighborChanged {
                                pos: neighbor_pos,
                                state,
                                source_block,
                                source_pos,
                                moved_by_piston: false,
                            },
                        )?;
                        idx < LEGACY1201_NEIGHBOR_UPDATE_ORDER.len()
                    }
                };

                if !keep_on_stack {
                    self.stack.pop();
                    break;
                }
            }
        }

        self.stack.clear();
        self.added_this_layer.clear();
        self.count = 0;
        Ok(executed)
    }
}

/// 1.20.1 `TickPriority` (`dxz.class`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(i32)]
pub enum Legacy1201TickPriority {
    ExtremelyHigh = -3,
    VeryHigh = -2,
    High = -1,
    #[default]
    Normal = 0,
    Low = 1,
    VeryLow = 2,
    ExtremelyLow = 3,
}

/// 1.20.1 `ScheduledTick<T>` (`dya.class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201ScheduledTick<T: Copy + Eq> {
    pub kind: T,
    pub pos: BlockPos,
    pub trigger_tick: i64,
    pub priority: Legacy1201TickPriority,
    pub sub_tick_order: u64,
}

impl<T: Copy + Eq> Ord for Legacy1201ScheduledTick<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering for max-heap `BinaryHeap` so earliest tick / highest priority pops first.
        other
            .trigger_tick
            .cmp(&self.trigger_tick)
            .then_with(|| other.priority.cmp(&self.priority))
            .then_with(|| other.sub_tick_order.cmp(&self.sub_tick_order))
    }
}

impl<T: Copy + Eq> PartialOrd for Legacy1201ScheduledTick<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

pub trait Legacy1201TickTarget: Copy + Eq {
    type Key: Copy + Eq + std::hash::Hash;
    fn tick_key(self) -> Self::Key;
}

impl Legacy1201TickTarget for BlockStateId {
    type Key = BlockStateId;
    #[inline]
    fn tick_key(self) -> Self::Key {
        self
    }
}

impl Legacy1201TickTarget for FluidKind {
    type Key = u8;
    #[inline]
    fn tick_key(self) -> Self::Key {
        self as u8
    }
}

/// Single-domain `LevelTicks<T>` queue (used for `blockTicks` and `fluidTicks` in 1.20.1).
#[derive(Clone, Debug)]
pub struct Legacy1201DomainTicks<T: Legacy1201TickTarget> {
    queue: BinaryHeap<Legacy1201ScheduledTick<T>>,
    scheduled_set: HashSet<(BlockPos, T::Key)>,
    to_run_this_tick: VecDeque<Legacy1201ScheduledTick<T>>,
}

impl<T: Legacy1201TickTarget> Default for Legacy1201DomainTicks<T> {
    fn default() -> Self {
        Self {
            queue: BinaryHeap::new(),
            scheduled_set: HashSet::new(),
            to_run_this_tick: VecDeque::new(),
        }
    }
}

impl<T: Legacy1201TickTarget> Legacy1201DomainTicks<T> {
    pub fn schedule(&mut self, tick: Legacy1201ScheduledTick<T>) -> bool {
        if self.scheduled_set.insert((tick.pos, tick.kind.tick_key())) {
            self.queue.push(tick);
            true
        } else {
            false
        }
    }

    pub fn has_scheduled(&self, pos: BlockPos, kind: T) -> bool {
        self.scheduled_set.contains(&(pos, kind.tick_key()))
    }

    pub fn len(&self) -> usize {
        self.queue.len() + self.to_run_this_tick.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// 1.20.1 `LevelTicks.tick(long gameTime, int maxTicks, BiConsumer<BlockPos, T> ticker)`:
    /// collects up to `max_ticks` due ticks (`trigger_tick <= game_time`), then runs them
    /// in priority order.
    pub fn tick<F>(&mut self, game_time: i64, max_ticks: usize, mut ticker: F) -> io::Result<usize>
    where
        F: FnMut(BlockPos, T) -> io::Result<()>,
    {
        while self.to_run_this_tick.len() < max_ticks {
            let Some(&next) = self.queue.peek() else {
                break;
            };
            if next.trigger_tick > game_time {
                break;
            }
            let popped = self.queue.pop().unwrap();
            self.to_run_this_tick.push_back(popped);
        }

        let mut executed = 0usize;
        while let Some(item) = self.to_run_this_tick.pop_front() {
            self.scheduled_set.remove(&(item.pos, item.kind.tick_key()));
            executed += 1;
            ticker(item.pos, item.kind)?;
        }
        Ok(executed)
    }
}
/// Combined 1.20.1 `ServerLevel` tick scheduler (`blockTicks` + `fluidTicks` + random tick LCG).
#[derive(Clone, Debug)]
pub struct Legacy1201ServerTickEngine {
    pub block_ticks: Legacy1201DomainTicks<BlockStateId>,
    pub fluid_ticks: Legacy1201DomainTicks<FluidKind>,
    sub_tick_counter: u64,
    random_tick_lcg: i32,
}

impl Legacy1201ServerTickEngine {
    pub fn new(random_tick_seed: i32) -> Self {
        Self {
            block_ticks: Legacy1201DomainTicks::default(),
            fluid_ticks: Legacy1201DomainTicks::default(),
            sub_tick_counter: 0,
            random_tick_lcg: random_tick_seed,
        }
    }

    #[inline]
    pub fn random_tick_lcg(&self) -> i32 {
        self.random_tick_lcg
    }

    #[inline]
    pub fn next_sub_tick(&mut self) -> u64 {
        self.sub_tick_counter = self.sub_tick_counter.wrapping_add(1);
        self.sub_tick_counter
    }

    pub fn schedule_block_tick(
        &mut self,
        game_time: i64,
        pos: BlockPos,
        block: BlockStateId,
        delay: i32,
        priority: Legacy1201TickPriority,
    ) -> bool {
        let sub_tick_order = self.next_sub_tick();
        self.block_ticks.schedule(Legacy1201ScheduledTick {
            kind: block,
            pos,
            trigger_tick: game_time.wrapping_add(i64::from(delay.max(0))),
            priority,
            sub_tick_order,
        })
    }

    pub fn schedule_fluid_tick(
        &mut self,
        game_time: i64,
        pos: BlockPos,
        fluid: FluidKind,
        delay: i32,
    ) -> bool {
        let sub_tick_order = self.next_sub_tick();
        self.fluid_ticks.schedule(Legacy1201ScheduledTick {
            kind: fluid,
            pos,
            trigger_tick: game_time.wrapping_add(i64::from(delay.max(0))),
            priority: Legacy1201TickPriority::Normal,
            sub_tick_order,
        })
    }

    /// 1.20.1 `ServerLevel.tick`: runs up to `65,536` due block ticks first, then up to
    /// `65,536` due fluid ticks.
    pub fn run_scheduled_ticks<FB, FF>(
        &mut self,
        game_time: i64,
        block_ticker: FB,
        fluid_ticker: FF,
    ) -> io::Result<(usize, usize)>
    where
        FB: FnMut(BlockPos, BlockStateId) -> io::Result<()>,
        FF: FnMut(BlockPos, FluidKind) -> io::Result<()>,
    {
        let blocks = self
            .block_ticks
            .tick(game_time, LEGACY1201_MAX_TICKS_PER_STEP, block_ticker)?;
        let fluids = self
            .fluid_ticks
            .tick(game_time, LEGACY1201_MAX_TICKS_PER_STEP, fluid_ticker)?;
        Ok((blocks, fluids))
    }

    /// 1.20.1 `Level.getBlockRandomPos(int x, int y, int z, int yMask)` (`cmm.a(IIII)`).
    #[inline]
    pub fn next_block_random_pos(&mut self, x: i32, y: i32, z: i32, y_mask: i32) -> BlockPos {
        self.random_tick_lcg = self
            .random_tick_lcg
            .wrapping_mul(LEGACY1201_RANDOM_TICK_LCG_MUL)
            .wrapping_add(LEGACY1201_RANDOM_TICK_LCG_ADD);
        let v = self.random_tick_lcg >> 2;
        BlockPos::new(
            x + (v & 15),
            y + ((v >> 16) & y_mask),
            z + ((v >> 8) & 15),
        )
    }

    /// Generate the exact `randomTickSpeed` positions for a `16x16x16` chunk section in
    /// `ServerLevel.tickChunk` (`aif.a(LevelChunk, int)`).
    pub fn sample_section_random_tick_positions(
        &mut self,
        chunk_min_x: i32,
        section_min_y: i32,
        chunk_min_z: i32,
        random_tick_speed: u32,
    ) -> io::Result<Vec<BlockPos>> {
        let mut out = Vec::new();
        out.try_reserve_exact(random_tick_speed as usize)
            .map_err(|_| invalid("out of memory allocating random tick positions"))?;
        for _ in 0..random_tick_speed {
            out.push(self.next_block_random_pos(chunk_min_x, section_min_y, chunk_min_z, 15));
        }
        Ok(out)
    }
}

/// Propagate `BlockState.updateNeighbourShapes` in 1.20.1 `UPDATE_SHAPE_ORDER`
/// (`[DOWN, UP, NORTH, SOUTH, WEST, EAST]`).
pub fn legacy1201_enqueue_neighbor_shapes<L: Legacy1201Level + ?Sized>(
    updater: &mut Legacy1201CollectingNeighborUpdater,
    level: &L,
    registry: &BlockRegistry,
    state: BlockStateId,
    pos: BlockPos,
    flags: i32,
    recursion_left: i32,
) -> io::Result<()> {
    if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
        || !ptr::eq(level.collision_block_registry(), registry)
        || registry.len() != 24_135
    {
        return Err(invalid("1.20.1 updateNeighbourShapes registry/version mismatch"));
    }
    if recursion_left <= 0 {
        return Ok(());
    }
    for dir in LEGACY1201_UPDATE_SHAPE_ORDER {
        let [dx, dy, dz] = dir.normal();
        let neighbor_pos = pos.offset(dx, dy, dz);
        updater.shape_update(
            dir.opposite(),
            state,
            neighbor_pos,
            pos,
            flags,
            recursion_left,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_1_20_1_block_survival::tests::build_synthetic_24135_registry;
    use crate::level::LevelAccess;
    use horizon_core::random::LegacyRandom;
    use std::collections::HashMap;

    struct SimpleLevel<'r> {
        registry: &'r BlockRegistry,
        air: BlockStateId,
        blocks: HashMap<BlockPos, BlockStateId>,
        random: LegacyRandom,
    }

    impl<'r> SimpleLevel<'r> {
        fn new(registry: &'r BlockRegistry) -> Self {
            Self {
                registry,
                air: registry.default_state("minecraft:air").unwrap(),
                blocks: HashMap::new(),
                random: LegacyRandom::new(7),
            }
        }
    }

    impl LevelAccess for SimpleLevel<'_> {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.blocks.get(&pos).copied().unwrap_or(self.air)
        }
        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            Some(self.blocks.insert(pos, state).unwrap_or(self.air))
        }
        fn is_loaded(&self, _pos: BlockPos) -> bool {
            true
        }
        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }
        fn game_time(&self) -> i64 {
            0
        }
        fn day_time(&self) -> i64 {
            0
        }
        fn block_light(&self, _pos: BlockPos) -> i32 {
            0
        }
    }

    impl Legacy1201Level for SimpleLevel<'_> {
        fn collision_data_version(&self) -> i32 {
            LEGACY_1_20_1_DATA_VERSION
        }
        fn collision_block_registry(&self) -> &BlockRegistry {
            self.registry
        }
    }

    #[test]
    fn collecting_neighbor_updater_visits_six_directions_in_1201_order_and_nests_depth_first() {
        let registry = build_synthetic_24135_registry();
        let mut level = SimpleLevel::new(&registry);
        let mut updater = Legacy1201CollectingNeighborUpdater::default();

        let center = BlockPos::new(10, 64, 10);
        let stone = registry.default_state("minecraft:stone").unwrap();
        let dirt = registry.default_state("minecraft:dirt").unwrap();

        updater.update_neighbors_at_except_from_facing(center, stone, None);

        let mut trace = Vec::new();
        updater
            .run_updates(&mut level, |u, _lvl, ev| {
                if let Legacy1201DispatchedUpdate::NeighborChanged {
                    pos, source_block, ..
                } = ev
                {
                    trace.push((pos, source_block));
                    // When the first neighbor (West) fires, schedule a child SimpleNeighborUpdate;
                    // it must execute BEFORE the remaining 5 directions of the parent MultiNeighborUpdate!
                    if pos == center.offset(-1, 0, 0) && source_block == stone {
                        u.neighbor_changed(BlockPos::new(99, 64, 99), dirt, pos);
                    }
                }
                Ok(())
            })
            .unwrap();

        assert_eq!(
            trace,
            vec![
                (center.offset(-1, 0, 0), stone), // West
                (BlockPos::new(99, 64, 99), dirt), // Child runs immediately before East!
                (center.offset(1, 0, 0), stone),  // East
                (center.offset(0, -1, 0), stone), // Down
                (center.offset(0, 1, 0), stone),  // Up
                (center.offset(0, 0, -1), stone), // North
                (center.offset(0, 0, 1), stone),  // South
            ]
        );
    }

    #[test]
    fn server_tick_engine_orders_priorities_and_matches_1201_lcg() {
        let registry = build_synthetic_24135_registry();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let mut engine = Legacy1201ServerTickEngine::new(123456789);

        let p1 = BlockPos::new(1, 64, 1);
        let p2 = BlockPos::new(2, 64, 2);
        let p3 = BlockPos::new(3, 64, 3);

        // Duplicate (p1, stone) should be ignored.
        assert!(engine.schedule_block_tick(10, p1, stone, 5, Legacy1201TickPriority::Low));
        assert!(!engine.schedule_block_tick(10, p1, stone, 1, Legacy1201TickPriority::ExtremelyHigh));

        // Same trigger tick (15), higher priority (High < Normal < Low) runs first.
        assert!(engine.schedule_block_tick(10, p2, stone, 5, Legacy1201TickPriority::Normal));
        assert!(engine.schedule_block_tick(10, p3, stone, 5, Legacy1201TickPriority::High));
        assert!(engine.schedule_fluid_tick(10, p1, FluidKind::Water, 5));

        let order = std::cell::RefCell::new(Vec::new());
        let (b_count, f_count) = engine
            .run_scheduled_ticks(
                15,
                |pos, _| {
                    order.borrow_mut().push((false, pos));
                    Ok(())
                },
                |pos, _| {
                    order.borrow_mut().push((true, pos));
                    Ok(())
                },
            )
            .unwrap();

        assert_eq!((b_count, f_count), (3, 1));
        assert_eq!(
            order.into_inner(),
            vec![
                (false, p3), // High (-1)
                (false, p2), // Normal (0)
                (false, p1), // Low (1)
                (true, p1),  // Fluid tick runs after all block ticks
            ]
        );

        let positions = engine
            .sample_section_random_tick_positions(16, 64, 32, 3)
            .unwrap();
        assert_eq!(positions.len(), 3);
        for p in positions {
            assert!((16..32).contains(&p.x));
            assert!((64..80).contains(&p.y));
            assert!((32..48).contains(&p.z));
        }
    }
}
