//! Minecraft Java 1.20.1 `NoiseRouter` and `WorldCarver` integration for the
//! 1.20.1 noise aquifer (`dgw$c`, `dho`, `coa`, `ebv`).
//!
//! Bridges the 1.5.19 `Legacy1201NoiseAquifer` to:
//! 1. `OverworldBiomeBuilder.isDeepDarkRegion(erosion, depth, pos)` (`coa.a`):
//!    exact single-precision-widened thresholds (`-0.225f32 as f64` and
//!    `0.9f32 as f64`) with short-circuiting so `depth` is never sampled when
//!    `erosion >= -0.225f32 as f64`.
//! 2. `NoiseChunk.preliminarySurfaceLevel(x, z)` (`dho.a(int, int)` /
//!    `dho.a(long)`): quart-block coordinate normalization (`(x >> 2) << 2`,
//!    `(z >> 2) << 2`), memoization by quart-aligned column, and top-down scan
//!    from `min_y + height` down to `min_y` (inclusive) in steps of
//!    `cell_height` testing `initial_density_without_jaggedness > 0.390625`
//!    (`25.0 / 64.0`), returning `i32::MAX` when no sample exceeds the cutoff.
//! 3. `WorldCarver.getCarveState` (`ebv`): evaluates the chunk's 1.20.1 aquifer
//!    (either disabled or noise-based) at `density = 0.0` for `Legacy1201CarverPolicy`
//!    and `output_with_legacy1201_postprocessing`.
//!
//! No 26.3 `AquiferSamplers`, `find_top_surface`, or `TerrainAquifer` state is
//! used here.

use crate::{
    compression::invalid,
    legacy_1_20_1_aquifer::Legacy1201DisabledAquifer,
    legacy_1_20_1_aquifer_solver::Legacy1201NoiseAquifer,
    legacy_1_20_1_carver_policy::{Legacy1201AquiferSample, Legacy1201CarverPolicy},
    legacy_1_20_1_carver_postprocess::Legacy1201CarverPostprocess,
    legacy_1_20_1_carver_world::{Legacy1201CarverColumn, Legacy1201CarverWorldOutput},
    legacy_1_20_1_world::Legacy1201Level,
    registry::BlockStateId,
};
use horizon_core::{
    carving::CarvingMask,
    position::{BlockPos, ChunkPos},
    random::PositionalRandom,
};
use std::{
    collections::{HashMap, HashSet},
    io,
};

/// `OverworldBiomeBuilder.isDeepDarkRegion` erosion cutoff in 1.20.1 bytecode:
/// `ldc2_w -0.22499999403953552d` (`(-0.225_f32) as f64`).
pub const LEGACY1201_DEEP_DARK_EROSION_MAX: f64 = (-0.225_f32) as f64;

/// `OverworldBiomeBuilder.isDeepDarkRegion` depth cutoff in 1.20.1 bytecode:
/// `ldc2_w 0.8999999761581421d` (`(0.9_f32) as f64`).
pub const LEGACY1201_DEEP_DARK_DEPTH_MIN: f64 = (0.9_f32) as f64;

/// `NoiseChunk.computePreliminarySurfaceLevel` density threshold in 1.20.1:
/// `ldc2_w 0.390625d` (`25.0 / 64.0`).
pub const LEGACY1201_PRELIMINARY_SURFACE_DENSITY_THRESHOLD: f64 = 0.390625;

/// `WorldCarver.getCarveState` evaluates `aquifer.computeSubstance` at `0.0D`.
pub const LEGACY1201_CARVER_AQUIFER_DENSITY: f64 = 0.0;

fn finite_sample(value: f64, label: &'static str) -> io::Result<f64> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid(format!("non-finite 1.20.1 {label} sample")))
    }
}

/// Normalize a world `(x, z)` block column to 1.20.1 `NoiseChunk` quart-aligned
/// block coordinates: `QuartPos.toBlock(QuartPos.fromBlock(coord))` (`coord & !3`).
#[inline]
pub const fn legacy1201_quart_block_column(x: i32, z: i32) -> (i32, i32) {
    ((x >> 2) << 2, (z >> 2) << 2)
}

