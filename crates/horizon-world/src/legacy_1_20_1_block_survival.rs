//! Stage 2.1 — Minecraft Java 1.20.1 (`DataVersion = 3465`) block survival,
//! cactus growth/destruction, ice/frosted-ice melting, and water/lava interactions.
//!
//! Preserves Horizon 26.3 performance optimizations:
//! - Flat preallocated per-state metadata table (`Vec::try_reserve_exact`) indexed by
//!   `BlockStateId(u32)` in O(1) with zero heap allocations or string lookups in hot loops.
//! - Strict `Legacy1201Level` guard (`DataVersion == 3465`, `24_135` states, pointer-identical
//!   `BlockRegistry`).

use crate::{
    compression::invalid,
    legacy_1_20_1_world::{LEGACY_1_20_1_DATA_VERSION, Legacy1201Level},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{geometry::Direction, position::BlockPos};
use std::{io, ptr};

/// Vanilla 1.20.1 `Direction.Plane.HORIZONTAL` iteration order (`ha$c.a`):
/// `NORTH, EAST, SOUTH, WEST`.
pub const LEGACY1201_HORIZONTAL_DIRECTIONS: [Direction; 4] = [
    Direction::North,
    Direction::East,
    Direction::South,
    Direction::West,
];

/// Vanilla 1.20.1 `LiquidBlock.POSSIBLE_FLOW_DIRECTIONS` (`cua.d`):
/// `DOWN, SOUTH, NORTH, EAST, WEST`.
/// Note: `LiquidBlock.shouldSpreadLiquid` probes `pos.relative(dir.getOpposite())`,
/// which visits `UP, NORTH, SOUTH, WEST, EAST`.
pub const LEGACY1201_LIQUID_FLOW_DIRECTIONS: [Direction; 5] = [
    Direction::Down,
    Direction::South,
    Direction::North,
    Direction::East,
    Direction::West,
];

/// Level event ID emitted by `LiquidBlock.fizz` (`1501`).
pub const LEGACY1201_LAVA_FIZZ_EVENT: i32 = 1501;

// Per-state bitflags precomputed at table construction time.
const FLAG_AIR: u16 = 1 << 0;
const FLAG_CACTUS: u16 = 1 << 1;
const FLAG_SAND_TAG: u16 = 1 << 2;
const FLAG_DIRT_TAG: u16 = 1 << 3;
const FLAG_WATER_FLUID: u16 = 1 << 4;
const FLAG_LAVA_FLUID: u16 = 1 << 5;
const FLAG_LIQUID_BLOCK: u16 = 1 << 6;
const FLAG_SOLID: u16 = 1 << 7;
const FLAG_BLOCKS_MOTION: u16 = 1 << 8;
const FLAG_ICE: u16 = 1 << 9;
const FLAG_FROSTED_ICE: u16 = 1 << 10;
const FLAG_BLUE_ICE: u16 = 1 << 11;
const FLAG_SOUL_SOIL: u16 = 1 << 12;
const FLAG_SUGAR_CANE: u16 = 1 << 13;
const FLAG_SNOW_LAYER: u16 = 1 << 14;
const FLAG_CANNOT_SUPPORT_SNOW: u16 = 1 << 15;

const FLAG2_SNOW_SUPPORT_OVERRIDE: u8 = 1 << 0;
const FLAG2_FULL_UP_SUPPORT: u8 = 1 << 1;

/// Compact metadata entry per `BlockStateId` for O(1) hot-path lookups.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201StateSurvivalMeta {
    pub flags: u16,
    pub flags2: u8,
    /// `age` property (`0..=15` for cactus/sugar_cane, `0..=3` for frosted_ice).
    pub age: u8,
    /// `level` property for `water`/`lava` (`0..=15`), or `layers` (`1..=8`) for `snow`.
    pub level_or_layers: u8,
    /// Light opacity (`BlockState.getLightBlock`) used by `IceBlock` / `FrostedIceBlock`.
    pub light_block: u8,
    /// Precomputed next-age state ID (for cactus `age + 1` or `0`, frosted ice `age + 1`).
    pub next_age_state: BlockStateId,
}

