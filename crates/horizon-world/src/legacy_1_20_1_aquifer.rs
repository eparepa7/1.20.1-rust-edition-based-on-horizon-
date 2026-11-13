//! Version-isolated aquifer primitives for Minecraft Java 1.20.1.
//!
//! This module implements the global fluid picker, the *disabled* aquifer,
//! and the 12-cell positional search used by the noise-based aquifer. It does
//! NOT approximate the 1.20.1 noise-based aquifer with 26.3 code or with the
//! disabled mode. For a vanilla Overworld, the remaining parts (surface
//! sampling, floodedness/spread/lava noises and pressure barriers) must be
//! implemented and compared with 1.20.1 before enabling chunk generation.
//!
//! A disabled aquifer can be passed to `output_with_legacy1201_postprocessing`
//! as `let mut sample = |pos| Ok(disabled.sample_carver_cell(pos));`, but ONLY
//! when the dimension's 1.20.1 noise settings disable aquifers. Ordinary
//! Overworld carving still requires a separate noise-based sampler.

use crate::{
    compression::invalid,
    legacy_1_20_1_carver_policy::Legacy1201AquiferSample,
    legacy_1_20_1_world::{Legacy1201Level, LEGACY_1_20_1_DATA_VERSION},
    registry::{BlockRegistry, BlockStateId},
};
use horizon_core::{
    position::BlockPos,
    random::{PositionalRandom, RandomSource},
};
use std::{io, ptr};

/// Java Aquifer.FluidStatus: a fluid is present strictly BELOW its surface.
/// The fluid state need not be water; noise settings choose the default fluid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201FluidStatus {
    pub level: i32,
    pub state: BlockStateId,
}

impl Legacy1201FluidStatus {
    pub const fn at(self, y: i32, air: BlockStateId) -> BlockStateId {
        if y < self.level { self.state } else { air }
    }
}

/// 1.20.1 NoiseBasedChunkGenerator global fluid picker, independent of 26.3.
/// The vanilla cutover uses `min(-54, sea_level)`; fluidLevel is -54 on the
/// lava side, and sea_level on the default-fluid side.
#[derive(Clone, Copy)]
pub struct Legacy1201GlobalFluidPicker<'r> {
    registry: &'r BlockRegistry,
    sea_level: i32,
    default_fluid: BlockStateId,
    lava: BlockStateId,
    air: BlockStateId,
}

impl<'r> Legacy1201GlobalFluidPicker<'r> {
    pub fn new(
        registry: &'r BlockRegistry,
        sea_level: i32,
        default_fluid: BlockStateId,
    ) -> io::Result<Self> {
        if registry.len() != 24_135 || registry.state(default_fluid).is_none() {
            return Err(invalid("1.20.1 aquifer needs its own 24135-state registry"));
        }
        let air = registry.default_state("minecraft:air")
            .ok_or_else(|| invalid("1.20.1 aquifer registry has no air"))?;
        let lava = registry.default_state("minecraft:lava")
            .ok_or_else(|| invalid("1.20.1 aquifer registry has no lava"))?;
        Ok(Self { registry, sea_level, default_fluid, lava, air })
    }

    /// Check identity before attaching the sampler to an actual proto-chunk.
    /// The same numeric state IDs in two registries do not imply the same blocks.
    pub fn verify_level<L: Legacy1201Level>(&self, level: &L) -> io::Result<()> {
        if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
            || !ptr::eq(level.collision_block_registry(), self.registry)
        {
            return Err(invalid("1.20.1 aquifer/level version or registry mismatch"));
        }
        Ok(())
    }

    pub const fn registry(&self) -> &'r BlockRegistry { self.registry }

    pub const fn air(&self) -> BlockStateId { self.air }

    pub const fn lava(&self) -> BlockStateId { self.lava }

    pub const fn default_fluid(&self) -> BlockStateId { self.default_fluid }

    pub const fn sea_level(&self) -> i32 { self.sea_level }

    pub fn compute(&self, position: BlockPos) -> Legacy1201FluidStatus {
        global_fluid_at(self.sea_level, self.default_fluid, self.lava, position.y)
    }
}

fn global_fluid_at(
    sea_level: i32,
    default_fluid: BlockStateId,
    lava: BlockStateId,
    y: i32,
) -> Legacy1201FluidStatus {
    if y < sea_level.min(-54) {
        Legacy1201FluidStatus { level: -54, state: lava }
    } else {
        Legacy1201FluidStatus { level: sea_level, state: default_fluid }
    }
}