/// Evaluate 1.20.1 `OverworldBiomeBuilder.isDeepDarkRegion(erosion, depth, pos)`
/// (`coa.a(dhd, dhd, dhd$b)`).
///
/// Preserves Java's short-circuit order: `sample_depth` is called only when
/// `sample_erosion(site) < (-0.225_f32 as f64)`.
pub fn legacy1201_is_deep_dark_region<E, D>(
    site: BlockPos,
    sample_erosion: &mut E,
    sample_depth: &mut D,
) -> io::Result<bool>
where
    E: FnMut(BlockPos) -> io::Result<f64>,
    D: FnMut(BlockPos) -> io::Result<f64>,
{
    let erosion = finite_sample(sample_erosion(site)?, "aquifer erosion")?;
    if !(erosion < LEGACY1201_DEEP_DARK_EROSION_MAX) {
        return Ok(false);
    }
    let depth = finite_sample(sample_depth(site)?, "aquifer depth")?;
    Ok(depth > LEGACY1201_DEEP_DARK_DEPTH_MIN)
}

/// Compute 1.20.1 `NoiseChunk.computePreliminarySurfaceLevel` (`dho.a(long)`)
/// for a column `(x, z)` using `NoiseRouter.initialDensityWithoutJaggedness`.
///
/// The input coordinates `(x, z)` are first normalized to quart-aligned block
/// coordinates `(qx, qz) = ((x >> 2) << 2, (z >> 2) << 2)`. The scan starts at
/// `min_y + height` (inclusive) and steps down by `cell_height` while
/// `y >= min_y`, returning the first `y` where `density > 0.390625`. If no
/// sample exceeds `0.390625`, returns `i32::MAX` (`2_147_483_647`).
pub fn legacy1201_compute_preliminary_surface_level<I>(
    x: i32,
    z: i32,
    min_y: i32,
    height: i32,
    cell_height: i32,
    initial_density_without_jaggedness: &mut I,
) -> io::Result<i32>
where
    I: FnMut(BlockPos) -> io::Result<f64>,
{
    if !(1..=4064).contains(&height) || !(1..=16).contains(&cell_height) || height % cell_height != 0
    {
        return Err(invalid(
            "invalid 1.20.1 preliminary surface height or cell_height",
        ));
    }
    let top_y = min_y
        .checked_add(height)
        .ok_or_else(|| invalid("1.20.1 preliminary surface top_y overflow"))?;
    let (qx, qz) = legacy1201_quart_block_column(x, z);
    let mut y = top_y;
    while y >= min_y {
        let density = finite_sample(
            initial_density_without_jaggedness(BlockPos::new(qx, y, qz))?,
            "initial_density_without_jaggedness",
        )?;
        if density > LEGACY1201_PRELIMINARY_SURFACE_DENSITY_THRESHOLD {
            return Ok(y);
        }
        match y.checked_sub(cell_height) {
            Some(next_y) => y = next_y,
            None => break,
        }
    }
    Ok(i32::MAX)
}

/// Chunk-scoped memo for 1.20.1 `NoiseChunk.preliminarySurfaceLevel(x, z)`,
/// keyed by quart-aligned block coordinates `((x >> 2) << 2, (z >> 2) << 2)`
/// just like `dho.l` (`Long2IntOpenHashMap` keyed by `ColumnPos.asLong(qx, qz)`).
///
/// Transient errors from `initial_density_without_jaggedness` are never cached.
pub struct Legacy1201QuartSurfaceCache {
    min_y: i32,
    height: i32,
    cell_height: i32,
    columns: HashMap<(i32, i32), i32>,
}

impl Legacy1201QuartSurfaceCache {
    pub fn new(min_y: i32, height: i32, cell_height: i32) -> io::Result<Self> {
        if !(1..=4064).contains(&height)
            || !(1..=16).contains(&cell_height)
            || height % cell_height != 0
            || min_y.checked_add(height).is_none()
        {
            return Err(invalid(
                "invalid 1.20.1 quart surface cache dimension bounds",
            ));
        }
        Ok(Self {
            min_y,
            height,
            cell_height,
            columns: HashMap::new(),
        })
    }

    pub fn min_y(&self) -> i32 {
        self.min_y
    }

    pub fn height(&self) -> i32 {
        self.height
    }

    pub fn cell_height(&self) -> i32 {
        self.cell_height
    }