impl Default for Legacy1201StateSurvivalMeta {
    #[inline]
    fn default() -> Self {
        Self {
            flags: 0,
            flags2: 0,
            age: 0,
            level_or_layers: 0,
            light_block: 0,
            next_age_state: BlockStateId(0),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201CactusTickResult {
    Survived,
    DestroyedToAir(BlockStateId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201CactusRandomTickResult {
    BlockedAbove,
    MaxHeightReached(usize),
    Aged {
        new_state: BlockStateId,
        new_age: u8,
    },
    Grew {
        base_reset_state: BlockStateId,
        top_state: BlockStateId,
        top_survives: bool,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201IceMeltResult {
    Unchanged,
    EvaporatedToAir(BlockStateId),
    TurnedToWater(BlockStateId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201LavaInteractionResult {
    CanSpread,
    TurnedToObsidian {
        new_state: BlockStateId,
        fizz_event: i32,
    },
    TurnedToCobblestone {
        new_state: BlockStateId,
        fizz_event: i32,
    },
    TurnedToBasalt {
        new_state: BlockStateId,
        fizz_event: i32,
    },
}

/// Precomputed 1.20.1 survival and fluid/ice/cactus rule table bound to a
/// specific 24,135-state 1.20.1 `BlockRegistry`.
pub struct Legacy1201SurvivalTable<'r> {
    registry: &'r BlockRegistry,
    meta: Vec<Legacy1201StateSurvivalMeta>,
    air: BlockStateId,
    water_source: BlockStateId,
    lava_source: BlockStateId,
    cactus_default: BlockStateId,
    obsidian: BlockStateId,
    cobblestone: BlockStateId,
    stone: BlockStateId,
    basalt: BlockStateId,
    ice: BlockStateId,
    frosted_ice_ages: [BlockStateId; 4],
}

fn is_1201_sand_tag(name: &str) -> bool {
    matches!(
        name,
        "minecraft:sand" | "minecraft:red_sand" | "minecraft:suspicious_sand"
    )
}

fn is_1201_dirt_tag(name: &str) -> bool {
    matches!(
        name,
        "minecraft:dirt"
            | "minecraft:grass_block"
            | "minecraft:podzol"
            | "minecraft:coarse_dirt"
            | "minecraft:mycelium"
            | "minecraft:rooted_dirt"
            | "minecraft:moss_block"
            | "minecraft:mud"
            | "minecraft:muddy_mangrove_roots"
    )
}

fn is_1201_inherent_water_plant(name: &str) -> bool {
    matches!(
        name,
        "minecraft:bubble_column"
            | "minecraft:kelp"
            | "minecraft:kelp_plant"
            | "minecraft:seagrass"
            | "minecraft:tall_seagrass"
    )
}

fn is_1201_non_solid_or_no_motion(name: &str) -> bool {
    matches!(
        name,
        "minecraft:air"
            | "minecraft:cave_air"
            | "minecraft:void_air"
            | "minecraft:water"
            | "minecraft:lava"
            | "minecraft:fire"
            | "minecraft:soul_fire"
            | "minecraft:torch"
            | "minecraft:wall_torch"
            | "minecraft:soul_torch"
            | "minecraft:soul_wall_torch"
            | "minecraft:redstone_torch"
            | "minecraft:redstone_wall_torch"
            | "minecraft:redstone_wire"
            | "minecraft:lever"
            | "minecraft:stone_button"
            | "minecraft:oak_button"
            | "minecraft:spruce_button"
            | "minecraft:birch_button"
            | "minecraft:jungle_button"
            | "minecraft:acacia_button"
            | "minecraft:dark_oak_button"
            | "minecraft:mangrove_button"
            | "minecraft:cherry_button"
            | "minecraft:bamboo_button"
            | "minecraft:crimson_button"
            | "minecraft:warped_button"
            | "minecraft:polished_blackstone_button"
            | "minecraft:sugar_cane"
            | "minecraft:grass"
            | "minecraft:tall_grass"
            | "minecraft:fern"
            | "minecraft:large_fern"
            | "minecraft:dead_bush"
            | "minecraft:dandelion"
            | "minecraft:poppy"
            | "minecraft:vine"
            | "minecraft:rail"
            | "minecraft:powered_rail"
            | "minecraft:detector_rail"
            | "minecraft:activator_rail"
            | "minecraft:tripwire"
            | "minecraft:tripwire_hook"
            | "minecraft:cobweb"
    )
}

impl<'r> Legacy1201SurvivalTable<'r> {
    pub fn new(registry: &'r BlockRegistry) -> io::Result<Self> {
        if registry.len() != 24_135 {
            return Err(invalid(
                "1.20.1 survival table requires the 24135-state vanilla registry",
            ));
        }

        let req_default = |name: &str| -> io::Result<BlockStateId> {
            registry
                .default_state(name)
                .ok_or_else(|| invalid(format!("missing 1.20.1 block {name}")))
        };

        let air = req_default("minecraft:air")?;
        let water_source = req_default("minecraft:water")?;
        let lava_source = req_default("minecraft:lava")?;
        let cactus_default = req_default("minecraft:cactus")?;
        let obsidian = req_default("minecraft:obsidian")?;
        let cobblestone = req_default("minecraft:cobblestone")?;
        let stone = req_default("minecraft:stone")?;
        let basalt = req_default("minecraft:basalt")?;
        let ice = req_default("minecraft:ice")?;
        let frosted_default = req_default("minecraft:frosted_ice")?;

        let mut cactus_ages = [cactus_default; 16];
        let mut frosted_ice_ages = [frosted_default; 4];

        for id_usize in 0..registry.len() {
            let id = BlockStateId(id_usize as u32);
            let Some(state) = registry.state(id) else {
                continue;
            };
            if state.name == "minecraft:cactus" {
                if let Some(age_str) = state.properties.get("age")
                    && let Ok(age) = age_str.parse::<usize>()
                    && age < 16
                {
                    cactus_ages[age] = id;
                }
            } else if state.name == "minecraft:frosted_ice"
                && let Some(age_str) = state.properties.get("age")
                && let Ok(age) = age_str.parse::<usize>()
                && age < 4
            {
                frosted_ice_ages[age] = id;
            }
        }

        let mut meta = Vec::new();
        meta.try_reserve_exact(registry.len())
            .map_err(|_| invalid("out of memory allocating 1.20.1 survival table"))?;

        for id_usize in 0..registry.len() {
            let id = BlockStateId(id_usize as u32);
            let state = registry
                .state(id)
                .ok_or_else(|| invalid("missing state in 1.20.1 registry"))?;
            let name = state.name.as_str();
            let mut flags = 0u16;
            let mut flags2 = 0u8;

            let is_air = matches!(
                name,
                "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
            );
            if is_air {
                flags |= FLAG_AIR;
            }
            if name == "minecraft:cactus" {
                flags |= FLAG_CACTUS;
            }
            if is_1201_sand_tag(name) {
                flags |= FLAG_SAND_TAG;
            }
            if is_1201_dirt_tag(name) {
                flags |= FLAG_DIRT_TAG;
            }

            let waterlogged = state
                .properties
                .get("waterlogged")
                .is_some_and(|v| v == "true");
            if name == "minecraft:water" || waterlogged || is_1201_inherent_water_plant(name) {
                flags |= FLAG_WATER_FLUID;
            }
            if name == "minecraft:lava" {
                flags |= FLAG_LAVA_FLUID;
            }
            if name == "minecraft:water"
                || name == "minecraft:lava"
                || waterlogged
                || is_1201_inherent_water_plant(name)
            {
                flags |= FLAG_LIQUID_BLOCK;
            }

            let layers = state
                .properties
                .get("layers")
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(0);
            let level = state
                .properties
                .get("level")
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(0);
            let age = state
                .properties
                .get("age")
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(0);

            if name == "minecraft:snow" {
                flags |= FLAG_SNOW_LAYER;
                if layers == 8 {
                    flags |= FLAG_SOLID | FLAG_BLOCKS_MOTION;
                    flags2 |= FLAG2_FULL_UP_SUPPORT;
                } else if layers > 1 {
                    flags |= FLAG_SOLID | FLAG_BLOCKS_MOTION;
                }
            } else if !is_1201_non_solid_or_no_motion(name) {
                flags |= FLAG_SOLID | FLAG_BLOCKS_MOTION;
                let is_bottom_slab = name.ends_with("_slab")
                    && state.properties.get("type").is_some_and(|v| v == "bottom");
                let is_bottom_stair = name.ends_with("_stairs")
                    && state.properties.get("half").is_some_and(|v| v == "bottom");
                let is_trapdoor_open_or_bottom = name.ends_with("_trapdoor")
                    && (state.properties.get("open").is_some_and(|v| v == "true")
                        || state.properties.get("half").is_some_and(|v| v == "bottom"));
                let is_narrow = name.ends_with("_fence")
                    || name.ends_with("_fence_gate")
                    || name.ends_with("_wall")
                    || name.ends_with("_pane")
                    || name == "minecraft:iron_bars"
                    || name == "minecraft:cactus"
                    || name == "minecraft:farmland"
                    || name == "minecraft:dirt_path";
                if !is_bottom_slab
                    && !is_bottom_stair
                    && !is_trapdoor_open_or_bottom
                    && !is_narrow
                {
                    flags2 |= FLAG2_FULL_UP_SUPPORT;
                }
            }

            if name == "minecraft:ice" {
                flags |= FLAG_ICE;
            }
            if name == "minecraft:frosted_ice" {
                flags |= FLAG_FROSTED_ICE;
            }
            if name == "minecraft:blue_ice" {
                flags |= FLAG_BLUE_ICE;
            }
            if name == "minecraft:soul_soil" {
                flags |= FLAG_SOUL_SOIL;
            }
            if name == "minecraft:sugar_cane" {
                flags |= FLAG_SUGAR_CANE;
            }
            if matches!(
                name,
                "minecraft:ice" | "minecraft:packed_ice" | "minecraft:barrier"
            ) {
                flags |= FLAG_CANNOT_SUPPORT_SNOW;
            }
            if matches!(
                name,
                "minecraft:honey_block" | "minecraft:soul_sand" | "minecraft:mud"
            ) {
                flags2 |= FLAG2_SNOW_SUPPORT_OVERRIDE;
            }

            let light_block = if is_air {
                0
            } else if matches!(
                name,
                "minecraft:ice" | "minecraft:frosted_ice" | "minecraft:water"
            ) {
                1
            } else {
                15
            };

            let next_age_state = if name == "minecraft:cactus" {
                if age < 15 {
                    cactus_ages[(age + 1) as usize]
                } else {
                    cactus_ages[0]
                }
            } else if name == "minecraft:frosted_ice" {
                frosted_ice_ages[(age.min(2) + 1) as usize]
            } else {
                id
            };

            meta.push(Legacy1201StateSurvivalMeta {
                flags,
                flags2,
                age,
                level_or_layers: if name == "minecraft:snow" {
                    layers
                } else {
                    level
                },
                light_block,
                next_age_state,
            });
        }

        Ok(Self {
            registry,
            meta,
            air,
            water_source,
            lava_source,
            cactus_default,
            obsidian,
            cobblestone,
            stone,
            basalt,
            ice,
            frosted_ice_ages,
        })
    }

    #[inline]
    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }

    #[inline]
    pub fn air(&self) -> BlockStateId {
        self.air
    }

    #[inline]
    pub fn water_source(&self) -> BlockStateId {
        self.water_source
    }

    #[inline]
    pub fn lava_source(&self) -> BlockStateId {
        self.lava_source
    }

    #[inline]
    pub fn cactus_default(&self) -> BlockStateId {
        self.cactus_default
    }

    #[inline]
    pub fn obsidian(&self) -> BlockStateId {
        self.obsidian
    }

    #[inline]
    pub fn cobblestone(&self) -> BlockStateId {
        self.cobblestone
    }

    #[inline]
    pub fn stone(&self) -> BlockStateId {
        self.stone
    }

    #[inline]
    pub fn basalt(&self) -> BlockStateId {
        self.basalt
    }

    #[inline]
    pub fn ice(&self) -> BlockStateId {
        self.ice
    }

    #[inline]
    pub fn frosted_ice(&self, age: u8) -> BlockStateId {
        self.frosted_ice_ages[age.min(3) as usize]
    }

    #[inline]
    pub fn state_meta(&self, state: BlockStateId) -> io::Result<Legacy1201StateSurvivalMeta> {
        self.meta
            .get(state.0 as usize)
            .copied()
            .ok_or_else(|| invalid("1.20.1 state ID out of bounds"))
    }

    pub fn verify_level<L: Legacy1201Level + ?Sized>(&self, level: &L) -> io::Result<()> {
        if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION {
            return Err(invalid("1.20.1 survival level DataVersion mismatch"));
        }
        if !ptr::eq(level.collision_block_registry(), self.registry) {
            return Err(invalid(
                "1.20.1 survival level uses a different BlockRegistry instance",
            ));
        }
        Ok(())
    }

    /// 1.20.1 `CactusBlock.canSurvive(BlockState, LevelReader, BlockPos)` (`cpx.a`).
    pub fn can_cactus_survive<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
    ) -> io::Result<bool> {
        self.verify_level(level)?;
        for dir in LEGACY1201_HORIZONTAL_DIRECTIONS {
            let [dx, dy, dz] = dir.normal();
            let neighbor = level.block_state(pos.offset(dx, dy, dz));
            let m = self.state_meta(neighbor)?;
            if (m.flags & (FLAG_SOLID | FLAG_LAVA_FLUID)) != 0 {
                return Ok(false);
            }
        }
        let below = level.block_state(pos.offset(0, -1, 0));
        let below_m = self.state_meta(below)?;
        if (below_m.flags & (FLAG_CACTUS | FLAG_SAND_TAG)) == 0 {
            return Ok(false);
        }
        let above = level.block_state(pos.offset(0, 1, 0));
        let above_m = self.state_meta(above)?;
        Ok((above_m.flags & FLAG_LIQUID_BLOCK) == 0)
    }

    /// 1.20.1 `CactusBlock.tick(BlockState, ServerLevel, BlockPos, RandomSource)` (`cpx.a`).
    pub fn cactus_scheduled_tick<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
    ) -> io::Result<Legacy1201CactusTickResult> {
        if !self.can_cactus_survive(level, pos)? {
            level
                .set_raw(pos, self.air)
                .ok_or_else(|| invalid("failed to destroy unsupported 1.20.1 cactus"))?;
            Ok(Legacy1201CactusTickResult::DestroyedToAir(self.air))
        } else {
            Ok(Legacy1201CactusTickResult::Survived)
        }
    }

    /// 1.20.1 `CactusBlock.randomTick(BlockState, ServerLevel, BlockPos, RandomSource)` (`cpx.b`).
    pub fn cactus_random_tick<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
    ) -> io::Result<Legacy1201CactusRandomTickResult> {
        self.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.state_meta(state)?;
        if (m.flags & FLAG_CACTUS) == 0 {
            return Err(invalid("cactus_random_tick called on non-cactus state"));
        }
        let above_pos = pos.offset(0, 1, 0);
        let above_m = self.state_meta(level.block_state(above_pos))?;
        if (above_m.flags & FLAG_AIR) == 0 {
            return Ok(Legacy1201CactusRandomTickResult::BlockedAbove);
        }
        let mut height = 1usize;
        while (self
            .state_meta(level.block_state(pos.offset(0, -(height as i32), 0)))?
            .flags
            & FLAG_CACTUS)
            != 0
        {
            height += 1;
        }
        if height >= 3 {
            return Ok(Legacy1201CactusRandomTickResult::MaxHeightReached(height));
        }
        if m.age == 15 {
            level
                .set_raw(above_pos, self.cactus_default)
                .ok_or_else(|| invalid("failed to place grown cactus"))?;
            let reset = m.next_age_state;
            level
                .set_raw(pos, reset)
                .ok_or_else(|| invalid("failed to reset cactus age"))?;
            let top_survives = self.can_cactus_survive(level, above_pos)?;
            Ok(Legacy1201CactusRandomTickResult::Grew {
                base_reset_state: reset,
                top_state: self.cactus_default,
                top_survives,
            })
        } else {
            let next = m.next_age_state;
            level
                .set_raw(pos, next)
                .ok_or_else(|| invalid("failed to increment cactus age"))?;
            Ok(Legacy1201CactusRandomTickResult::Aged {
                new_state: next,
                new_age: m.age + 1,
            })
        }
    }

    /// 1.20.1 `IceBlock.melt(BlockState, Level, BlockPos)` (`cti.d`).
    pub fn ice_melt<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
        ultra_warm: bool,
    ) -> io::Result<Legacy1201IceMeltResult> {
        self.verify_level(level)?;
        if ultra_warm {
            level
                .set_raw(pos, self.air)
                .ok_or_else(|| invalid("failed to evaporate ice in ultraWarm dimension"))?;
            Ok(Legacy1201IceMeltResult::EvaporatedToAir(self.air))
        } else {
            level
                .set_raw(pos, self.water_source)
                .ok_or_else(|| invalid("failed to melt ice into water"))?;
            Ok(Legacy1201IceMeltResult::TurnedToWater(self.water_source))
        }
    }

    /// 1.20.1 `IceBlock.randomTick(BlockState, ServerLevel, BlockPos, RandomSource)` (`cti.b`):
    /// melts when `block_light > 11 - state.getLightBlock(level, pos)`.
    pub fn ice_random_tick<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
        ultra_warm: bool,
    ) -> io::Result<Legacy1201IceMeltResult> {
        self.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.state_meta(state)?;
        if (m.flags & (FLAG_ICE | FLAG_FROSTED_ICE)) == 0 {
            return Err(invalid("ice_random_tick called on non-ice state"));
        }
        let threshold = 11 - i32::from(m.light_block);
        if level.block_light(pos) > threshold {
            self.ice_melt(level, pos, ultra_warm)
        } else {
            Ok(Legacy1201IceMeltResult::Unchanged)
        }
    }

    /// 1.20.1 `IceBlock.playerDestroy` without Silk Touch (`cti.a`):
    /// turns into `water[level=0]` if `!ultra_warm && (below.blocksMotion() || below.liquid())`,
    /// otherwise `air`.
    pub fn ice_player_destroy_without_silk_touch<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
        ultra_warm: bool,
    ) -> io::Result<Legacy1201IceMeltResult> {
        self.verify_level(level)?;
        if ultra_warm {
            level
                .set_raw(pos, self.air)
                .ok_or_else(|| invalid("failed to remove broken ice in ultraWarm"))?;
            return Ok(Legacy1201IceMeltResult::EvaporatedToAir(self.air));
        }
        let below = level.block_state(pos.offset(0, -1, 0));
        let below_m = self.state_meta(below)?;
        if (below_m.flags & (FLAG_BLOCKS_MOTION | FLAG_LIQUID_BLOCK)) != 0 {
            level
                .set_raw(pos, self.water_source)
                .ok_or_else(|| invalid("failed to convert broken ice to water"))?;
            Ok(Legacy1201IceMeltResult::TurnedToWater(self.water_source))
        } else {
            level
                .set_raw(pos, self.air)
                .ok_or_else(|| invalid("failed to convert broken ice to air"))?;
            Ok(Legacy1201IceMeltResult::EvaporatedToAir(self.air))
        }
    }

    /// 1.20.1 `FrostedIceBlock.fewerNeigboursThan(BlockGetter, BlockPos, int)` (`cso.a`).
    pub fn frosted_ice_fewer_neighbors_than<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
        limit: usize,
    ) -> io::Result<bool> {
        self.verify_level(level)?;
        let mut count = 0usize;
        for dir in Direction::ALL {
            let [dx, dy, dz] = dir.normal();
            let neighbor = level.block_state(pos.offset(dx, dy, dz));
            if (self.state_meta(neighbor)?.flags & FLAG_FROSTED_ICE) != 0 {
                count += 1;
                if count >= limit {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// 1.20.1 `FrostedIceBlock.slightlyMelt(BlockState, Level, BlockPos)` (`cso.e`).
    /// Returns `Ok(true)` if the block fully melted (was at `age == 3`), or `Ok(false)` if its
    /// age was incremented (`0 -> 1`, `1 -> 2`, `2 -> 3`).
    pub fn frosted_ice_slightly_melt<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
        ultra_warm: bool,
    ) -> io::Result<bool> {
        self.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.state_meta(state)?;
        if (m.flags & FLAG_FROSTED_ICE) == 0 {
            return Err(invalid("frosted_ice_slightly_melt called on non-frosted-ice"));
        }
        if m.age < 3 {
            level
                .set_raw(pos, m.next_age_state)
                .ok_or_else(|| invalid("failed to age frosted_ice"))?;
            Ok(false)
        } else {
            self.ice_melt(level, pos, ultra_warm)?;
            Ok(true)
        }
    }

    /// 1.20.1 `LiquidBlock.shouldSpreadLiquid(Level, BlockPos, BlockState)` (`cua.a`).
    pub fn lava_should_spread_liquid<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
    ) -> io::Result<Legacy1201LavaInteractionResult> {
        self.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.state_meta(state)?;
        if (m.flags & FLAG_LAVA_FLUID) == 0 {
            return Ok(Legacy1201LavaInteractionResult::CanSpread);
        }
        let below = level.block_state(pos.offset(0, -1, 0));
        let is_over_soul_soil = (self.state_meta(below)?.flags & FLAG_SOUL_SOIL) != 0;

        for flow_dir in LEGACY1201_LIQUID_FLOW_DIRECTIONS {
            let probe_dir = flow_dir.opposite();
            let [dx, dy, dz] = probe_dir.normal();
            let neighbor_pos = pos.offset(dx, dy, dz);
            let neighbor = level.block_state(neighbor_pos);
            let nm = self.state_meta(neighbor)?;
            if (nm.flags & FLAG_WATER_FLUID) != 0 {
                if m.level_or_layers == 0 {
                    level
                        .set_raw(pos, self.obsidian)
                        .ok_or_else(|| invalid("failed to solidify lava into obsidian"))?;
                    return Ok(Legacy1201LavaInteractionResult::TurnedToObsidian {
                        new_state: self.obsidian,
                        fizz_event: LEGACY1201_LAVA_FIZZ_EVENT,
                    });
                } else {
                    level
                        .set_raw(pos, self.cobblestone)
                        .ok_or_else(|| invalid("failed to solidify lava into cobblestone"))?;
                    return Ok(Legacy1201LavaInteractionResult::TurnedToCobblestone {
                        new_state: self.cobblestone,
                        fizz_event: LEGACY1201_LAVA_FIZZ_EVENT,
                    });
                }
            }
            if is_over_soul_soil && (nm.flags & FLAG_BLUE_ICE) != 0 {
                level
                    .set_raw(pos, self.basalt)
                    .ok_or_else(|| invalid("failed to solidify lava into basalt"))?;
                return Ok(Legacy1201LavaInteractionResult::TurnedToBasalt {
                    new_state: self.basalt,
                    fizz_event: LEGACY1201_LAVA_FIZZ_EVENT,
                });
            }
        }
        Ok(Legacy1201LavaInteractionResult::CanSpread)
    }

    /// 1.20.1 `LavaFluid.spreadTo` downward into water -> `minecraft:stone` + fizz `1501`.
    pub fn lava_spread_down_into_water<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        target_pos: BlockPos,
    ) -> io::Result<Option<(BlockStateId, i32)>> {
        self.verify_level(level)?;
        let target = level.block_state(target_pos);
        let tm = self.state_meta(target)?;
        if (tm.flags & FLAG_WATER_FLUID) != 0 {
            level
                .set_raw(target_pos, self.stone)
                .ok_or_else(|| invalid("failed to turn water into stone under lava"))?;
            Ok(Some((self.stone, LEGACY1201_LAVA_FIZZ_EVENT)))
        } else {
            Ok(None)
        }
    }

    /// 1.20.1 `SnowLayerBlock.canSurvive` (`cww.a`), `SugarCaneBlock.canSurvive` (`cxo.a`),
    /// and `CactusBlock.canSurvive` (`cpx.a`).
    pub fn can_state_survive<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        state: BlockStateId,
        pos: BlockPos,
    ) -> io::Result<bool> {
        self.verify_level(level)?;
        let m = self.state_meta(state)?;
        if (m.flags & FLAG_CACTUS) != 0 {
            return self.can_cactus_survive(level, pos);
        }
        if (m.flags & FLAG_SNOW_LAYER) != 0 {
            let below = level.block_state(pos.offset(0, -1, 0));
            let bm = self.state_meta(below)?;
            if (bm.flags & FLAG_CANNOT_SUPPORT_SNOW) != 0 {
                return Ok(false);
            }
            if (bm.flags2 & FLAG2_SNOW_SUPPORT_OVERRIDE) != 0 {
                return Ok(true);
            }
            if (bm.flags & FLAG_SNOW_LAYER) != 0 && bm.level_or_layers == 8 {
                return Ok(true);
            }
            return Ok((bm.flags2 & FLAG2_FULL_UP_SUPPORT) != 0);
        }
        if (m.flags & FLAG_SUGAR_CANE) != 0 {
            let below_pos = pos.offset(0, -1, 0);
            let below = level.block_state(below_pos);
            let bm = self.state_meta(below)?;
            if (bm.flags & FLAG_SUGAR_CANE) != 0 {
                return Ok(true);
            }
            if (bm.flags & (FLAG_DIRT_TAG | FLAG_SAND_TAG)) != 0 {
                for dir in LEGACY1201_HORIZONTAL_DIRECTIONS {
                    let [dx, dy, dz] = dir.normal();
                    let adj = level.block_state(below_pos.offset(dx, dy, dz));
                    let am = self.state_meta(adj)?;
                    if (am.flags & (FLAG_WATER_FLUID | FLAG_FROSTED_ICE)) != 0 {
                        return Ok(true);
                    }
                }
            }
            return Ok(false);
        }
        Ok(true)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::chunk::BlockState;
    use crate::level::LevelAccess;
    use horizon_core::random::LegacyRandom;
    use std::collections::{BTreeMap, HashMap};

    struct Mock1201Level<'r> {
        registry: &'r BlockRegistry,
        air: BlockStateId,
        blocks: HashMap<BlockPos, BlockStateId>,
        light: HashMap<BlockPos, i32>,
        random: LegacyRandom,
    }