/// Java Aquifer.createDisabled, for dimensions with aquifers_enabled=false.
/// Do not use it as a fallback when the Overworld noise router is unavailable.
pub struct Legacy1201DisabledAquifer<'r> {
    picker: Legacy1201GlobalFluidPicker<'r>,
}

impl<'r> Legacy1201DisabledAquifer<'r> {
    pub const fn new(picker: Legacy1201GlobalFluidPicker<'r>) -> Self {
        Self { picker }
    }

    pub fn verify_level<L: Legacy1201Level>(&self, level: &L) -> io::Result<()> {
        self.picker.verify_level(level)
    }

    /// Positive density produces solid terrain (None). Java DCMPL treats NaN
    /// as non-positive here, so NaN asks the picker rather than making stone.
    pub fn compute_substance(&self, position: BlockPos, density: f64) -> Option<BlockStateId> {
        if density > 0.0 {
            None
        } else {
            Some(self.picker.compute(position).at(position.y, self.picker.air()))
        }
    }

    /// `WorldCarver` calls its aquifer with density 0, not the terrain density.
    /// The disabled implementation never requests fluid postprocessing.
    pub fn sample_carver_cell(&self, position: BlockPos) -> Legacy1201AquiferSample {
        Legacy1201AquiferSample {
            state: Some(self.picker.compute(position).at(position.y, self.picker.air())),
            schedule_fluid_update: false,
        }
    }
}

/// The noise-aquifer's positional source cell. The 1.20.1 search examines
/// 2 x 3 x 2 neighboring cells around this anchor in X/Y/Z loop order.
/// `wrapping_*` matches Java's signed int overflow; negative positions must
/// use floor division, not Rust's truncating `/`.
pub fn legacy1201_aquifer_anchor(position: BlockPos) -> BlockPos {
    BlockPos::new(
        position.x.wrapping_sub(5).div_euclid(16),
        position.y.wrapping_add(1).div_euclid(12),
        position.z.wrapping_sub(5).div_euclid(16),
    )
}

/// Reproducible 1.20.1 randomized site of a grid cell (16 x 12 x 16).
/// X/Z draw from [0,10), Y from [0,9), using the provided *aquifer*
/// positional RNG factory. The caller must fork that factory in 1.20.1 order.
/// Java caches packed BlockPos longs; pack/unpack preserves its truncation.
pub fn legacy1201_aquifer_site<R: PositionalRandom>(
    random: &R,
    cell: BlockPos,
) -> BlockPos {
    let mut rng = random.at(cell);
    let x = cell.x.wrapping_mul(16)
        .wrapping_add(rng.next_int_bound(10).expect("constant positive bound"));
    let y = cell.y.wrapping_mul(12)
        .wrapping_add(rng.next_int_bound(9).expect("constant positive bound"));
    let z = cell.z.wrapping_mul(16)
        .wrapping_add(rng.next_int_bound(10).expect("constant positive bound"));
    BlockPos::unpack(BlockPos::new(x, y, z).pack())
}

/// One of the three nearest randomized aquifer sites, retaining its source
/// cell so the eventual FluidStatus cache can be keyed correctly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Legacy1201AquiferSite {
    pub cell: BlockPos,
    pub position: BlockPos,
    pub squared_distance: i32,
}

/// Select the nearest three of twelve cells, preserving the 1.20.1 visit and
/// tie order. This is geometry ONLY: it does not decide fluids or pressure.
/// The owning noise-based aquifer will cache the same sites and statuses.
pub fn legacy1201_nearest_aquifer_sites<R: PositionalRandom>(
    random: &R,
    position: BlockPos,
) -> [Legacy1201AquiferSite; 3] {
    let anchor = legacy1201_aquifer_anchor(position);
    let initial = Legacy1201AquiferSite {
        cell: BlockPos::default(),
        position: BlockPos::default(),
        squared_distance: i32::MAX,
    };
    let mut best = [initial; 3];
    for dx in 0..=1i32 {
        for dy in -1..=1i32 {
            for dz in 0..=1i32 {
                let cell = BlockPos::new(
                    anchor.x.wrapping_add(dx),
                    anchor.y.wrapping_add(dy),
                    anchor.z.wrapping_add(dz),
                );
                let site = legacy1201_aquifer_site(random, cell);
                let vx = site.x.wrapping_sub(position.x);
                let vy = site.y.wrapping_sub(position.y);
                let vz = site.z.wrapping_sub(position.z);
                let distance = vx.wrapping_mul(vx)
                    .wrapping_add(vy.wrapping_mul(vy))
                    .wrapping_add(vz.wrapping_mul(vz));
                let candidate = Legacy1201AquiferSite {
                    cell, position: site, squared_distance: distance,
                };
                insert_nearest(&mut best, candidate);
            }
        }
    }
    best
}