    pub fn len(&self) -> usize {
        self.columns.len()
    }

    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }

    pub fn clear(&mut self) {
        self.columns.clear();
    }

    /// Resolve `NoiseChunk.preliminarySurfaceLevel(x, z)` at world block
    /// coordinates `(x, z)`, normalizing to quart-aligned block coordinates
    /// before checking or populating the memo table.
    pub fn get<I>(
        &mut self,
        x: i32,
        z: i32,
        initial_density_without_jaggedness: &mut I,
    ) -> io::Result<i32>
    where
        I: FnMut(BlockPos) -> io::Result<f64>,
    {
        let key = legacy1201_quart_block_column(x, z);
        if let Some(&cached) = self.columns.get(&key) {
            return Ok(cached);
        }
        let computed = legacy1201_compute_preliminary_surface_level(
            key.0,
            key.1,
            self.min_y,
            self.height,
            self.cell_height,
            initial_density_without_jaggedness,
        )?;
        self.columns.insert(key, computed);
        Ok(computed)
    }
}

/// Evaluate one 1.20.1 `Legacy1201NoiseAquifer` sample directly from the
/// 1.20.1 `NoiseRouter` density functions (`initial_density_without_jaggedness`,
/// `erosion`, `depth`, `fluid_level_floodedness`, `fluid_level_spread`, `lava`,
/// and `barrier`), using `surface_cache` for quart-aligned preliminary surface
/// memoization.
#[allow(clippy::too_many_arguments)]
pub fn legacy1201_sample_noise_aquifer_with_router<'r, R, I, E, D, F, S, L, B>(
    aquifer: &mut Legacy1201NoiseAquifer<'r, R>,
    surface_cache: &mut Legacy1201QuartSurfaceCache,
    position: BlockPos,
    density: f64,
    initial_density_without_jaggedness: &mut I,
    erosion: &mut E,
    depth: &mut D,
    floodedness: &mut F,
    spread: &mut S,
    lava_noise: &mut L,
    barrier: &mut B,
) -> io::Result<Legacy1201AquiferSample>
where
    R: PositionalRandom,
    I: FnMut(BlockPos) -> io::Result<f64>,
    E: FnMut(BlockPos) -> io::Result<f64>,
    D: FnMut(BlockPos) -> io::Result<f64>,
    F: FnMut(BlockPos) -> io::Result<f64>,
    S: FnMut(BlockPos) -> io::Result<f64>,
    L: FnMut(BlockPos) -> io::Result<f64>,
    B: FnMut(BlockPos) -> io::Result<f64>,
{
    let mut sample_surface = |x: i32, z: i32| {
        surface_cache.get(x, z, initial_density_without_jaggedness)
    };
    let mut deep_dark = |site: BlockPos| {
        legacy1201_is_deep_dark_region(site, erosion, depth)
    };
    aquifer.sample_at(
        position,
        density,
        &mut sample_surface,
        &mut deep_dark,
        floodedness,
        spread,
        lava_noise,
        barrier,
    )
}

/// Chunk-scoped 1.20.1 aquifer selector for `NoiseChunk` (`dho.m`): either
/// `Aquifer.createDisabled` (`aquifers_enabled == false`) or `Aquifer.create`
/// (`aquifers_enabled == true`).
pub enum Legacy1201ChunkAquifer<'r, R: PositionalRandom> {
    Disabled(Legacy1201DisabledAquifer<'r>),
    Noise {
        aquifer: Legacy1201NoiseAquifer<'r, R>,
        surface_cache: Legacy1201QuartSurfaceCache,
    },
}

