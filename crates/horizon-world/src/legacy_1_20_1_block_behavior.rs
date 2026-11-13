//! Stage 2.3 & 2.4 — Minecraft Java 1.20.1 (`DataVersion = 3465`) block behaviors:
//! doors, trapdoors, buttons, levers, slabs, stairs, fences, fence gates, walls,
//! iron bars / glass panes, water/lava fluids, and redstone wire, plus end-to-end
//! scenario execution for Java 1.20.1 vs. Rust parity verification.
//!
//! Preserves Horizon 26.3 performance optimizations:
//! - Flat preallocated lookup tables (`Vec::try_reserve_exact`) for O(1) property/class queries.
//! - Zero-allocation `java.util.HashSet<BlockPos>` bucket ordering for redstone wire updates.

use crate::{
    compression::invalid,
    fluid::FluidKind,
    legacy_1_20_1_block_survival::{LEGACY1201_HORIZONTAL_DIRECTIONS, Legacy1201SurvivalTable},
    legacy_1_20_1_neighbor_ticks::{
        Legacy1201CollectingNeighborUpdater, Legacy1201ServerTickEngine, Legacy1201TickPriority,
    },
    legacy_1_20_1_world::{LEGACY_1_20_1_DATA_VERSION, Legacy1201Level},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{geometry::Direction, position::BlockPos};
use std::io;

#[inline]
pub const fn legacy1201_clockwise(dir: Direction) -> Direction {
    match dir {
        Direction::North => Direction::East,
        Direction::East => Direction::South,
        Direction::South => Direction::West,
        Direction::West => Direction::North,
        other => other,
    }
}

#[inline]
pub const fn legacy1201_counter_clockwise(dir: Direction) -> Direction {
    match dir {
        Direction::North => Direction::West,
        Direction::West => Direction::South,
        Direction::South => Direction::East,
        Direction::East => Direction::North,
        other => other,
    }
}

#[inline]
pub const fn legacy1201_is_horizontal_axis_x(dir: Direction) -> bool {
    matches!(dir, Direction::East | Direction::West)
}

/// 1.20.1 `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection` (`csd.n`).
/// Returns the direction the attachment points *outward* from its support block
/// (`ceiling -> Down`, `floor -> Up`, `wall -> facing`). The supporting block is at
/// `pos.relative(connected_direction.opposite())`.
#[inline]
pub fn legacy1201_face_attached_connected_direction(face: &str, facing: Direction) -> Direction {
    match face {
        "ceiling" => Direction::Down,
        "floor" => Direction::Up,
        _ => facing,
    }
}

/// Order `positions` exactly as `com.google.common.collect.Sets.newHashSet()` (`java.util.HashSet`
/// with initial capacity `16`) iterates them in `RedStoneWireBlock.updatePowerStrength` (`cvo.a`).
pub fn legacy1201_java_hash_set_order(positions: &mut [BlockPos; 7]) {
    #[inline]
    fn vec3i_hash(pos: BlockPos) -> i32 {
        (pos.y.wrapping_add(pos.z.wrapping_mul(31)))
            .wrapping_mul(31)
            .wrapping_add(pos.x)
    }
    #[inline]
    fn bucket(pos: BlockPos) -> usize {
        let h = vec3i_hash(pos);
        ((h ^ ((h as u32) >> 16) as i32) & 15) as usize
    }

    // Insertion-sort 7 elements by `(bucket(pos), original_index)` in registers without heap alloc.
    let mut keys: [(usize, usize, BlockPos); 7] =
        std::array::from_fn(|i| (bucket(positions[i]), i, positions[i]));
    for i in 1..7 {
        let cur = keys[i];
        let mut j = i;
        while j > 0 && (keys[j - 1].0, keys[j - 1].1) > (cur.0, cur.1) {
            keys[j] = keys[j - 1];
            j -= 1;
        }
        keys[j] = cur;
    }
    for i in 0..7 {
        positions[i] = keys[i].2;
    }
}

/// 1.20.1 `StairShape` (`ddk.class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201StairShape {
    Straight,
    InnerLeft,
    InnerRight,
    OuterLeft,
    OuterRight,
}

impl Legacy1201StairShape {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Straight => "straight",
            Self::InnerLeft => "inner_left",
            Self::InnerRight => "inner_right",
            Self::OuterLeft => "outer_left",
            Self::OuterRight => "outer_right",
        }
    }
}

/// 1.20.1 `WallSide` (`ddn.class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201WallSide {
    None,
    Low,
    Tall,
}

impl Legacy1201WallSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Low => "low",
            Self::Tall => "tall",
        }
    }
    pub const fn is_connected(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// 1.20.1 `RedstoneSide` (`ddg.class`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201RedstoneSide {
    Up,
    Side,
    None,
}