/// Equal distances displace earlier entries, rather than picking the first
/// visited site. This ordering matters when two fluid tables disagree.
pub(crate) fn insert_nearest(
    best: &mut [Legacy1201AquiferSite; 3],
    candidate: Legacy1201AquiferSite,
) {
    let distance = candidate.squared_distance;
    if distance <= best[0].squared_distance {
        best[2] = best[1];
        best[1] = best[0];
        best[0] = candidate;
    } else if distance <= best[1].squared_distance {
        best[2] = best[1];
        best[1] = candidate;
    } else if distance <= best[2].squared_distance {
        best[2] = candidate;
    }
}

// ---------------------------------------------------------------------------
// 1.5.16: 1.20.1 noise-aquifer fluid-level and fluid-type primitives.
//
// Deliberately independent of the Horizon 26.3 noise router. The supplied
// callbacks receive *noise-grid* coordinates, not world block coordinates.
// Callers must use the Minecraft Java 1.20.1 noise router and preliminary
// surface heights. These helpers do not enable 1.20.1 chunk generation.
// ---------------------------------------------------------------------------

/// Java aquifer's sentinel for a dry cell (not a real world Y level).
pub const LEGACY1201_DRY_FLUID_LEVEL: i32 = -32_512;

/// The three floodedness outcomes that can be chosen from local noise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201Floodedness {
    Flooded,
    Randomized,
    Dry,
}

/// Select the cell's water-level mode using its surface distance and the
/// 1.20.1 `fluid_level_floodedness` density value.
///
/// `lowest_surface` is the lowest **preliminary** surface sampled near the
/// aquifer cell (the `+8` adjustment is applied here). `surface_under_global`
/// means the centre's adjusted surface is covered by its global fluid picker.
/// Deep-dark exclusion must happen before sampling floodedness noise.
///
/// Thresholds are *strict*: at a threshold equality selects the next lower
/// mode. This routine is not a replacement for 13-point surface sampling.
pub fn legacy1201_aquifer_floodedness(
    y: i32,
    lowest_surface: i32,
    surface_under_global: bool,
    deep_dark: bool,
    floodedness_noise: f64,
) -> io::Result<Legacy1201Floodedness> {
    if deep_dark {
        return Ok(Legacy1201Floodedness::Dry);
    }
    if lowest_surface == i32::MAX || !floodedness_noise.is_finite() {
        return Err(invalid("1.20.1 aquifer needs preliminary surface and finite floodedness noise"));
    }

    // Under a flooded surface, adjust the thresholds over a 64-block band;
    // otherwise use the ordinary 0.8 / 0.4 thresholds.
    let distance = (i64::from(lowest_surface) + 8 - i64::from(y)) as f64;
    let influence = if surface_under_global {
        1.0 - (distance / 64.0).clamp(0.0, 1.0)
    } else {
        0.0
    };
    // Preserve Java Mth.map interpolation order at exact threshold values.
    let t = 1.0 - influence;
    let fully_flooded_threshold = -0.3 + t * (0.8 - (-0.3));
    let partially_flooded_threshold = -0.8 + t * (0.4 - (-0.8));
    let noise = floodedness_noise.clamp(-1.0, 1.0);

    if noise > fully_flooded_threshold {
        Ok(Legacy1201Floodedness::Flooded)
    } else if noise > partially_flooded_threshold {
        Ok(Legacy1201Floodedness::Randomized)
    } else {
        Ok(Legacy1201Floodedness::Dry)
    }
}

