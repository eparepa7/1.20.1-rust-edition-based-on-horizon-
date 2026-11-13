//! Java 1.20.1 configured-carver block replacement, separate from 26.3.
//!
//! The geometry sink in `legacy_1_20_1_carver_world` calls this policy for
//! each candidate. The registered 1.20.1 replaceable set, lava anchor, debug
//! substitutions and Nether's independent lava cutoff are handled here.
//!
//! A 1.20.1 aquifer sampler is REQUIRED for ordinary cave/canyon cells above
//! the configured lava level. Its implementation must come from the 1.20.1
//! terrain pipeline, not the current 26.3 `TerrainAquifer`. The `after_write`
//! callback is REQUIRED to record fluid postprocessing and surface restoration
//! in the owning proto-chunk. This file does not claim those stages are done.
//!
//! The geometry sink carries the carving mask and duplicate-cell policy;
//! this module neither owns nor recreates them.

use crate::{
    compression::invalid,
    legacy_1_20_1_carver_world::{Legacy1201CarverColumn, Legacy1201CarverWorldOutput},
    legacy_1_20_1_world::{Legacy1201Level, LEGACY_1_20_1_DATA_VERSION},
    registry::{BlockRegistry, BlockStateId},
    worldgen::{
        anchor::WorldGenerationContext,
        carver::legacy1201::{Legacy1201Carver, Legacy1201CarverShape},
    },
};
use horizon_core::{
    carving::CarvingMask,
    position::{BlockPos, ChunkPos},
};
use std::{collections::HashSet, io, ptr};

/// One result from the *1.20.1* aquifer for a carver cell at density zero.
/// `None` means the aquifer's barrier (not air). The scheduler flag belongs
/// to the same sample, before another query can overwrite aquifer state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201AquiferSample {
    pub state: Option<BlockStateId>,
    pub schedule_fluid_update: bool,
}

/// Successful raw write, reported immediately to the owning proto-chunk.
/// No implicit 26.3 heightmap, fluid, or surface-rule hooks run here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201CarveWrite {
    pub position: BlockPos,
    pub previous: BlockStateId,
    pub replacement: BlockStateId,
    /// Was the directly replaced state grass or mycelium? This value alone
    /// cannot drive surface restoration: the column flag may have been set by
    /// an earlier, non-replaceable candidate.
    pub was_grassy: bool,
    /// The 1.20.1 WorldCarver foundSurface flag, sticky only within one
    /// ellipsoid column (including candidates which could not be replaced).
    pub found_surface: bool,
    /// Mark fluid postprocessing if this is true; never schedule a live-world
    /// fluid tick directly during proto-chunk carving.
    pub fluid_postprocessing: bool,
}

pub struct Legacy1201CarverPolicy<'c, 'r> {
    carver: &'c Legacy1201Carver<'r>,
    registry: &'r BlockRegistry,
    lava_y: i32,
    nether: bool,
    air: BlockStateId,
    water: BlockStateId,
    lava: BlockStateId,
    cave_air: BlockStateId,
    debug_water: BlockStateId,
}

impl<'c, 'r> Legacy1201CarverPolicy<'c, 'r> {
    /// Construct from the SAME 1.20.1 block registry used to decode `carver`
    /// and create the live level. The world/registry pointer is rechecked
    /// before writes; 26.3 block IDs are never accepted as a fallback.
    pub fn new(
        carver: &'c Legacy1201Carver<'r>,
        registry: &'r BlockRegistry,
        context: WorldGenerationContext,
    ) -> io::Result<Self> {
        if registry.len() != 24_135 || !(1..=4064).contains(&context.height) {
            return Err(invalid("invalid 1.20.1 carver registry or world height"));
        }
        let required = |name| {
            registry.default_state(name).ok_or_else(|| {
                invalid(format!("missing 1.20.1 carver state {name}"))
            })
        };
        let air = required("minecraft:air")?;
        let water = required("minecraft:water")?;
        let lava = required("minecraft:lava")?;
        let cave_air = required("minecraft:cave_air")?;
        let nether = matches!(
            &carver.shape,
            Legacy1201CarverShape::Cave { nether: true, .. }
        );
        let lava_y = if nether {
            // NetherWorldCarver ignores configured `lava_level`.
            context.min_y.checked_add(31)
                .ok_or_else(|| invalid("1.20.1 Nether lava cutoff overflow"))?
        } else {
            carver.config.lava_level.resolve(context)
        };
        let mut debug_water = carver.config.debug.states[1];
        if carver.config.debug.enabled && !nether {
            // `getDebugState(WATER)` sets WATERLOGGED=true on a replacement
            // block that carries that property (e.g. the configured candle).
            let mut descriptor = registry.state(debug_water)
                .ok_or_else(|| invalid("1.20.1 debug water outside registry"))?
                .clone();
            if descriptor.properties.contains_key("waterlogged") {
                descriptor.properties.insert("waterlogged".into(), "true".into());
                debug_water = registry.resolve(&descriptor)?;
            }
        }
        Ok(Self {
            carver, registry, lava_y, nether, air, water, lava,
            cave_air, debug_water,
        })
    }