    impl<'r> Mock1201Level<'r> {
        fn new(registry: &'r BlockRegistry) -> Self {
            let air = registry.default_state("minecraft:air").unwrap();
            Self {
                registry,
                air,
                blocks: HashMap::new(),
                light: HashMap::new(),
                random: LegacyRandom::new(1201),
            }
        }
    }

    impl LevelAccess for Mock1201Level<'_> {
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
        fn block_light(&self, pos: BlockPos) -> i32 {
            self.light.get(&pos).copied().unwrap_or(0)
        }
    }

    impl Legacy1201Level for Mock1201Level<'_> {
        fn collision_data_version(&self) -> i32 {
            LEGACY_1_20_1_DATA_VERSION
        }
        fn collision_block_registry(&self) -> &BlockRegistry {
            self.registry
        }
    }

    pub(crate) fn build_synthetic_24135_registry() -> BlockRegistry {
        let mut states = Vec::with_capacity(24_135);
        let mut defaults = BTreeMap::new();

        let mut push_block = |name: &str, props_list: Vec<Vec<(&str, &str)>>| {
            let first_id = states.len() as i64;
            for props in props_list {
                let mut map = crate::block_state_properties::BlockProperties::new();
                for (k, v) in props {
                    map.insert(k.to_string(), v.to_string());
                }
                states.push(BlockState {
                    name: name.to_string(),
                    properties: map,
                });
            }
            defaults.insert(name.to_string(), first_id);
        };

        push_block("minecraft:air", vec![vec![]]);
        push_block("minecraft:cave_air", vec![vec![]]);
        push_block("minecraft:void_air", vec![vec![]]);
        push_block("minecraft:stone", vec![vec![]]);
        push_block("minecraft:cobblestone", vec![vec![]]);
        push_block("minecraft:obsidian", vec![vec![]]);
        push_block("minecraft:basalt", vec![vec![]]);
        push_block("minecraft:soul_soil", vec![vec![]]);
        push_block("minecraft:blue_ice", vec![vec![]]);
        push_block("minecraft:ice", vec![vec![]]);
        push_block("minecraft:packed_ice", vec![vec![]]);
        push_block("minecraft:sand", vec![vec![]]);
        push_block("minecraft:red_sand", vec![vec![]]);
        push_block("minecraft:suspicious_sand", vec![vec![]]);
        push_block("minecraft:dirt", vec![vec![]]);
        push_block("minecraft:grass_block", vec![vec![]]);
        push_block("minecraft:honey_block", vec![vec![]]);
        push_block("minecraft:soul_sand", vec![vec![]]);
        push_block("minecraft:mud", vec![vec![]]);
        push_block("minecraft:barrier", vec![vec![]]);
        push_block("minecraft:torch", vec![vec![]]);

        let age16: Vec<Vec<(&str, &str)>> = (0..16)
            .map(|a| vec![("age", Box::leak(a.to_string().into_boxed_str()) as &str)])
            .collect();
        push_block("minecraft:cactus", age16.clone());
        push_block("minecraft:sugar_cane", age16);

        let level16: Vec<Vec<(&str, &str)>> = (0..16)
            .map(|l| vec![("level", Box::leak(l.to_string().into_boxed_str()) as &str)])
            .collect();
        push_block("minecraft:water", level16.clone());
        push_block("minecraft:lava", level16);

        let age4: Vec<Vec<(&str, &str)>> = (0..4)
            .map(|a| vec![("age", Box::leak(a.to_string().into_boxed_str()) as &str)])
            .collect();
        push_block("minecraft:frosted_ice", age4);

        let layers8: Vec<Vec<(&str, &str)>> = (1..=8)
            .map(|l| vec![("layers", Box::leak(l.to_string().into_boxed_str()) as &str)])
            .collect();
        push_block("minecraft:snow", layers8);

        while states.len() < 24_135 {
            let idx = states.len();
            let name = format!("minecraft:filler_{idx}");
            defaults.insert(name.clone(), idx as i64);
            states.push(BlockState {
                name,
                properties: Default::default(),
            });
        }

        let root = crate::nbt::Tag::compound([
            ("DataVersion", crate::nbt::Tag::Int(crate::DATA_VERSION)),
            (
                "states",
                crate::nbt::Tag::List(states.iter().map(BlockState::to_nbt).collect()),
            ),
            (
                "defaults",
                crate::nbt::Tag::compound(
                    defaults
                        .iter()
                        .map(|(k, v)| (k.as_str(), crate::nbt::Tag::Int(*v as i32))),
                ),
            ),
        ]);
        BlockRegistry::from_nbt(&root).unwrap()
    }

    #[test]
    fn cactus_survival_and_growth_follow_1201_rules() {
        let registry = build_synthetic_24135_registry();
        let table = Legacy1201SurvivalTable::new(&registry).unwrap();
        let mut level = Mock1201Level::new(&registry);

        let base = BlockPos::new(10, 64, 10);
        let sand = registry.default_state("minecraft:sand").unwrap();
        let stone = registry.default_state("minecraft:stone").unwrap();
        let torch = registry.default_state("minecraft:torch").unwrap();

        level.set_raw(base, table.cactus_default());
        assert!(!table.can_cactus_survive(&level, base).unwrap());

        level.set_raw(base.offset(0, -1, 0), sand);
        assert!(table.can_cactus_survive(&level, base).unwrap());

        level.set_raw(base.offset(1, 0, 0), torch);
        assert!(table.can_cactus_survive(&level, base).unwrap());

        level.set_raw(base.offset(1, 0, 0), stone);
        assert!(!table.can_cactus_survive(&level, base).unwrap());
        level.set_raw(base.offset(1, 0, 0), table.air());

        level.set_raw(base.offset(0, 1, 0), table.water_source());
        assert!(!table.can_cactus_survive(&level, base).unwrap());
        level.set_raw(base.offset(0, 1, 0), table.air());

        for expected_age in 1..=15 {
            let res = table.cactus_random_tick(&mut level, base).unwrap();
            assert_eq!(
                res,
                Legacy1201CactusRandomTickResult::Aged {
                    new_state: level.block_state(base),
                    new_age: expected_age,
                }
            );
        }
        let grew = table.cactus_random_tick(&mut level, base).unwrap();
        assert_eq!(
            grew,
            Legacy1201CactusRandomTickResult::Grew {
                base_reset_state: table.cactus_default(),
                top_state: table.cactus_default(),
                top_survives: true,
            }
        );
        assert_eq!(level.block_state(base), table.cactus_default());
        assert_eq!(level.block_state(base.offset(0, 1, 0)), table.cactus_default());
    }

    #[test]
    fn ice_melting_and_lava_interactions_follow_1201_rules() {
        let registry = build_synthetic_24135_registry();
        let table = Legacy1201SurvivalTable::new(&registry).unwrap();
        let mut level = Mock1201Level::new(&registry);

        let pos = BlockPos::new(0, 70, 0);
        level.set_raw(pos, table.ice());

        level.light.insert(pos, 10);
        assert_eq!(
            table.ice_random_tick(&mut level, pos, false).unwrap(),
            Legacy1201IceMeltResult::Unchanged
        );
        level.light.insert(pos, 11);
        assert_eq!(
            table.ice_random_tick(&mut level, pos, false).unwrap(),
            Legacy1201IceMeltResult::TurnedToWater(table.water_source())
        );
        assert_eq!(level.block_state(pos), table.water_source());

        level.set_raw(pos, table.ice());
        assert_eq!(
            table.ice_random_tick(&mut level, pos, true).unwrap(),
            Legacy1201IceMeltResult::EvaporatedToAir(table.air())
        );
        assert_eq!(level.block_state(pos), table.air());

        let lava_pos = BlockPos::new(5, 64, 5);
        level.set_raw(lava_pos, table.lava_source());
        level.set_raw(lava_pos.offset(0, 0, -1), table.water_source());
        assert_eq!(
            table.lava_should_spread_liquid(&mut level, lava_pos).unwrap(),
            Legacy1201LavaInteractionResult::TurnedToObsidian {
                new_state: table.obsidian(),
                fizz_event: LEGACY1201_LAVA_FIZZ_EVENT,
            }
        );

        let basalt_pos = BlockPos::new(20, 64, 20);
        let soul_soil = registry.default_state("minecraft:soul_soil").unwrap();
        let blue_ice = registry.default_state("minecraft:blue_ice").unwrap();
        level.set_raw(basalt_pos, table.lava_source());
        level.set_raw(basalt_pos.offset(0, -1, 0), soul_soil);
        level.set_raw(basalt_pos.offset(1, 0, 0), blue_ice);
        assert_eq!(
            table.lava_should_spread_liquid(&mut level, basalt_pos).unwrap(),
            Legacy1201LavaInteractionResult::TurnedToBasalt {
                new_state: table.basalt(),
                fizz_event: LEGACY1201_LAVA_FIZZ_EVENT,
            }
        );
    }
}