impl<'r, R> Legacy1201ChunkAquifer<'r, R>
where
    R: PositionalRandom,
{
    pub const fn disabled(aquifer: Legacy1201DisabledAquifer<'r>) -> Self {
        Self::Disabled(aquifer)
    }

    pub fn noise(
        aquifer: Legacy1201NoiseAquifer<'r, R>,
        cell_height: i32,
    ) -> io::Result<Self> {
        let height = aquifer
            .max_y()
            .checked_sub(aquifer.min_y())
            .ok_or_else(|| invalid("1.20.1 aquifer height overflow"))?;
        let surface_cache = Legacy1201QuartSurfaceCache::new(
            aquifer.min_y(),
            height,
            cell_height,
        )?;
        Ok(Self::Noise {
            aquifer,
            surface_cache,
        })
    }

    /// Evaluate the 1.20.1 aquifer at `position` and `density`. When disabled,
    /// none of the router callbacks are invoked.
    #[allow(clippy::too_many_arguments)]
    pub fn sample_with_router<I, E, D, F, S, L, B>(
        &mut self,
        position: BlockPos,
        density: f64,
        initial_density_without_jaggedness: &mut I,
        erosion: &mut E,
        depth: &mut D,
        floodedness: &mut F,
        spread: &mut S,
        lava_noise: &mut L,
        barrier: &mut B,
    ) -> io::Result<Legacy1201AquiferSample>
    where
        I: FnMut(BlockPos) -> io::Result<f64>,
        E: FnMut(BlockPos) -> io::Result<f64>,
        D: FnMut(BlockPos) -> io::Result<f64>,
        F: FnMut(BlockPos) -> io::Result<f64>,
        S: FnMut(BlockPos) -> io::Result<f64>,
        L: FnMut(BlockPos) -> io::Result<f64>,
        B: FnMut(BlockPos) -> io::Result<f64>,
    {
        match self {
            Self::Disabled(disabled) => Ok(Legacy1201AquiferSample {
                state: disabled.compute_substance(position, density),
                schedule_fluid_update: false,
            }),
            Self::Noise {
                aquifer,
                surface_cache,
            } => legacy1201_sample_noise_aquifer_with_router(
                aquifer,
                surface_cache,
                position,
                density,
                initial_density_without_jaggedness,
                erosion,
                depth,
                floodedness,
                spread,
                lava_noise,
                barrier,
            ),
        }
    }

    /// Evaluate the 1.20.1 aquifer for `WorldCarver.getCarveState` at
    /// `density = 0.0` (`LEGACY1201_CARVER_AQUIFER_DENSITY`).
    #[allow(clippy::too_many_arguments)]
    pub fn sample_for_carver<I, E, D, F, S, L, B>(
        &mut self,
        position: BlockPos,
        initial_density_without_jaggedness: &mut I,
        erosion: &mut E,
        depth: &mut D,
        floodedness: &mut F,
        spread: &mut S,
        lava_noise: &mut L,
        barrier: &mut B,
    ) -> io::Result<Legacy1201AquiferSample>
    where
        I: FnMut(BlockPos) -> io::Result<f64>,
        E: FnMut(BlockPos) -> io::Result<f64>,
        D: FnMut(BlockPos) -> io::Result<f64>,
        F: FnMut(BlockPos) -> io::Result<f64>,
        S: FnMut(BlockPos) -> io::Result<f64>,
        L: FnMut(BlockPos) -> io::Result<f64>,
        B: FnMut(BlockPos) -> io::Result<f64>,
    {
        self.sample_with_router(
            position,
            LEGACY1201_CARVER_AQUIFER_DENSITY,
            initial_density_without_jaggedness,
            erosion,
            depth,
            floodedness,
            spread,
            lava_noise,
            barrier,
        )
    }
}