    pub const fn carver(&self) -> &'c Legacy1201Carver<'r> {
        self.carver
    }

    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }

    pub fn debug(&self) -> bool {
        self.carver.config.debug.enabled
    }

    /// Apply one geometry candidate to a live 1.20.1 proto-chunk.
    /// `aquifer` is *not called* for Nether caves or for cells below lava_y.
    /// The `column` flag must be reset at the start of each ellipsoid column.
    /// `after_write` runs only after a successful raw storage write, and its
    /// error leaves that write in place (the same non-transactional ordering
    /// as other worldgen writes). Surface restoration is not automatic.
    pub fn apply<L, A, E>(
        &self,
        level: &mut L,
        position: BlockPos,
        column: &mut Legacy1201CarverColumn,
        aquifer: &mut A,
        after_write: &mut E,
    ) -> io::Result<bool>
    where
        L: Legacy1201Level,
        A: FnMut(BlockPos) -> io::Result<Legacy1201AquiferSample>,
        E: FnMut(&mut L, Legacy1201CarveWrite) -> io::Result<()>,
    {
        if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
            || !ptr::eq(level.collision_block_registry(), self.registry)
        {
            return Err(invalid("1.20.1 carver policy level/registry mismatch"));
        }
        if position.y < level.min_y() || position.y >= level.max_y()
            || !level.is_loaded(position)
        {
            return Err(invalid("1.20.1 carver policy position not loaded or in bounds"));
        }
        let old = level.block_state(position);
        let old_name = &self.registry.state(old)
            .ok_or_else(|| invalid("1.20.1 carver encountered unknown block ID"))?
            .name;
        let was_grassy = matches!(old_name.as_str(), "minecraft:grass_block" | "minecraft:mycelium");
        // In Java the MutableBoolean is set before the replaceable and
        // aquifer checks, even when the block is ultimately left untouched.
        column.observed(was_grassy);
        let replaceable = self.carver.config.can_replace(old)
            .map_err(|e| invalid(format!("1.20.1 replaceable lookup: {e}")))?;
        // NetherWorldCarver is independent of the debug-mode bypass used by
        // WorldCarver. Its own replacement rule uses only the holder set.
        if !replaceable && (self.nether || !self.carver.config.debug.enabled) {
            return Ok(false);
        }

        let (replacement, postprocess) = if self.nether {
            (nether_carved_state(position.y, self.lava_y, self.lava, self.cave_air), false)
        } else {
            let sample = if position.y <= self.lava_y {
                Legacy1201AquiferSample {
                    state: Some(self.lava), schedule_fluid_update: false,
                }
            } else {
                aquifer(position)?
            };
            let Some((replacement, schedule)) = select_carved_state(
                sample,
                self.carver.config.debug.enabled.then_some(
                    [self.carver.config.debug.states[0], self.debug_water,
                     self.carver.config.debug.states[2], self.carver.config.debug.states[3]],
                ),
                self.air, self.water, self.lava,
            ) else {
                return Ok(false);
            };
            let new_descriptor = self.registry.state(replacement)
                .ok_or_else(|| invalid("1.20.1 aquifer/debug state outside registry"))?;
            let has_fluid = matches!(new_descriptor.name.as_str(), "minecraft:water" | "minecraft:lava")
                || new_descriptor.properties.get("waterlogged").is_some_and(|v| v == "true");
            (replacement, schedule && has_fluid)
        };
        if self.registry.state(replacement).is_none() {
            return Err(invalid("1.20.1 carver replacement outside registry"));
        }
        let previous = level.set_raw(position, replacement)
            .ok_or_else(|| invalid("1.20.1 carver raw write rejected"))?;
        after_write(level, Legacy1201CarveWrite {
            position,
            previous,
            replacement,
            was_grassy,
            found_surface: column.found_surface(),
            fluid_postprocessing: postprocess,
        })?;
        Ok(true)
    }
}