/// Compute the fluid surface for a *randomized* floodedness cell.
///
/// Java divides the coordinates by 16/40/16 using floor division, samples
/// `fluid_level_spread` at those grid coordinates, scales it by 10, and
/// quantizes the result in 3-block steps. The base Y is the centre of the
/// 40-block cell; the final level cannot exceed the lowest preliminary
/// surface. `sample_spread` must use the 1.20.1 router, not 26.3.
pub fn legacy1201_randomized_fluid_surface<F>(
    position: BlockPos,
    lowest_surface: i32,
    sample_spread: &mut F,
) -> io::Result<i32>
where
    F: FnMut(BlockPos) -> io::Result<f64>,
{
    if lowest_surface == i32::MAX {
        return Err(invalid("missing 1.20.1 preliminary surface"));
    }
    let cell = BlockPos::new(
        position.x.div_euclid(16),
        position.y.div_euclid(40),
        position.z.div_euclid(16),
    );
    let spread = sample_spread(cell)?;
    if !spread.is_finite() {
        return Err(invalid("non-finite 1.20.1 aquifer fluid_level_spread"));
    }
    let steps = ((spread * 10.0) / 3.0).floor();
    if steps < (i32::MIN as f64) / 3.0 || steps > (i32::MAX as f64) / 3.0 {
        return Err(invalid("out-of-range 1.20.1 aquifer spread"));
    }
    let offset = (steps as i32).checked_mul(3)
        .ok_or_else(|| invalid("1.20.1 aquifer spread overflow"))?;
    let centre = cell.y.checked_mul(40)
        .and_then(|value| value.checked_add(20))
        .ok_or_else(|| invalid("1.20.1 aquifer cell Y overflow"))?;
    let level = centre.checked_add(offset)
        .ok_or_else(|| invalid("1.20.1 aquifer fluid level overflow"))?;
    Ok(level.min(lowest_surface))
}