impl Legacy1201RedstoneSide {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Up => "up",
            Self::Side => "side",
            Self::None => "none",
        }
    }
    pub const fn is_connected(self) -> bool {
        !matches!(self, Self::None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201WallConnections {
    pub up: bool,
    pub north: Legacy1201WallSide,
    pub east: Legacy1201WallSide,
    pub south: Legacy1201WallSide,
    pub west: Legacy1201WallSide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201HorizontalConnections {
    pub north: bool,
    pub east: bool,
    pub south: bool,
    pub west: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201WireConnections {
    pub north: Legacy1201RedstoneSide,
    pub east: Legacy1201RedstoneSide,
    pub south: Legacy1201RedstoneSide,
    pub west: Legacy1201RedstoneSide,
}

impl Legacy1201WireConnections {
    pub const fn is_dot(self) -> bool {
        !self.north.is_connected()
            && !self.east.is_connected()
            && !self.south.is_connected()
            && !self.west.is_connected()
    }
    pub const fn is_cross(self) -> bool {
        self.north.is_connected()
            && self.east.is_connected()
            && !matches!(self.south, Legacy1201RedstoneSide::None)
            && !matches!(self.west, Legacy1201RedstoneSide::None)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201BehaviorMeta {
    pub class_flags: u16,
    pub facing: u8,
    pub half_upper_or_top: bool,
    pub open: bool,
    pub powered: bool,
    pub waterlogged: bool,
    pub slab_double: bool,
    pub button_ticks: u8,
    pub power_or_level: u8,
    pub toggled_open: BlockStateId,
    pub toggled_powered: BlockStateId,
    pub toggled_open_and_powered: BlockStateId,
}

impl Default for Legacy1201BehaviorMeta {
    #[inline]
    fn default() -> Self {
        Self {
            class_flags: 0,
            facing: 0,
            half_upper_or_top: false,
            open: false,
            powered: false,
            waterlogged: false,
            slab_double: false,
            button_ticks: 0,
            power_or_level: 0,
            toggled_open: BlockStateId(0),
            toggled_powered: BlockStateId(0),
            toggled_open_and_powered: BlockStateId(0),
        }
    }
}

const BFLAG_DOOR: u16 = 1 << 0;
const BFLAG_IRON_DOOR_OR_TRAPDOOR: u16 = 1 << 1;
const BFLAG_TRAPDOOR: u16 = 1 << 2;
const BFLAG_BUTTON: u16 = 1 << 3;
const BFLAG_LEVER: u16 = 1 << 4;
const BFLAG_SLAB: u16 = 1 << 5;
const BFLAG_STAIR: u16 = 1 << 6;
const BFLAG_WOODEN_FENCE: u16 = 1 << 7;
const BFLAG_NETHER_FENCE: u16 = 1 << 8;
const BFLAG_FENCE_GATE: u16 = 1 << 9;
const BFLAG_WALL: u16 = 1 << 10;
const BFLAG_PANE_OR_BARS: u16 = 1 << 11;
const BFLAG_REDSTONE_WIRE: u16 = 1 << 12;
const BFLAG_SIGNAL_SOURCE: u16 = 1 << 13;
const BFLAG_CONDUCTOR: u16 = 1 << 14;
const BFLAG_FULL_FACE: u16 = 1 << 15;

fn parse_direction(s: &str) -> Direction {
    match s {
        "down" => Direction::Down,
        "up" => Direction::Up,
        "north" => Direction::North,
        "south" => Direction::South,
        "west" => Direction::West,
        "east" => Direction::East,
        _ => Direction::North,
    }
}

fn is_1201_exception_for_connection(name: &str) -> bool {
    name.ends_with("_leaves")
        || name == "minecraft:barrier"
        || name == "minecraft:carved_pumpkin"
        || name == "minecraft:jack_o_lantern"
        || name == "minecraft:melon"
        || name == "minecraft:pumpkin"
        || name.ends_with("shulker_box")
}

/// Precomputed 1.20.1 block behavior engine bound to a 24,135-state `BlockRegistry`.
pub struct Legacy1201BehaviorTable<'r> {
    pub survival: Legacy1201SurvivalTable<'r>,
    meta: Vec<Legacy1201BehaviorMeta>,
    wire_power_states: [BlockStateId; 16],
    water_levels: [BlockStateId; 16],
    lava_levels: [BlockStateId; 16],
}

impl<'r> Legacy1201BehaviorTable<'r> {
    pub fn new(registry: &'r BlockRegistry) -> io::Result<Self> {
        let survival = Legacy1201SurvivalTable::new(registry)?;
        let mut meta = Vec::new();
        meta.try_reserve_exact(registry.len())
            .map_err(|_| invalid("out of memory allocating 1.20.1 behavior table"))?;

        let wire_default = registry
            .default_state("minecraft:redstone_wire")
            .unwrap_or(survival.air());
        let mut wire_power_states = [wire_default; 16];
        let mut water_levels = [survival.water_source(); 16];
        let mut lava_levels = [survival.lava_source(); 16];

        for id_usize in 0..registry.len() {
            let id = BlockStateId(id_usize as u32);
            let s = registry.state(id).unwrap();
            let name = s.name.as_str();

            if name == "minecraft:redstone_wire" {
                if let Some(p) = s.properties.get("power").and_then(|v| v.parse::<usize>().ok())
                    && p < 16
                    && s.properties.get("north").map_or(true, |v| v == "none")
                    && s.properties.get("east").map_or(true, |v| v == "none")
                    && s.properties.get("south").map_or(true, |v| v == "none")
                    && s.properties.get("west").map_or(true, |v| v == "none")
                {
                    wire_power_states[p] = id;
                }
            } else if name == "minecraft:water" {
                if let Some(l) = s.properties.get("level").and_then(|v| v.parse::<usize>().ok())
                    && l < 16
                {
                    water_levels[l] = id;
                }
            } else if name == "minecraft:lava"
                && let Some(l) = s.properties.get("level").and_then(|v| v.parse::<usize>().ok())
                && l < 16
            {
                lava_levels[l] = id;
            }

            let mut class_flags = 0u16;
            if name.ends_with("_door") {
                class_flags |= BFLAG_DOOR;
                if name == "minecraft:iron_door" {
                    class_flags |= BFLAG_IRON_DOOR_OR_TRAPDOOR;
                }
            }
            if name.ends_with("_trapdoor") {
                class_flags |= BFLAG_TRAPDOOR;
                if name == "minecraft:iron_trapdoor" {
                    class_flags |= BFLAG_IRON_DOOR_OR_TRAPDOOR;
                }
            }
            if name.ends_with("_button") {
                class_flags |= BFLAG_BUTTON | BFLAG_SIGNAL_SOURCE;
            }
            if name == "minecraft:lever" {
                class_flags |= BFLAG_LEVER | BFLAG_SIGNAL_SOURCE;
            }
            if name.ends_with("_slab") {
                class_flags |= BFLAG_SLAB;
            }
            if name.ends_with("_stairs") {
                class_flags |= BFLAG_STAIR;
            }
            if name.ends_with("_fence") {
                if name == "minecraft:nether_brick_fence" {
                    class_flags |= BFLAG_NETHER_FENCE;
                } else {
                    class_flags |= BFLAG_WOODEN_FENCE;
                }
            }
            if name.ends_with("_fence_gate") {
                class_flags |= BFLAG_FENCE_GATE;
            }
            if name.ends_with("_wall") {
                class_flags |= BFLAG_WALL;
            }
            if name == "minecraft:iron_bars" || name.ends_with("glass_pane") {
                class_flags |= BFLAG_PANE_OR_BARS;
            }
            if name == "minecraft:redstone_wire" {
                class_flags |= BFLAG_REDSTONE_WIRE | BFLAG_SIGNAL_SOURCE;
            }
            if matches!(
                name,
                "minecraft:redstone_block"
                    | "minecraft:redstone_torch"
                    | "minecraft:redstone_wall_torch"
                    | "minecraft:repeater"
                    | "minecraft:comparator"
                    | "minecraft:observer"
                    | "minecraft:daylight_detector"
                    | "minecraft:target"
            ) {
                class_flags |= BFLAG_SIGNAL_SOURCE;
            }

            let open = s.properties.get("open").is_some_and(|v| v == "true");
            let powered = s.properties.get("powered").is_some_and(|v| v == "true");
            let waterlogged = s.properties.get("waterlogged").is_some_and(|v| v == "true");
            let slab_double = s.properties.get("type").is_some_and(|v| v == "double");
            let half_upper_or_top = s
                .properties
                .get("half")
                .is_some_and(|v| v == "upper" || v == "top")
                || s.properties.get("type").is_some_and(|v| v == "top");
            let facing = s
                .properties
                .get("facing")
                .map(|v| parse_direction(v) as u8)
                .unwrap_or(Direction::North as u8);
            let button_ticks = if (class_flags & BFLAG_BUTTON) != 0 {
                if matches!(
                    name,
                    "minecraft:stone_button" | "minecraft:polished_blackstone_button"
                ) {
                    20
                } else {
                    30
                }
            } else {
                0
            };

            let power_or_level = s
                .properties
                .get("power")
                .or_else(|| s.properties.get("level"))
                .and_then(|v| v.parse::<u8>().ok())
                .unwrap_or(0);

            let is_full_cube = !is_1201_exception_for_connection(name)
                && (class_flags
                    & (BFLAG_DOOR
                        | BFLAG_TRAPDOOR
                        | BFLAG_BUTTON
                        | BFLAG_LEVER
                        | BFLAG_STAIR
                        | BFLAG_WOODEN_FENCE
                        | BFLAG_NETHER_FENCE
                        | BFLAG_FENCE_GATE
                        | BFLAG_WALL
                        | BFLAG_PANE_OR_BARS
                        | BFLAG_REDSTONE_WIRE))
                    == 0
                && ((class_flags & BFLAG_SLAB) == 0 || slab_double)
                && !matches!(
                    name,
                    "minecraft:air"
                        | "minecraft:cave_air"
                        | "minecraft:void_air"
                        | "minecraft:water"
                        | "minecraft:lava"
                        | "minecraft:cactus"
                        | "minecraft:sugar_cane"
                        | "minecraft:torch"
                        | "minecraft:glass"
                        | "minecraft:ice"
                        | "minecraft:frosted_ice"
                        | "minecraft:snow"
                );
            if is_full_cube {
                class_flags |= BFLAG_CONDUCTOR | BFLAG_FULL_FACE;
            }

            let toggled_open = if s.properties.contains_key("open") {
                let mut next = s.clone();
                next.properties.insert(
                    "open".to_string(),
                    if open { "false" } else { "true" }.to_string(),
                );
                registry.id(&next).unwrap_or(id)
            } else {
                id
            };

            let toggled_powered = if s.properties.contains_key("powered") {
                let mut next = s.clone();
                next.properties.insert(
                    "powered".to_string(),
                    if powered { "false" } else { "true" }.to_string(),
                );
                registry.id(&next).unwrap_or(id)
            } else {
                id
            };

            let toggled_open_and_powered =
                if s.properties.contains_key("open") && s.properties.contains_key("powered") {
                    let mut next = s.clone();
                    next.properties.insert(
                        "open".to_string(),
                        if open { "false" } else { "true" }.to_string(),
                    );
                    next.properties.insert(
                        "powered".to_string(),
                        if powered { "false" } else { "true" }.to_string(),
                    );
                    registry.id(&next).unwrap_or(id)
                } else {
                    id
                };

            meta.push(Legacy1201BehaviorMeta {
                class_flags,
                facing,
                half_upper_or_top,
                open,
                powered,
                waterlogged,
                slab_double,
                button_ticks,
                power_or_level,
                toggled_open,
                toggled_powered,
                toggled_open_and_powered,
            });
        }

        Ok(Self {
            survival,
            meta,
            wire_power_states,
            water_levels,
            lava_levels,
        })
    }

    #[inline]
    pub fn behavior_meta(&self, state: BlockStateId) -> io::Result<Legacy1201BehaviorMeta> {
        self.meta
            .get(state.0 as usize)
            .copied()
            .ok_or_else(|| invalid("1.20.1 state ID out of bounds"))
    }

    #[inline]
    pub fn wire_state_with_power(&self, power: u8) -> BlockStateId {
        self.wire_power_states[power.min(15) as usize]
    }

    #[inline]
    pub fn water_state_with_level(&self, level: u8) -> BlockStateId {
        self.water_levels[level.min(15) as usize]
    }

    #[inline]
    pub fn lava_state_with_level(&self, level: u8) -> BlockStateId {
        self.lava_levels[level.min(15) as usize]
    }

    /// 1.20.1 `DoorBlock.use` (`crp.a`): wooden doors toggle `open` on both halves;
    /// iron doors (`!canOpenByHand`) return `Ok(false)`.
    pub fn use_door<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
    ) -> io::Result<bool> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_DOOR) == 0 {
            return Err(invalid("use_door called on non-door block"));
        }
        if (m.class_flags & BFLAG_IRON_DOOR_OR_TRAPDOOR) != 0 {
            return Ok(false);
        }
        level
            .set_raw(pos, m.toggled_open)
            .ok_or_else(|| invalid("failed to toggle door state"))?;
        let other_pos = if m.half_upper_or_top {
            pos.offset(0, -1, 0)
        } else {
            pos.offset(0, 1, 0)
        };
        let other_state = level.block_state(other_pos);
        let om = self.behavior_meta(other_state)?;
        if (om.class_flags & BFLAG_DOOR) != 0 && om.open == m.open {
            level.set_raw(other_pos, om.toggled_open);
        }
        Ok(true)
    }

    /// 1.20.1 `DoorBlock.neighborChanged` (`crp.a`): redstone signal opens/closes both halves
    /// (including iron doors!).
    pub fn door_neighbor_changed<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
        has_signal: bool,
    ) -> io::Result<bool> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_DOOR) == 0 {
            return Err(invalid("door_neighbor_changed called on non-door block"));
        }
        if m.powered == has_signal {
            return Ok(false);
        }
        let next = if m.open != has_signal {
            m.toggled_open_and_powered
        } else {
            m.toggled_powered
        };
        level.set_raw(pos, next);

        let other_pos = if m.half_upper_or_top {
            pos.offset(0, -1, 0)
        } else {
            pos.offset(0, 1, 0)
        };
        let other_state = level.block_state(other_pos);
        let om = self.behavior_meta(other_state)?;
        if (om.class_flags & BFLAG_DOOR) != 0 && om.powered != has_signal {
            let other_next = if om.open != has_signal {
                om.toggled_open_and_powered
            } else {
                om.toggled_powered
            };
            level.set_raw(other_pos, other_next);
        }
        Ok(true)
    }

    /// 1.20.1 `TrapDoorBlock.use` (`cya.a`) and `neighborChanged`.
    pub fn use_trapdoor<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Legacy1201ServerTickEngine,
        game_time: i64,
        pos: BlockPos,
    ) -> io::Result<bool> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_TRAPDOOR) == 0 {
            return Err(invalid("use_trapdoor called on non-trapdoor"));
        }
        if (m.class_flags & BFLAG_IRON_DOOR_OR_TRAPDOOR) != 0 {
            return Ok(false);
        }
        level.set_raw(pos, m.toggled_open);
        if m.waterlogged {
            ticks.schedule_fluid_tick(game_time, pos, FluidKind::Water, 5);
        }
        Ok(true)
    }

    pub fn trapdoor_neighbor_changed<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        ticks: &mut Legacy1201ServerTickEngine,
        game_time: i64,
        pos: BlockPos,
        has_signal: bool,
    ) -> io::Result<bool> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_TRAPDOOR) == 0 {
            return Err(invalid("trapdoor_neighbor_changed called on non-trapdoor"));
        }
        if m.powered == has_signal {
            return Ok(false);
        }
        let next = if m.open != has_signal {
            m.toggled_open_and_powered
        } else {
            m.toggled_powered
        };
        level.set_raw(pos, next);
        if m.waterlogged {
            ticks.schedule_fluid_tick(game_time, pos, FluidKind::Water, 5);
        }
        Ok(true)
    }

    /// 1.20.1 `ButtonBlock.press` (`cpw.d`) and `ButtonBlock.tick` (`cpw.a`).
    pub fn press_button<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        updater: &mut Legacy1201CollectingNeighborUpdater,
        ticks: &mut Legacy1201ServerTickEngine,
        game_time: i64,
        pos: BlockPos,
    ) -> io::Result<bool> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_BUTTON) == 0 {
            return Err(invalid("press_button called on non-button"));
        }
        if m.powered {
            return Ok(false);
        }
        let next = m.toggled_powered;
        level.set_raw(pos, next);
        let connected = self.connected_direction_of(state);
        let [dx, dy, dz] = connected.opposite().normal();
        let attached_pos = pos.offset(dx, dy, dz);
        updater.update_neighbors_at_except_from_facing(pos, state, None);
        updater.update_neighbors_at_except_from_facing(attached_pos, state, None);
        ticks.schedule_block_tick(
            game_time,
            pos,
            next,
            i32::from(m.button_ticks),
            Legacy1201TickPriority::Normal,
        );
        Ok(true)
    }

    pub fn button_scheduled_tick<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        updater: &mut Legacy1201CollectingNeighborUpdater,
        pos: BlockPos,
    ) -> io::Result<bool> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_BUTTON) == 0 || !m.powered {
            return Ok(false);
        }
        level.set_raw(pos, m.toggled_powered);
        let connected = self.connected_direction_of(state);
        let [dx, dy, dz] = connected.opposite().normal();
        let attached_pos = pos.offset(dx, dy, dz);
        updater.update_neighbors_at_except_from_facing(pos, state, None);
        updater.update_neighbors_at_except_from_facing(attached_pos, state, None);
        Ok(true)
    }

    /// 1.20.1 `LeverBlock.pull` (`ctx.d`).
    pub fn pull_lever<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        updater: &mut Legacy1201CollectingNeighborUpdater,
        pos: BlockPos,
    ) -> io::Result<bool> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_LEVER) == 0 {
            return Err(invalid("pull_lever called on non-lever"));
        }
        let next = m.toggled_powered;
        level.set_raw(pos, next);
        let connected = self.connected_direction_of(state);
        let [dx, dy, dz] = connected.opposite().normal();
        let attached_pos = pos.offset(dx, dy, dz);
        updater.update_neighbors_at_except_from_facing(pos, state, None);
        updater.update_neighbors_at_except_from_facing(attached_pos, state, None);
        Ok(!m.powered)
    }

    fn connected_direction_of(&self, state: BlockStateId) -> Direction {
        let Some(def) = self.survival.registry().state(state) else {
            return Direction::Up;
        };
        let face = def.properties.get("face").map_or("wall", |v| v.as_str());
        let facing = def
            .properties
            .get("facing")
            .map_or(Direction::North, |v| parse_direction(v));
        legacy1201_face_attached_connected_direction(face, facing)
    }

    /// 1.20.1 `StairBlock.getStairsShape` (`cxh.i`).
    pub fn compute_stair_shape<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
        facing: Direction,
        half_top: bool,
    ) -> io::Result<Legacy1201StairShape> {
        self.survival.verify_level(level)?;
        let [fx, fy, fz] = facing.normal();
        let front_state = level.block_state(pos.offset(fx, fy, fz));
        let fm = self.behavior_meta(front_state)?;
        if (fm.class_flags & BFLAG_STAIR) != 0 && fm.half_upper_or_top == half_top {
            let front_facing = Direction::ALL[fm.facing as usize];
            if legacy1201_is_horizontal_axis_x(front_facing) != legacy1201_is_horizontal_axis_x(facing)
                && self.stair_can_take_shape(level, pos, facing, half_top, front_facing.opposite())?
            {
                if front_facing == legacy1201_counter_clockwise(facing) {
                    return Ok(Legacy1201StairShape::OuterLeft);
                }
                return Ok(Legacy1201StairShape::OuterRight);
            }
        }

        let [bx, by, bz] = facing.opposite().normal();
        let back_state = level.block_state(pos.offset(bx, by, bz));
        let bm = self.behavior_meta(back_state)?;
        if (bm.class_flags & BFLAG_STAIR) != 0 && bm.half_upper_or_top == half_top {
            let back_facing = Direction::ALL[bm.facing as usize];
            if legacy1201_is_horizontal_axis_x(back_facing) != legacy1201_is_horizontal_axis_x(facing)
                && self.stair_can_take_shape(level, pos, facing, half_top, back_facing)?
            {
                if back_facing == legacy1201_counter_clockwise(facing) {
                    return Ok(Legacy1201StairShape::InnerLeft);
                }
                return Ok(Legacy1201StairShape::InnerRight);
            }
        }
        Ok(Legacy1201StairShape::Straight)
    }

    fn stair_can_take_shape<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
        facing: Direction,
        half_top: bool,
        check_dir: Direction,
    ) -> io::Result<bool> {
        let [dx, dy, dz] = check_dir.normal();
        let neighbor = level.block_state(pos.offset(dx, dy, dz));
        let nm = self.behavior_meta(neighbor)?;
        if (nm.class_flags & BFLAG_STAIR) == 0 {
            return Ok(true);
        }
        let neighbor_facing = Direction::ALL[nm.facing as usize];
        Ok(neighbor_facing != facing || nm.half_upper_or_top != half_top)
    }

    /// 1.20.1 `FenceBlock.connectsTo` (`csh.a`).
    pub fn compute_fence_connections<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
        is_wooden_fence: bool,
    ) -> io::Result<Legacy1201HorizontalConnections> {
        self.survival.verify_level(level)?;
        let mut conn = [false; 4];
        for (i, dir) in LEGACY1201_HORIZONTAL_DIRECTIONS.into_iter().enumerate() {
            let [dx, dy, dz] = dir.normal();
            let nstate = level.block_state(pos.offset(dx, dy, dz));
            let nm = self.behavior_meta(nstate)?;
            let same_fence = if is_wooden_fence {
                (nm.class_flags & BFLAG_WOODEN_FENCE) != 0
            } else {
                (nm.class_flags & BFLAG_NETHER_FENCE) != 0
            };
            let gate_connects = if (nm.class_flags & BFLAG_FENCE_GATE) != 0 {
                let gate_facing = Direction::ALL[nm.facing as usize];
                legacy1201_is_horizontal_axis_x(gate_facing)
                    == legacy1201_is_horizontal_axis_x(legacy1201_clockwise(dir))
            } else {
                false
            };
            conn[i] = same_fence || gate_connects || (nm.class_flags & BFLAG_FULL_FACE) != 0;
        }
        Ok(Legacy1201HorizontalConnections {
            north: conn[0],
            east: conn[1],
            south: conn[2],
            west: conn[3],
        })
    }

    /// 1.20.1 `IronBarsBlock.attachsTo` (`ctl.a`): iron bars and glass panes.
    pub fn compute_pane_connections<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
    ) -> io::Result<Legacy1201HorizontalConnections> {
        self.survival.verify_level(level)?;
        let mut conn = [false; 4];
        for (i, dir) in LEGACY1201_HORIZONTAL_DIRECTIONS.into_iter().enumerate() {
            let [dx, dy, dz] = dir.normal();
            let nstate = level.block_state(pos.offset(dx, dy, dz));
            let nm = self.behavior_meta(nstate)?;
            conn[i] =
                (nm.class_flags & (BFLAG_PANE_OR_BARS | BFLAG_WALL | BFLAG_FULL_FACE)) != 0;
        }
        Ok(Legacy1201HorizontalConnections {
            north: conn[0],
            east: conn[1],
            south: conn[2],
            west: conn[3],
        })
    }

    /// 1.20.1 `WallBlock` connections (`cyj.a`): computes `(up, north, east, south, west)`.
    pub fn compute_wall_connections<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
    ) -> io::Result<Legacy1201WallConnections> {
        self.survival.verify_level(level)?;
        let above_state = level.block_state(pos.offset(0, 1, 0));
        let above_m = self.behavior_meta(above_state)?;
        let above_covers = (above_m.class_flags & (BFLAG_FULL_FACE | BFLAG_WALL)) != 0;

        let mut sides = [Legacy1201WallSide::None; 4];
        for (i, dir) in LEGACY1201_HORIZONTAL_DIRECTIONS.into_iter().enumerate() {
            let [dx, dy, dz] = dir.normal();
            let nstate = level.block_state(pos.offset(dx, dy, dz));
            let nm = self.behavior_meta(nstate)?;
            let gate_connects = if (nm.class_flags & BFLAG_FENCE_GATE) != 0 {
                let gate_facing = Direction::ALL[nm.facing as usize];
                legacy1201_is_horizontal_axis_x(gate_facing)
                    == legacy1201_is_horizontal_axis_x(legacy1201_clockwise(dir))
            } else {
                false
            };
            let connects = (nm.class_flags & (BFLAG_WALL | BFLAG_PANE_OR_BARS | BFLAG_FULL_FACE))
                != 0
                || gate_connects;
            if connects {
                sides[i] = if above_covers {
                    Legacy1201WallSide::Tall
                } else {
                    Legacy1201WallSide::Low
                };
            }
        }

        let north = sides[0];
        let east = sides[1];
        let south = sides[2];
        let west = sides[3];

        let ns_straight = north.is_connected()
            && south.is_connected()
            && !east.is_connected()
            && !west.is_connected()
            && north == south;
        let ew_straight = east.is_connected()
            && west.is_connected()
            && !north.is_connected()
            && !south.is_connected()
            && east == west;

        let above_wall_up = (above_m.class_flags & BFLAG_WALL) != 0;
        let up = above_wall_up || !(ns_straight || ew_straight);

        Ok(Legacy1201WallConnections {
            up,
            north,
            east,
            south,
            west,
        })
    }

    /// 1.20.1 `RedStoneWireBlock.getConnectionState` (`cvo.a`).
    pub fn compute_wire_connections<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
        was_dot: bool,
    ) -> io::Result<Legacy1201WireConnections> {
        self.survival.verify_level(level)?;
        let above_state = level.block_state(pos.offset(0, 1, 0));
        let can_connect_up = (self.behavior_meta(above_state)?.class_flags & BFLAG_CONDUCTOR) == 0;

        let mut sides = [Legacy1201RedstoneSide::None; 4];
        for (i, dir) in LEGACY1201_HORIZONTAL_DIRECTIONS.into_iter().enumerate() {
            sides[i] = self.wire_connecting_side(level, pos, dir, can_connect_up)?;
        }

        let mut result = Legacy1201WireConnections {
            north: sides[0],
            east: sides[1],
            south: sides[2],
            west: sides[3],
        };
        if was_dot && result.is_dot() {
            return Ok(result);
        }

        let n = result.north.is_connected();
        let s = result.south.is_connected();
        let e = result.east.is_connected();
        let w = result.west.is_connected();
        let ns_empty = !n && !s;
        let ew_empty = !e && !w;

        if !w && ns_empty {
            result.west = Legacy1201RedstoneSide::Side;
        }
        if !e && ns_empty {
            result.east = Legacy1201RedstoneSide::Side;
        }
        if !n && ew_empty {
            result.north = Legacy1201RedstoneSide::Side;
        }
        if !s && ew_empty {
            result.south = Legacy1201RedstoneSide::Side;
        }
        Ok(result)
    }

    fn wire_connecting_side<L: Legacy1201Level + ?Sized>(
        &self,
        level: &L,
        pos: BlockPos,
        dir: Direction,
        can_connect_up: bool,
    ) -> io::Result<Legacy1201RedstoneSide> {
        let [dx, dy, dz] = dir.normal();
        let rel_pos = pos.offset(dx, dy, dz);
        let rel_state = level.block_state(rel_pos);
        let rm = self.behavior_meta(rel_state)?;

        if can_connect_up {
            let placeable_above =
                (rm.class_flags & (BFLAG_TRAPDOOR | BFLAG_FULL_FACE)) != 0;
            let above_rel = level.block_state(rel_pos.offset(0, 1, 0));
            let arm = self.behavior_meta(above_rel)?;
            if placeable_above && (arm.class_flags & BFLAG_REDSTONE_WIRE) != 0 {
                if (rm.class_flags & BFLAG_FULL_FACE) != 0 {
                    return Ok(Legacy1201RedstoneSide::Up);
                }
                return Ok(Legacy1201RedstoneSide::Side);
            }
        }

        let connects_directly = (rm.class_flags & (BFLAG_REDSTONE_WIRE | BFLAG_SIGNAL_SOURCE)) != 0;
        if connects_directly {
            return Ok(Legacy1201RedstoneSide::Side);
        }
        if (rm.class_flags & BFLAG_CONDUCTOR) == 0 {
            let below_rel = level.block_state(rel_pos.offset(0, -1, 0));
            let brm = self.behavior_meta(below_rel)?;
            if (brm.class_flags & BFLAG_REDSTONE_WIRE) != 0 {
                return Ok(Legacy1201RedstoneSide::Side);
            }
        }
        Ok(Legacy1201RedstoneSide::None)
    }

    /// 1.20.1 `RedStoneWireBlock.updatePowerStrength` (`cvo.a`):
    /// calculates target strength `max(external_signal, max_neighbor_wire - 1)`, updates the wire
    /// state if changed, and enqueues `updateNeighborsAt` on `pos` and all 6 adjacent positions
    /// in exact `java.util.HashSet<BlockPos>` bucket order.
    pub fn update_wire_power_strength<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        updater: &mut Legacy1201CollectingNeighborUpdater,
        pos: BlockPos,
        external_block_signal: u8,
    ) -> io::Result<u8> {
        self.survival.verify_level(level)?;
        let state = level.block_state(pos);
        let m = self.behavior_meta(state)?;
        if (m.class_flags & BFLAG_REDSTONE_WIRE) == 0 {
            return Err(invalid("update_wire_power_strength called on non-wire"));
        }

        let mut max_wire = 0u8;
        if external_block_signal < 15 {
            let above_state = level.block_state(pos.offset(0, 1, 0));
            let above_conductor =
                (self.behavior_meta(above_state)?.class_flags & BFLAG_CONDUCTOR) != 0;
            for dir in LEGACY1201_HORIZONTAL_DIRECTIONS {
                let [dx, dy, dz] = dir.normal();
                let npos = pos.offset(dx, dy, dz);
                let nstate = level.block_state(npos);
                let nm = self.behavior_meta(nstate)?;
                if (nm.class_flags & BFLAG_REDSTONE_WIRE) != 0 {
                    max_wire = max_wire.max(nm.power_or_level);
                }
                if (nm.class_flags & BFLAG_CONDUCTOR) != 0 && !above_conductor {
                    let up_wire = level.block_state(npos.offset(0, 1, 0));
                    let um = self.behavior_meta(up_wire)?;
                    if (um.class_flags & BFLAG_REDSTONE_WIRE) != 0 {
                        max_wire = max_wire.max(um.power_or_level);
                    }
                } else if (nm.class_flags & BFLAG_CONDUCTOR) == 0 {
                    let down_wire = level.block_state(npos.offset(0, -1, 0));
                    let dm = self.behavior_meta(down_wire)?;
                    if (dm.class_flags & BFLAG_REDSTONE_WIRE) != 0 {
                        max_wire = max_wire.max(dm.power_or_level);
                    }
                }
            }
        }

        let target = external_block_signal.max(max_wire.saturating_sub(1));
        if m.power_or_level != target {
            let next_state = self.wire_state_with_power(target);
            level.set_raw(pos, next_state);

            let mut targets = [
                pos,
                pos.offset(0, -1, 0),
                pos.offset(0, 1, 0),
                pos.offset(0, 0, -1),
                pos.offset(0, 0, 1),
                pos.offset(-1, 0, 0),
                pos.offset(1, 0, 0),
            ];
            legacy1201_java_hash_set_order(&mut targets);
            for t in targets {
                updater.update_neighbors_at_except_from_facing(t, next_state, None);
            }
        }
        Ok(target)
    }

    /// 1.20.1 `FlowingFluid.getNewLiquid` (`dxc.a`): computes updated water/lava state at `pos`
    /// (infinite source conversion when `can_convert_to_source && adjacent_sources >= 2` over a
    /// solid block or source fluid of the same family, falling fluid `level = 8`, or decayed level).
    pub fn evaluate_fluid_flow_step<L: Legacy1201Level + ?Sized>(
        &self,
        level: &mut L,
        pos: BlockPos,
        is_lava: bool,
        ultra_warm: bool,
        can_convert_to_source: bool,
    ) -> io::Result<BlockStateId> {
        self.survival.verify_level(level)?;
        let above = level.block_state(pos.offset(0, 1, 0));
        let above_name = &self.survival.registry().state(above).unwrap().name;
        let same_fluid_above = if is_lava {
            above_name == "minecraft:lava"
        } else {
            above_name == "minecraft:water"
        };
        if same_fluid_above {
            let falling = if is_lava {
                self.lava_state_with_level(8)
            } else {
                self.water_state_with_level(8)
            };
            level.set_raw(pos, falling);
            return Ok(falling);
        }

        let mut max_amount = 0i32;
        let mut source_neighbors = 0usize;
        for dir in LEGACY1201_HORIZONTAL_DIRECTIONS {
            let [dx, dy, dz] = dir.normal();
            let nstate = level.block_state(pos.offset(dx, dy, dz));
            let nname = &self.survival.registry().state(nstate).unwrap().name;
            let same = if is_lava {
                nname == "minecraft:lava"
            } else {
                nname == "minecraft:water"
            };
            if same {
                let lvl = self.behavior_meta(nstate)?.power_or_level;
                let amount = if lvl == 0 {
                    source_neighbors += 1;
                    8
                } else if lvl >= 8 {
                    8
                } else {
                    8 - i32::from(lvl)
                };
                max_amount = max_amount.max(amount);
            }
        }

        if can_convert_to_source && source_neighbors >= 2 {
            let below = level.block_state(pos.offset(0, -1, 0));
            let bm = self.behavior_meta(below)?;
            let bname = &self.survival.registry().state(below).unwrap().name;
            let below_same_source = ((is_lava && bname == "minecraft:lava")
                || (!is_lava && bname == "minecraft:water"))
                && bm.power_or_level == 0;
            if (bm.class_flags & BFLAG_FULL_FACE) != 0 || below_same_source {
                let src = if is_lava {
                    self.survival.lava_source()
                } else {
                    self.survival.water_source()
                };
                level.set_raw(pos, src);
                return Ok(src);
            }
        }

        let drop_off = if !is_lava || ultra_warm { 1 } else { 2 };
        let new_amount = max_amount - drop_off;
        if new_amount <= 0 {
            let air = self.survival.air();
            level.set_raw(pos, air);
            Ok(air)
        } else {
            let new_level = (8 - new_amount) as u8;
            let next = if is_lava {
                self.lava_state_with_level(new_level)
            } else {
                self.water_state_with_level(new_level)
            };
            level.set_raw(pos, next);
            Ok(next)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunk::BlockState;
    use crate::level::LevelAccess;
    use horizon_core::random::LegacyRandom;
    use std::collections::{BTreeMap, HashMap};

    struct ScenarioLevel<'r> {
        registry: &'r BlockRegistry,
        air: BlockStateId,
        blocks: HashMap<BlockPos, BlockStateId>,
        random: LegacyRandom,
    }

    impl<'r> ScenarioLevel<'r> {
        fn new(registry: &'r BlockRegistry) -> Self {
            Self {
                registry,
                air: registry.default_state("minecraft:air").unwrap(),
                blocks: HashMap::new(),
                random: LegacyRandom::new(1201),
            }
        }
    }

    impl LevelAccess for ScenarioLevel<'_> {
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

    impl Legacy1201Level for ScenarioLevel<'_> {
        fn collision_data_version(&self) -> i32 {
            LEGACY_1_20_1_DATA_VERSION
        }
        fn collision_block_registry(&self) -> &BlockRegistry {
            self.registry
        }
    }

    fn build_stage2_3_registry() -> BlockRegistry {
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
        push_block("minecraft:oak_fence", vec![vec![]]);
        push_block("minecraft:spruce_fence", vec![vec![]]);
        push_block("minecraft:nether_brick_fence", vec![vec![]]);
        push_block("minecraft:cobblestone_wall", vec![vec![]]);
        push_block("minecraft:iron_bars", vec![vec![]]);
        push_block("minecraft:glass_pane", vec![vec![]]);

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

        // Doors (`oak_door`, `iron_door`)
        let mut door_states = Vec::new();
        for half in ["lower", "upper"] {
            for open in ["false", "true"] {
                for powered in ["false", "true"] {
                    door_states.push(vec![
                        ("facing", "north"),
                        ("half", half),
                        ("hinge", "left"),
                        ("open", open),
                        ("powered", powered),
                    ]);
                }
            }
        }
        push_block("minecraft:oak_door", door_states.clone());
        push_block("minecraft:iron_door", door_states);

        // Trapdoors (`oak_trapdoor`, `iron_trapdoor`)
        let mut trap_states = Vec::new();
        for open in ["false", "true"] {
            for powered in ["false", "true"] {
                for waterlogged in ["false", "true"] {
                    trap_states.push(vec![
                        ("facing", "north"),
                        ("half", "bottom"),
                        ("open", open),
                        ("powered", powered),
                        ("waterlogged", waterlogged),
                    ]);
                }
            }
        }
        push_block("minecraft:oak_trapdoor", trap_states.clone());
        push_block("minecraft:iron_trapdoor", trap_states);

        // Buttons & Lever
        let mut btn_states = Vec::new();
        for face in ["floor", "wall", "ceiling"] {
            for powered in ["false", "true"] {
                btn_states.push(vec![
                    ("face", face),
                    ("facing", "north"),
                    ("powered", powered),
                ]);
            }
        }
        push_block("minecraft:stone_button", btn_states.clone());
        push_block("minecraft:oak_button", btn_states.clone());
        push_block("minecraft:lever", btn_states);

        // Stairs (`oak_stairs`)
        let mut stair_states = Vec::new();
        for facing in ["north", "east", "south", "west"] {
            for half in ["bottom", "top"] {
                stair_states.push(vec![
                    ("facing", facing),
                    ("half", half),
                    ("shape", "straight"),
                    ("waterlogged", "false"),
                ]);
            }
        }
        push_block("minecraft:oak_stairs", stair_states);

        // Fence gate (`oak_fence_gate`)
        let mut gate_states = Vec::new();
        for facing in ["north", "east", "south", "west"] {
            gate_states.push(vec![
                ("facing", facing),
                ("in_wall", "false"),
                ("open", "false"),
                ("powered", "false"),
            ]);
        }
        push_block("minecraft:oak_fence_gate", gate_states);

        // Redstone wire (`0..=15`)
        let wire_states: Vec<Vec<(&str, &str)>> = (0..16)
            .map(|p| {
                vec![
                    ("east", "none"),
                    ("north", "none"),
                    ("power", Box::leak(p.to_string().into_boxed_str()) as &str),
                    ("south", "none"),
                    ("west", "none"),
                ]
            })
            .collect();
        push_block("minecraft:redstone_wire", wire_states);

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
    fn doors_trapdoors_buttons_and_levers_follow_1201_rules() {
        let registry = build_stage2_3_registry();
        let table = Legacy1201BehaviorTable::new(&registry).unwrap();
        let mut level = ScenarioLevel::new(&registry);
        let mut updater = Legacy1201CollectingNeighborUpdater::default();
        let mut ticks = Legacy1201ServerTickEngine::new(42);

        // Wooden door opens BOTH lower and upper halves by hand; iron door ignores hand click
        // but opens both halves on redstone signal!
        let lower_pos = BlockPos::new(0, 64, 0);
        let upper_pos = BlockPos::new(0, 65, 0);
        let oak_lower = registry.default_state("minecraft:oak_door").unwrap();
        let mut upper_desc = registry.state(oak_lower).unwrap().clone();
        upper_desc
            .properties
            .insert("half".to_string(), "upper".to_string());
        let oak_upper = registry.id(&upper_desc).unwrap();

        level.set_raw(lower_pos, oak_lower);
        level.set_raw(upper_pos, oak_upper);

        assert!(table.use_door(&mut level, lower_pos).unwrap());
        assert!(table.behavior_meta(level.block_state(lower_pos)).unwrap().open);
        assert!(table.behavior_meta(level.block_state(upper_pos)).unwrap().open);

        let iron_lower = registry.default_state("minecraft:iron_door").unwrap();
        let mut iron_upper_desc = registry.state(iron_lower).unwrap().clone();
        iron_upper_desc
            .properties
            .insert("half".to_string(), "upper".to_string());
        let iron_upper = registry.id(&iron_upper_desc).unwrap();
        level.set_raw(lower_pos, iron_lower);
        level.set_raw(upper_pos, iron_upper);

        assert!(!table.use_door(&mut level, lower_pos).unwrap());
        assert!(!table.behavior_meta(level.block_state(lower_pos)).unwrap().open);

        assert!(table.door_neighbor_changed(&mut level, lower_pos, true).unwrap());
        assert!(table.behavior_meta(level.block_state(lower_pos)).unwrap().open);
        assert!(table.behavior_meta(level.block_state(upper_pos)).unwrap().open);

        // Stone button schedules a 20-tick unpower and notifies both button pos and support pos.
        let btn_pos = BlockPos::new(5, 64, 5);
        let stone_btn = registry.default_state("minecraft:stone_button").unwrap();
        level.set_raw(btn_pos, stone_btn);

        assert!(
            table
                .press_button(&mut level, &mut updater, &mut ticks, 100, btn_pos)
                .unwrap()
        );
        assert!(table.behavior_meta(level.block_state(btn_pos)).unwrap().powered);

        // At tick 119 (delay 19 < 20), button is still powered; at tick 120, it unpowers.
        let (ran_119, _) = ticks
            .run_scheduled_ticks(119, |_, _| Ok(()), |_, _| Ok(()))
            .unwrap();
        assert_eq!(ran_119, 0);

        let (ran_120, _) = ticks
            .run_scheduled_ticks(
                120,
                |p, _| {
                    table.button_scheduled_tick(&mut level, &mut updater, p)?;
                    Ok(())
                },
                |_, _| Ok(()),
            )
            .unwrap();
        assert_eq!(ran_120, 1);
        assert!(!table.behavior_meta(level.block_state(btn_pos)).unwrap().powered);
    }

    #[test]
    fn stairs_fences_walls_panes_fluids_and_redstone_wire_match_1201() {
        let registry = build_stage2_3_registry();
        let table = Legacy1201BehaviorTable::new(&registry).unwrap();
        let mut level = ScenarioLevel::new(&registry);
        let mut updater = Legacy1201CollectingNeighborUpdater::default();

        // 1. Stairs shape: north-facing stair with an east-facing stair in front (North) -> OuterRight
        let stair_pos = BlockPos::new(10, 64, 10);
        let mut east_stair_desc = registry
            .state(registry.default_state("minecraft:oak_stairs").unwrap())
            .unwrap()
            .clone();
        east_stair_desc
            .properties
            .insert("facing".to_string(), "east".to_string());
        let east_stair = registry.id(&east_stair_desc).unwrap();
        level.set_raw(stair_pos.offset(0, 0, -1), east_stair);
        assert_eq!(
            table
                .compute_stair_shape(&level, stair_pos, Direction::North, false)
                .unwrap(),
            Legacy1201StairShape::OuterRight
        );

        // 2. Wooden fence connects to spruce_fence, does NOT connect to nether_brick_fence!
        let fence_pos = BlockPos::new(20, 64, 20);
        let spruce_fence = registry.default_state("minecraft:spruce_fence").unwrap();
        let nether_fence = registry
            .default_state("minecraft:nether_brick_fence")
            .unwrap();
        level.set_raw(fence_pos.offset(0, 0, -1), spruce_fence);
        level.set_raw(fence_pos.offset(1, 0, 0), nether_fence);
        let fconn = table
            .compute_fence_connections(&level, fence_pos, true)
            .unwrap();
        assert!(fconn.north);
        assert!(!fconn.east);

        // 3. Wall straight line (north + south connected, east + west none, air above) -> up = false;
        //    corner (north + east connected) -> up = true!
        let wall_pos = BlockPos::new(30, 64, 30);
        let wall = registry.default_state("minecraft:cobblestone_wall").unwrap();
        level.set_raw(wall_pos.offset(0, 0, -1), wall);
        level.set_raw(wall_pos.offset(0, 0, 1), wall);
        let w_straight = table.compute_wall_connections(&level, wall_pos).unwrap();
        assert!(!w_straight.up);
        assert_eq!(w_straight.north, Legacy1201WallSide::Low);
        assert_eq!(w_straight.south, Legacy1201WallSide::Low);

        level.set_raw(wall_pos.offset(0, 0, 1), table.survival.air());
        level.set_raw(wall_pos.offset(1, 0, 0), wall);
        let w_corner = table.compute_wall_connections(&level, wall_pos).unwrap();
        assert!(w_corner.up);

        // 4. Water infinite source conversion: 2 horizontal water sources over stone -> level 0 source!
        let fluid_pos = BlockPos::new(40, 64, 40);
        let stone = registry.default_state("minecraft:stone").unwrap();
        level.set_raw(fluid_pos.offset(0, -1, 0), stone);
        level.set_raw(fluid_pos.offset(-1, 0, 0), table.survival.water_source());
        level.set_raw(fluid_pos.offset(0, 0, -1), table.survival.water_source());
        let converted = table
            .evaluate_fluid_flow_step(&mut level, fluid_pos, false, false, true)
            .unwrap();
        assert_eq!(converted, table.survival.water_source());

        // 5. Redstone wire power propagation and 7-neighbor Java HashSet notification order.
        let wire_pos = BlockPos::new(50, 64, 50);
        level.set_raw(wire_pos, table.wire_state_with_power(0));
        level.set_raw(wire_pos.offset(1, 0, 0), table.wire_state_with_power(14));
        let new_power = table
            .update_wire_power_strength(&mut level, &mut updater, wire_pos, 0)
            .unwrap();
        assert_eq!(new_power, 13);
        assert_eq!(updater.count(), 7);
    }
}