/// Wire a 1.20.1 carver to a `Legacy1201ChunkAquifer` (evaluating at
/// `density = 0.0`), the owning proto-chunk's deferred fluid postprocessing
/// queue, and the 1.20.1 `CarvingContext.topMaterial` callback.
#[allow(clippy::too_many_arguments)]
pub fn output_with_legacy1201_chunk_aquifer<'a, 'r, L, R, I, E, D, F, S, V, B, T>(
    level: &'a mut L,
    policy: &'a Legacy1201CarverPolicy<'_, 'r>,
    mask: &'a mut CarvingMask,
    seen: &'a mut HashSet<(i32, i32, i32)>,
    target: ChunkPos,
    aquifer: &'a mut Legacy1201ChunkAquifer<'r, R>,
    initial_density_without_jaggedness: &'a mut I,
    erosion: &'a mut E,
    depth: &'a mut D,
    floodedness: &'a mut F,
    spread: &'a mut S,
    lava_noise: &'a mut V,
    barrier: &'a mut B,
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
    R: PositionalRandom + 'a,
    I: FnMut(BlockPos) -> io::Result<f64> + 'a,
    E: FnMut(BlockPos) -> io::Result<f64> + 'a,
    D: FnMut(BlockPos) -> io::Result<f64> + 'a,
    F: FnMut(BlockPos) -> io::Result<f64> + 'a,
    S: FnMut(BlockPos) -> io::Result<f64> + 'a,
    V: FnMut(BlockPos) -> io::Result<f64> + 'a,
    B: FnMut(BlockPos) -> io::Result<f64> + 'a,
    T: FnMut(&L, BlockPos) -> io::Result<Option<BlockStateId>> + 'a,
{
    if effects.target() != target {
        return Err(invalid("1.20.1 postprocessing queue has wrong target chunk"));
    }
    if let Legacy1201ChunkAquifer::Noise { aquifer, .. } = aquifer {
        if aquifer.target() != target {
            return Err(invalid("1.20.1 noise aquifer has wrong target chunk"));
        }
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
            let mut sample_aquifer = |pos: BlockPos| {
                aquifer.sample_for_carver(
                    pos,
                    initial_density_without_jaggedness,
                    erosion,
                    depth,
                    floodedness,
                    spread,
                    lava_noise,
                    barrier,
                )
            };
            let mut after_write = |level: &mut L, write| {
                effects.after_write(level, registry, write, top_material)
            };
            policy.apply(
                level,
                position,
                column,
                &mut sample_aquifer,
                &mut after_write,
            )
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        legacy_1_20_1_aquifer::Legacy1201GlobalFluidPicker,
        nbt::Tag,
        registry::BlockRegistry,
    };
    use horizon_core::random::LegacyPositionalRandom;
    use std::sync::OnceLock;

    fn registry_24135() -> &'static BlockRegistry {
        static REGISTRY: OnceLock<BlockRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let mut states = Vec::with_capacity(24_135);
            for name in [
                "minecraft:air",
                "minecraft:water",
                "minecraft:lava",
                "minecraft:stone",
                "minecraft:grass_block",
                "minecraft:mycelium",
                "minecraft:dirt",
                "minecraft:cave_air",
                "minecraft:acacia_button",
                "minecraft:candle",
                "minecraft:orange_stained_glass",
                "minecraft:glass",
            ] {
                states.push(Tag::compound([("id", Tag::String(name.into()))]));
            }
            for idx in 12..24_135 {
                let s = idx.to_string();
                states.push(Tag::compound([
                    ("id", Tag::String("test:filler".into())),
                    (
                        "properties",
                        Tag::compound([("idx", Tag::String(s.as_str().into()))]),
                    ),
                ]));
            }
            BlockRegistry::from_nbt(&Tag::compound([
                ("DataVersion", Tag::Int(crate::DATA_VERSION)),
                ("states", Tag::List(states)),
                (
                    "defaults",
                    Tag::compound([
                        ("minecraft:air", Tag::Int(0)),
                        ("minecraft:water", Tag::Int(1)),
                        ("minecraft:lava", Tag::Int(2)),
                        ("minecraft:stone", Tag::Int(3)),
                        ("minecraft:grass_block", Tag::Int(4)),
                        ("minecraft:mycelium", Tag::Int(5)),
                        ("minecraft:dirt", Tag::Int(6)),
                        ("minecraft:cave_air", Tag::Int(7)),
                        ("minecraft:acacia_button", Tag::Int(8)),
                        ("minecraft:candle", Tag::Int(9)),
                        ("minecraft:orange_stained_glass", Tag::Int(10)),
                        ("minecraft:glass", Tag::Int(11)),
                        ("test:filler", Tag::Int(12)),
                    ]),
                ),
            ]))
            .unwrap()
        })
    }

    #[test]
    fn deep_dark_constants_match_f32_widened_bytecode_literals() {
        assert_eq!(
            LEGACY1201_DEEP_DARK_EROSION_MAX.to_bits(),
            (-0.225_f32 as f64).to_bits()
        );
        assert_eq!(
            LEGACY1201_DEEP_DARK_DEPTH_MIN.to_bits(),
            (0.9_f32 as f64).to_bits()
        );
        assert_eq!(
            LEGACY1201_PRELIMINARY_SURFACE_DENSITY_THRESHOLD.to_bits(),
            0.390625_f64.to_bits()
        );
    }

    #[test]
    fn deep_dark_short_circuits_depth_when_erosion_is_not_below_threshold() {
        let site = BlockPos::new(12, -24, -8);
        let mut no_depth = |_| -> io::Result<f64> {
            panic!("depth must not be sampled when erosion >= threshold")
        };
        let mut at_cutoff = |_| Ok(LEGACY1201_DEEP_DARK_EROSION_MAX);
        assert!(
            !legacy1201_is_deep_dark_region(site, &mut at_cutoff, &mut no_depth).unwrap()
        );

        let mut below_cutoff = |_| Ok(LEGACY1201_DEEP_DARK_EROSION_MAX - 1e-9);
        let mut shallow = |_| Ok(LEGACY1201_DEEP_DARK_DEPTH_MIN);
        assert!(
            !legacy1201_is_deep_dark_region(site, &mut below_cutoff, &mut shallow).unwrap()
        );

        let mut deep = |_| Ok(LEGACY1201_DEEP_DARK_DEPTH_MIN + 1e-9);
        assert!(
            legacy1201_is_deep_dark_region(site, &mut below_cutoff, &mut deep).unwrap()
        );
    }

    #[test]
    fn preliminary_surface_quantizes_quart_columns_and_scans_top_down() {
        let mut visited = Vec::new();
        let mut initial_density = |pos: BlockPos| {
            visited.push(pos);
            if pos.y == 48 {
                Ok(0.390625) // exact equality is not > 0.390625
            } else if pos.y == 40 {
                Ok(0.3906251)
            } else {
                Ok(0.0)
            }
        };
        let level = legacy1201_compute_preliminary_surface_level(
            -5,
            7,
            -64,
            128,
            8,
            &mut initial_density,
        )
        .unwrap();
        assert_eq!(level, 40);
        // -5 >> 2 == -2 -> -8; 7 >> 2 == 1 -> 4; scan starts at -64 + 128 = 64.
        assert_eq!(visited[0], BlockPos::new(-8, 64, 4));
        assert_eq!(visited[1], BlockPos::new(-8, 56, 4));
        assert_eq!(visited[2], BlockPos::new(-8, 48, 4));
        assert_eq!(visited[3], BlockPos::new(-8, 40, 4));
        assert_eq!(visited.len(), 4);
    }

    #[test]
    fn preliminary_surface_returns_i32_max_when_never_exceeding_threshold() {
        let mut count = 0;
        let mut initial_density = |_: BlockPos| {
            count += 1;
            Ok(0.390625)
        };
        let level = legacy1201_compute_preliminary_surface_level(
            0,
            0,
            -64,
            384,
            8,
            &mut initial_density,
        )
        .unwrap();
        assert_eq!(level, i32::MAX);
        // From 320 down to -64 inclusive in steps of 8 is 49 samples.
        assert_eq!(count, 49);
    }

    #[test]
    fn quart_surface_cache_shares_entries_across_same_four_by_four_column() {
        let mut cache = Legacy1201QuartSurfaceCache::new(-64, 384, 8).unwrap();
        let mut calls = 0;
        let mut initial_density = |pos: BlockPos| {
            calls += 1;
            Ok(if pos.y <= 64 { 0.5 } else { 0.0 })
        };
        for dx in 0..4 {
            for dz in 0..4 {
                assert_eq!(
                    cache.get(12 + dx, -8 + dz, &mut initial_density).unwrap(),
                    64
                );
            }
        }
        assert_eq!(cache.len(), 1);
        // 320 down to 64 inclusive in steps of 8 is 33 samples, run only once.
        assert_eq!(calls, 33);
    }

    #[test]
    fn chunk_aquifer_carver_adapter_samples_at_zero_density() {
        let registry = registry_24135();
        let water = registry.default_state("minecraft:water").unwrap();
        let picker = Legacy1201GlobalFluidPicker::new(registry, 63, water).unwrap();
        let factory = LegacyPositionalRandom::new(99);
        let noise_aquifer =
            Legacy1201NoiseAquifer::new(picker, &factory, ChunkPos::new(0, 0), -64, 384)
                .unwrap();
        let mut chunk_aquifer =
            Legacy1201ChunkAquifer::noise(noise_aquifer, 8).unwrap();

        let mut initial_density = |pos: BlockPos| Ok(if pos.y <= 32 { 0.6 } else { 0.0 });
        let mut erosion = |_: BlockPos| Ok(0.0);
        let mut depth = |_: BlockPos| -> io::Result<f64> {
            panic!("depth must not be sampled when erosion >= -0.225")
        };
        let mut flood = |_: BlockPos| Ok(0.25);
        let mut spread = |_: BlockPos| Ok(-0.2);
        let mut lava = |_: BlockPos| Ok(0.0);
        let mut barrier = |_: BlockPos| Ok(0.0);

        let sample = chunk_aquifer
            .sample_for_carver(
                BlockPos::new(4, 16, 4),
                &mut initial_density,
                &mut erosion,
                &mut depth,
                &mut flood,
                &mut spread,
                &mut lava,
                &mut barrier,
            )
            .unwrap();
        assert_eq!(sample.state, Some(water));
    }
}