/// Replace the 1.5.6 sink's arbitrary policy with a decoded 1.20.1 carver
/// policy. Effects remain mandatory: in particular, implement per-column
/// surface restoration in `after_write` before accepting world parity.
pub fn output_with_legacy1201_policy<'a, L, A, E>(
    level: &'a mut L,
    policy: &'a Legacy1201CarverPolicy<'_, '_>,
    mask: &'a mut CarvingMask,
    seen: &'a mut HashSet<(i32, i32, i32)>,
    target: ChunkPos,
    aquifer: &'a mut A,
    after_write: &'a mut E,
) -> io::Result<Legacy1201CarverWorldOutput<'a, L, impl FnMut(&mut L, BlockPos, &mut Legacy1201CarverColumn) -> io::Result<bool> + 'a>>
where
    L: Legacy1201Level + 'a,
    A: FnMut(BlockPos) -> io::Result<Legacy1201AquiferSample> + 'a,
    E: FnMut(&mut L, Legacy1201CarveWrite) -> io::Result<()> + 'a,
{
    let registry = policy.registry();
    Legacy1201CarverWorldOutput::new(
        level, registry, mask, seen, target, policy.debug(),
        move |world: &mut L, position, column| policy.apply(world, position, column, aquifer, after_write),
    )
}

fn nether_carved_state(y: i32, cutoff: i32, lava: BlockStateId, cave_air: BlockStateId) -> BlockStateId {
    if y <= cutoff { lava } else { cave_air }
}

/// Returns None for a solid aquifer barrier in non-debug mode; in debug mode
/// it becomes the configured barrier marker. Output's scheduling flag always
/// belongs to the same sample, including when a debug substitution is used.
fn select_carved_state(
    sample: Legacy1201AquiferSample,
    debug: Option<[BlockStateId; 4]>,
    air: BlockStateId,
    water: BlockStateId,
    lava: BlockStateId,
) -> Option<(BlockStateId, bool)> {
    let state = match (sample.state, debug) {
        (None, None) => return None,
        (None, Some(markers)) => markers[3],
        (Some(state), Some(markers)) if state == air => markers[0],
        (Some(state), Some(markers)) if state == water => markers[1],
        (Some(state), Some(markers)) if state == lava => markers[2],
        (Some(state), _) => state,
    };
    Some((state, sample.schedule_fluid_update))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(id: Option<u32>, schedule: bool) -> Legacy1201AquiferSample {
        Legacy1201AquiferSample {
            state: id.map(BlockStateId),
            schedule_fluid_update: schedule,
        }
    }

    #[test]
    fn nether_cutoff_is_inclusive_and_ignores_aquifers() {
        assert_eq!(nether_carved_state(31, 31, BlockStateId(10), BlockStateId(11)), BlockStateId(10));
        assert_eq!(nether_carved_state(32, 31, BlockStateId(10), BlockStateId(11)), BlockStateId(11));
        assert_eq!(nether_carved_state(-1, 31, BlockStateId(10), BlockStateId(11)), BlockStateId(10));
    }

    #[test]
    fn missing_aquifer_state_is_not_air() {
        assert_eq!(select_carved_state(sample(None, false), None, BlockStateId(1), BlockStateId(2), BlockStateId(3)), None);
    }

    #[test]
    fn debug_barrier_replaces_missing_aquifer_state() {
        let markers = [BlockStateId(20), BlockStateId(21), BlockStateId(22), BlockStateId(23)];
        assert_eq!(select_carved_state(sample(None, false), Some(markers), BlockStateId(1), BlockStateId(2), BlockStateId(3)), Some((markers[3], false)));
    }

    #[test]
    fn debug_air_water_and_lava_each_use_their_own_marker() {
        let markers = [BlockStateId(20), BlockStateId(21), BlockStateId(22), BlockStateId(23)];
        for (raw, wanted) in [(1, 20), (2, 21), (3, 22)] {
            assert_eq!(select_carved_state(sample(Some(raw), true), Some(markers), BlockStateId(1), BlockStateId(2), BlockStateId(3)), Some((BlockStateId(wanted), true)));
        }
    }

    #[test]
    fn other_aquifer_states_are_not_replaced_with_debug_air() {
        let markers = [BlockStateId(20), BlockStateId(21), BlockStateId(22), BlockStateId(23)];
        assert_eq!(select_carved_state(sample(Some(4), false), Some(markers), BlockStateId(1), BlockStateId(2), BlockStateId(3)), Some((BlockStateId(4), false)));
    }

    #[test]
    fn aquifer_fluid_scheduling_flag_survives_selection() {
        assert_eq!(select_carved_state(sample(Some(2), true), None, BlockStateId(1), BlockStateId(2), BlockStateId(3)), Some((BlockStateId(2), true)));
        assert_eq!(select_carved_state(sample(Some(2), false), None, BlockStateId(1), BlockStateId(2), BlockStateId(3)), Some((BlockStateId(2), false)));
    }
}