/// Select the actual fluid state for one aquifer cell.
///
/// At levels above -10, in dry cells, and when the global picker already
/// chooses lava, no lava-noise sample is needed. Else 1.20.1 samples the
/// `lava` noise in 64 x 40 x 64 cells, selecting lava for |noise| > 0.3.
/// The callback is required to return the 1.20.1 router value.
pub fn legacy1201_aquifer_fluid_type<F>(
    position: BlockPos,
    fluid_level: i32,
    global: Legacy1201FluidStatus,
    lava: BlockStateId,
    sample_lava: &mut F,
) -> io::Result<BlockStateId>
where
    F: FnMut(BlockPos) -> io::Result<f64>,
{
    if fluid_level > -10 || fluid_level == LEGACY1201_DRY_FLUID_LEVEL || global.state == lava {
        return Ok(global.state);
    }
    let cell = BlockPos::new(
        position.x.div_euclid(64),
        position.y.div_euclid(40),
        position.z.div_euclid(64),
    );
    let noise = sample_lava(cell)?;
    if !noise.is_finite() {
        return Err(invalid("non-finite 1.20.1 aquifer lava noise"));
    }
    Ok(if noise.abs() > 0.3 { lava } else { global.state })
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_core::random::{LegacyPositionalRandom, XoroshiroPositionalRandom};

    const AIR: BlockStateId = BlockStateId(0);
    const WATER: BlockStateId = BlockStateId(1);
    const LAVA: BlockStateId = BlockStateId(2);
    const OTHER: BlockStateId = BlockStateId(3);

    #[test]
    fn fluid_level_is_exclusive() {
        let fluid = Legacy1201FluidStatus { level: 63, state: WATER };
        assert_eq!(fluid.at(62, AIR), WATER);
        assert_eq!(fluid.at(63, AIR), AIR);
        assert_eq!(fluid.at(64, AIR), AIR);
    }

    #[test]
    fn overworld_lava_and_sea_boundaries() {
        for (y, expected) in [
            (-64, LAVA), (-55, LAVA), (-54, WATER), (62, WATER),
            (63, AIR), (319, AIR),
        ] {
            let status = global_fluid_at(63, WATER, LAVA, y);
            assert_eq!(status.at(y, AIR), expected, "y={y}");
        }
        assert_eq!(global_fluid_at(63, WATER, LAVA, -55).level, -54);
        assert_eq!(global_fluid_at(63, WATER, LAVA, -54).level, 63);
    }

    #[test]
    fn custom_sea_level_and_non_water_default_are_preserved() {
        let at = |y| global_fluid_at(-80, OTHER, LAVA, y).at(y, AIR);
        assert_eq!(at(-81), LAVA);
        assert_eq!(at(-80), AIR);
        assert_eq!(at(-54), AIR);
        let at_nether = |y| global_fluid_at(32, LAVA, LAVA, y).at(y, AIR);
        assert_eq!(at_nether(31), LAVA);
        assert_eq!(at_nether(32), AIR);
    }

    fn small_registry() -> BlockRegistry {
        use crate::nbt::Tag;
        let names = ["minecraft:air", "minecraft:water", "minecraft:lava", "test:other"];
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("states", Tag::List(names.iter().map(|name| {
                Tag::compound([("id", Tag::String((*name).into()))])
            }).collect())),
            ("defaults", Tag::compound(names.iter().enumerate().map(|(i, name)| {
                (*name, Tag::Int(i as i32))
            }))),
        ])).unwrap()
    }

    #[test]
    fn picker_rejects_unversioned_fixture_and_invalid_id() {
        let registry = small_registry();
        assert!(Legacy1201GlobalFluidPicker::new(&registry, 63, WATER).is_err());
        assert!(Legacy1201GlobalFluidPicker::new(&registry, 63, BlockStateId(u32::MAX)).is_err());
    }

    #[test]
    fn disabled_aquifer_does_not_schedule_and_respects_positive_density() {
        // The normal constructor requires a complete 1.20.1 registry;
        // this small synthetic registry tests only the pure fluid rule.
        let registry = small_registry();
        let picker = Legacy1201GlobalFluidPicker {
            registry: &registry, sea_level: 63,
            default_fluid: WATER, lava: LAVA, air: AIR,
        };
        for y in [-64, -54, 0, 62, 63, 320] {
            let reference = picker.compute(BlockPos::new(0, y, 0));
            assert_eq!(picker.compute(BlockPos::new(-1000, y, 2000)), reference);
        }
        let disabled = Legacy1201DisabledAquifer::new(picker);
        let under_sea = BlockPos::new(0, 62, 0);
        for density in [0.01, 1.0, f64::INFINITY] {
            assert_eq!(disabled.compute_substance(under_sea, density), None);
        }
        for density in [0.0, -1.0, f64::NEG_INFINITY, f64::NAN] {
            assert_eq!(disabled.compute_substance(under_sea, density), Some(WATER));
        }
        assert_eq!(disabled.sample_carver_cell(BlockPos::new(0, -64, 0)),
            Legacy1201AquiferSample { state: Some(LAVA), schedule_fluid_update: false });
        assert_eq!(disabled.sample_carver_cell(BlockPos::new(0, 63, 0)),
            Legacy1201AquiferSample { state: Some(AIR), schedule_fluid_update: false });
    }

    #[test]
    fn anchors_floor_divide_negative_coordinates() {
        let cases = [
            (BlockPos::new(5, -1, 5), BlockPos::new(0, 0, 0)),
            (BlockPos::new(4, -2, 4), BlockPos::new(-1, -1, -1)),
            (BlockPos::new(-11, -13, -11), BlockPos::new(-1, -1, -1)),
            (BlockPos::new(-12, -14, -12), BlockPos::new(-2, -2, -2)),
            (BlockPos::new(21, 11, 21), BlockPos::new(1, 1, 1)),
        ];
        for (position, expected) in cases {
            assert_eq!(legacy1201_aquifer_anchor(position), expected, "{position:?}");
        }
    }

    #[test]
    fn jitter_offsets_obey_legacy_spacing_and_are_deterministic() {
        let random = LegacyPositionalRandom::new(1234567);
        for x in -3..=3 {
            for y in -3..=3 {
                for z in -3..=3 {
                    let cell = BlockPos::new(x, y, z);
                    let site = legacy1201_aquifer_site(&random, cell);
                    assert_eq!(site, legacy1201_aquifer_site(&random, cell));
                    assert!((0..10).contains(&(site.x - x * 16)));
                    assert!((0..9).contains(&(site.y - y * 12)));
                    assert!((0..10).contains(&(site.z - z * 16)));
                }
            }
        }
    }

    #[test]
    fn positional_jitter_matches_independent_java_lcg_vectors() {
        // Reference arithmetic: Java 48-bit LCG + Mth.getSeed(BlockPos),
        // with X/Y/Z nextInt(10/9/10) in that order. Not a server oracle.
        for (seed, cell, expected) in [
            (0, BlockPos::new(0, 0, 0), BlockPos::new(0, 7, 9)),
            (7, BlockPos::new(-1, -2, 3), BlockPos::new(-10, -23, 54)),
            (-123, BlockPos::new(12, 5, -8), BlockPos::new(193, 62, -125)),
            (1_234_567, BlockPos::new(3, -3, -3), BlockPos::new(55, -30, -46)),
        ] {
            let random = LegacyPositionalRandom::new(seed);
            assert_eq!(legacy1201_aquifer_site(&random, cell), expected);
        }
    }

    #[test]
    fn nearest_sites_are_sorted_and_from_the_twelve_neighbours() {
        for seed in [0, 1, -1, 9999] {
            let random = LegacyPositionalRandom::new(seed);
            for position in [
                BlockPos::new(0, -64, 0), BlockPos::new(-20, 63, 17),
                BlockPos::new(42, 300, -42), BlockPos::new(5, -2, 4),
            ] {
                let nearest = legacy1201_nearest_aquifer_sites(&random, position);
                let anchor = legacy1201_aquifer_anchor(position);
                let mut distances = Vec::new();
                for dx in 0..=1 {
                    for dy in -1..=1 {
                        for dz in 0..=1 {
                            let cell = anchor.offset(dx, dy, dz);
                            let site = legacy1201_aquifer_site(&random, cell);
                            let d = site.x - position.x;
                            let e = site.y - position.y;
                            let f = site.z - position.z;
                            distances.push((d*d+e*e+f*f, cell, site));
                        }
                    }
                }
                assert!(nearest[0].squared_distance <= nearest[1].squared_distance);
                assert!(nearest[1].squared_distance <= nearest[2].squared_distance);
                let mut ranked: Vec<i32> = distances.iter().map(|v| v.0).collect();
                ranked.sort_unstable();
                for (index, item) in nearest.iter().enumerate() {
                    assert_eq!(item.squared_distance, ranked[index]);
                    assert!(distances.iter().any(|v| *v ==
                        (item.squared_distance, item.cell, item.position)));
                }
            }
        }
    }

    #[test]
    fn xoroshiro_positional_factory_obeys_the_same_site_contract() {
        let factory = XoroshiroPositionalRandom::new(5, 8);
        for cell in [BlockPos::new(-2, -6, 3), BlockPos::new(0, 0, 0),
                     BlockPos::new(20, 12, -20)] {
            let site = legacy1201_aquifer_site(&factory, cell);
            assert_eq!(legacy1201_aquifer_site(&factory, cell), site);
            assert!((0..10).contains(&(site.x - cell.x * 16)));
            assert!((0..9).contains(&(site.y - cell.y * 12)));
            assert!((0..10).contains(&(site.z - cell.z * 16)));
        }
    }

    #[test]
    fn equal_distances_replace_earlier_cells_in_the_nearest_three() {
        let blank = Legacy1201AquiferSite {
            cell: BlockPos::default(), position: BlockPos::default(),
            squared_distance: i32::MAX,
        };
        let mut best = [blank; 3];
        for i in 1..=4 {
            insert_nearest(&mut best, Legacy1201AquiferSite {
                cell: BlockPos::new(i, 0, 0),
                position: BlockPos::new(i, 0, 0),
                squared_distance: 42,
            });
        }
        assert_eq!(best.map(|candidate| candidate.cell.x), [4, 3, 2]);
    }

    #[test]
    fn cell_search_never_depends_on_query_history() {
        let random = LegacyPositionalRandom::new(-123);
        let a = BlockPos::new(-1, -50, 16);
        let b = BlockPos::new(41, 200, -41);
        let baseline = legacy1201_nearest_aquifer_sites(&random, a);
        let _ = legacy1201_nearest_aquifer_sites(&random, b);
        assert_eq!(legacy1201_nearest_aquifer_sites(&random, a), baseline);
    }

    #[test]
    fn floodedness_uses_strict_thresholds_and_undersea_transition() {
        use Legacy1201Floodedness::{Dry, Flooded, Randomized};
        let pick = |y, submerged, noise| {
            legacy1201_aquifer_floodedness(y, 56, submerged, false, noise).unwrap()
        };
        // At y=64 the preliminary surface+8=64; if submerged, influence=1.
        assert_eq!(pick(64, true, -0.3), Randomized);
        assert_eq!(pick(64, true, -0.8), Dry);
        assert_eq!(pick(64, true, -0.299), Flooded);
        assert_eq!(pick(64, true, -0.799), Randomized);
        // At y=0 we are 64 blocks below that surface; normal thresholds.
        assert_eq!(pick(0, true, 0.8), Randomized);
        assert_eq!(pick(0, true, 0.4), Dry);
        assert_eq!(pick(0, true, 0.801), Flooded);
        assert_eq!(pick(0, false, 0.6), Randomized);
        assert_eq!(pick(0, false, 0.1), Dry);
    }

    #[test]
    fn deep_dark_is_dry_even_without_floodedness_sample() {
        assert_eq!(legacy1201_aquifer_floodedness(0, i32::MAX, true, true, f64::NAN)
            .unwrap(), Legacy1201Floodedness::Dry);
        assert!(legacy1201_aquifer_floodedness(0, 56, false, false, f64::NAN).is_err());
        assert!(legacy1201_aquifer_floodedness(0, i32::MAX, false, false, 0.2).is_err());
    }

    #[test]
    fn randomized_surface_quantizes_negative_noise_and_caps_at_surface() {
        let mut calls = Vec::new();
        let mut sample = |cell| { calls.push(cell); Ok(-0.01) };
        let point = BlockPos::new(-1, -1, -17);
        // -1/16=-1, -1/40=-1, -17/16=-2; centre Y is -20.
        assert_eq!(legacy1201_randomized_fluid_surface(point, 30, &mut sample).unwrap(), -23);
        assert_eq!(calls.as_slice(), &[BlockPos::new(-1, -1, -2)]);
        let mut positive = |_| Ok(0.91);
        assert_eq!(legacy1201_randomized_fluid_surface(BlockPos::new(16, 39, 16),
            25, &mut positive).unwrap(), 25); // centre 20 + 9, capped
        let mut zero = |_| Ok(0.0);
        assert_eq!(legacy1201_randomized_fluid_surface(BlockPos::new(16, 40, 16),
            100, &mut zero).unwrap(), 60);
    }

    #[test]
    fn randomized_surface_reports_missing_inputs_and_noise_errors() {
        let at = BlockPos::new(0, -64, 0);
        let mut no_call = |_| -> io::Result<f64> { panic!("must not sample with absent surface") };
        assert!(legacy1201_randomized_fluid_surface(at, i32::MAX, &mut no_call).is_err());
        let mut invalid = |_| Ok(f64::INFINITY);
        assert!(legacy1201_randomized_fluid_surface(at, 64, &mut invalid).is_err());
        let mut missing = |_| Err(io::Error::new(io::ErrorKind::WouldBlock, "missing router"));
        assert_eq!(legacy1201_randomized_fluid_surface(at, 64, &mut missing)
            .unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }

    #[test]
    fn fluid_type_lava_noise_only_in_deep_aquifers() {
        let global = Legacy1201FluidStatus { level: 63, state: WATER };
        let at = BlockPos::new(-1, -41, 64);
        let mut cells = Vec::new();
        let mut sampled = |cell| { cells.push(cell); Ok(-0.301) };
        assert_eq!(legacy1201_aquifer_fluid_type(at, -10, global, LAVA, &mut sampled).unwrap(), LAVA);
        assert_eq!(cells.as_slice(), &[BlockPos::new(-1, -2, 1)]);
        let mut boundary = |_| Ok(0.3);
        assert_eq!(legacy1201_aquifer_fluid_type(at, -10, global, LAVA, &mut boundary).unwrap(), WATER);
        let mut no_call = |_| -> io::Result<f64> { panic!("no noise needed") };
        assert_eq!(legacy1201_aquifer_fluid_type(at, -9, global, LAVA, &mut no_call).unwrap(), WATER);
        assert_eq!(legacy1201_aquifer_fluid_type(at, LEGACY1201_DRY_FLUID_LEVEL,
            global, LAVA, &mut no_call).unwrap(), WATER);
        assert_eq!(legacy1201_aquifer_fluid_type(at, -20,
            Legacy1201FluidStatus { level: -54, state: LAVA }, LAVA, &mut no_call).unwrap(), LAVA);
    }

    #[test]
    fn fluid_type_propagates_noise_errors() {
        let global = Legacy1201FluidStatus { level: 63, state: WATER };
        let at = BlockPos::new(0, -54, 0);
        let mut nan = |_| Ok(f64::NAN);
        assert!(legacy1201_aquifer_fluid_type(at, -11, global, LAVA, &mut nan).is_err());
        let mut missing = |_| Err(io::Error::new(io::ErrorKind::WouldBlock, "lava router"));
        assert_eq!(legacy1201_aquifer_fluid_type(at, -11, global, LAVA, &mut missing)
            .unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }

}
