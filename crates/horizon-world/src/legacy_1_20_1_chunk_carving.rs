//! Minecraft Java 1.20.1 `NoiseBasedChunkGenerator.applyCarvers` (`dhn.a`)
//! proto-chunk pipeline.
//!
//! Connects the 1.20.1 carver geometry (`Legacy1201Carver`), replacement policy
//! (`Legacy1201CarverPolicy`), `foundSurface` column tracking, `topMaterial`
//! dirt restoration, deferred fluid postprocessing (`Legacy1201CarverPostprocess`),
//! and `NoiseRouter`-backed aquifer (`Legacy1201ChunkAquifer`) across the full
//! 17×17 (`dx in -8..=8`, `dz in -8..=8`) origin-chunk neighbourhood.
//!
//! No 26.3 `apply_carving_mask`, `TerrainAquifer`, or 26.3 surface-rule worker
//! is used here.

use crate::{
    compression::invalid,
    legacy_1_20_1_aquifer_router::Legacy1201ChunkAquifer,
    legacy_1_20_1_carver_policy::Legacy1201CarverPolicy,
    legacy_1_20_1_carver_postprocess::Legacy1201CarverPostprocess,
    legacy_1_20_1_carver_world::Legacy1201CarverWorldOutput,
    legacy_1_20_1_world::{Legacy1201Level, LEGACY_1_20_1_DATA_VERSION},
    registry::{BlockRegistry, BlockStateId},
    worldgen::{anchor::WorldGenerationContext, carver::CarverError},
};
use horizon_core::{
    carving::{CarveError, CarvingMask},
    position::{BlockPos, ChunkPos},
    random::{LegacyRandom, PositionalRandom, RandomSource},
};
use std::{collections::HashSet, io, ptr};

/// Vanilla 1.20.1 `NoiseBasedChunkGenerator.applyCarvers` scans origin chunks
/// within `±8` chunks of the target proto-chunk (`17 × 17 = 289` chunks).
pub const LEGACY1201_CARVER_CHUNK_RANGE: i32 = 8;

/// `WorldgenRandom.setLargeFeatureSeed(long baseSeed, int chunkX, int chunkZ)`
/// (`dij.c(JII)V` in 1.20.1 bytecode).
///
/// Seeds `random` with `base_seed`, draws two `nextLong()` multipliers `a` and
/// `b`, and reseeds `random` with `((chunk_x as i64) * a) ^ ((chunk_z as i64) * b) ^ base_seed`.
pub fn legacy1201_set_large_feature_seed(
    random: &mut LegacyRandom,
    base_seed: i64,
    chunk_x: i32,
    chunk_z: i32,
) -> i64 {
    random.set_seed(base_seed);
    let a = random.next_long();
    let b = random.next_long();
    let mixed = i64::from(chunk_x)
        .wrapping_mul(a)
        ^ i64::from(chunk_z).wrapping_mul(b)
        ^ base_seed;
    random.set_seed(mixed);
    mixed
}

/// Diagnostic counters returned by one 1.20.1 proto-chunk carving step.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Legacy1201CarveStepSummary {
    pub origin_chunks_visited: usize,
    pub carvers_evaluated: usize,
    pub start_chunks_matched: usize,
    pub candidates_visited: usize,
    pub blocks_carved: usize,
    pub fluid_postprocessing_marked: usize,
}

fn map_carver_error(err: CarverError<io::Error>) -> io::Error {
    match err {
        CarverError::Geometry(CarveError::Output(inner)) => inner,
        other => invalid(format!("1.20.1 carver geometry error: {other}")),
    }
}

/// Execute one 1.20.1 `NoiseBasedChunkGenerator.applyCarvers` step (`AIR` or
/// `LIQUID`) on the target proto-chunk `target`.
///
/// - Iterates `dx in -8..=8` (outer) and `dz in -8..=8` (inner) around `target`.
/// - Queries `origin_carvers(origin)` for the configured carver policies of that
///   origin chunk's biome generation settings.
/// - For each `(index, policy)`:
///   1. Calls [`legacy1201_set_large_feature_seed`] with
///      `level_seed.wrapping_add(index as i64)`, `origin.x`, `origin.z`.
///   2. Tests `policy.carver().is_start_chunk(&mut random)`.
///   3. When matched, carves into `target` using the shared `mask`, duplicate-visit
///      set `seen`, `aquifer`, `top_material`, and deferred fluid queue.
/// - Flushes all recorded fluid postprocessing positions to `mark_fluid_postprocessing`
///   in first-write order at the end of the step.
#[allow(clippy::too_many_arguments)]
pub fn legacy1201_apply_chunk_carvers<'c, 'r, L, R, C, I, E, D, F, S, V, B, T, M>(
    level: &mut L,
    registry: &'r BlockRegistry,
    context: WorldGenerationContext,
    level_seed: i64,
    target: ChunkPos,
    mask: &mut CarvingMask,
    aquifer: &mut Legacy1201ChunkAquifer<'r, R>,
    mut origin_carvers: C,
    initial_density_without_jaggedness: &mut I,
    erosion: &mut E,
    depth: &mut D,
    floodedness: &mut F,
    spread: &mut S,
    lava_noise: &mut V,
    barrier: &mut B,
    top_material: &mut T,
    mark_fluid_postprocessing: M,
) -> io::Result<Legacy1201CarveStepSummary>
where
    'r: 'c,
    L: Legacy1201Level,
    R: PositionalRandom,
    C: FnMut(ChunkPos) -> io::Result<&'c [Legacy1201CarverPolicy<'c, 'r>]>,
    I: FnMut(BlockPos) -> io::Result<f64>,
    E: FnMut(BlockPos) -> io::Result<f64>,
    D: FnMut(BlockPos) -> io::Result<f64>,
    F: FnMut(BlockPos) -> io::Result<f64>,
    S: FnMut(BlockPos) -> io::Result<f64>,
    V: FnMut(BlockPos) -> io::Result<f64>,
    B: FnMut(BlockPos) -> io::Result<f64>,
    T: FnMut(&L, BlockPos) -> io::Result<Option<BlockStateId>>,
    M: FnMut(BlockPos) -> io::Result<()>,
{
    if level.collision_data_version() != LEGACY_1_20_1_DATA_VERSION
        || !ptr::eq(level.collision_block_registry(), registry)
        || registry.len() != 24_135
    {
        return Err(invalid(
            "1.20.1 apply_chunk_carvers level/registry mismatch",
        ));
    }
    let max_y = context
        .min_y
        .checked_add(context.height)
        .ok_or_else(|| invalid("1.20.1 apply_chunk_carvers vertical bounds overflow"))?;
    if context.min_y != level.min_y() || max_y != level.max_y() {
        return Err(invalid(
            "1.20.1 apply_chunk_carvers generation context does not match level bounds",
        ));
    }
    if let Legacy1201ChunkAquifer::Noise { aquifer, .. } = aquifer {
        if aquifer.target() != target {
            return Err(invalid("1.20.1 noise aquifer has wrong target chunk"));
        }
    }

    let mut random = LegacyRandom::new(0);
    let mut seen: HashSet<(i32, i32, i32)> = HashSet::new();
    let mut effects = Legacy1201CarverPostprocess::new(target);
    let mut summary = Legacy1201CarveStepSummary::default();

    for dx in -LEGACY1201_CARVER_CHUNK_RANGE..=LEGACY1201_CARVER_CHUNK_RANGE {
        let origin_x = target
            .x
            .checked_add(dx)
            .ok_or_else(|| invalid("1.20.1 carver origin chunk X overflow"))?;
        for dz in -LEGACY1201_CARVER_CHUNK_RANGE..=LEGACY1201_CARVER_CHUNK_RANGE {
            let origin_z = target
                .z
                .checked_add(dz)
                .ok_or_else(|| invalid("1.20.1 carver origin chunk Z overflow"))?;
            let origin = ChunkPos::new(origin_x, origin_z);
            summary.origin_chunks_visited += 1;

            let policies = origin_carvers(origin)?;
            for (index, policy) in policies.iter().enumerate() {
                if !ptr::eq(policy.registry(), registry) {
                    return Err(invalid(
                        "1.20.1 carver policy uses a different registry from the level",
                    ));
                }
                summary.carvers_evaluated += 1;
                let base_seed = level_seed.wrapping_add(index as i64);
                legacy1201_set_large_feature_seed(&mut random, base_seed, origin.x, origin.z);
                if !policy.carver().is_start_chunk(&mut random) {
                    continue;
                }
                summary.start_chunks_matched += 1;
                let (visited, applied) = {
                    let mut output = Legacy1201CarverWorldOutput::new(
                        level,
                        registry,
                        mask,
                        &mut seen,
                        target,
                        policy.debug(),
                        |level: &mut L, position, column| {
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
                    )?;
                    policy
                        .carver()
                        .carve_geometry(context, &mut random, target, origin, &mut output)
                        .map_err(map_carver_error)?;
                    (output.visited(), output.applied())
                };
                summary.candidates_visited += visited;
                summary.blocks_carved += applied;
            }
        }
    }

    summary.fluid_postprocessing_marked = effects.flush(mark_fluid_postprocessing)?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        legacy_1_20_1_aquifer::Legacy1201GlobalFluidPicker,
        legacy_1_20_1_aquifer_solver::Legacy1201NoiseAquifer,
        level::{LevelAccess, LevelEvent},
        nbt::Tag,
        worldgen::carver::legacy1201::Legacy1201Carver,
    };
    use horizon_core::random::LegacyPositionalRandom;
    use std::{
        collections::{BTreeMap, HashMap},
        sync::OnceLock,
    };

    const AIR: BlockStateId = BlockStateId(0);
    const WATER: BlockStateId = BlockStateId(1);
    const LAVA: BlockStateId = BlockStateId(2);
    const STONE: BlockStateId = BlockStateId(3);
    const GRASS: BlockStateId = BlockStateId(4);
    const DIRT: BlockStateId = BlockStateId(6);

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

    struct ProtoChunkLevel<'r> {
        registry: &'r BlockRegistry,
        target: ChunkPos,
        min_y: i32,
        max_y: i32,
        blocks: HashMap<BlockPos, BlockStateId>,
        random: LegacyRandom,
    }

    impl<'r> ProtoChunkLevel<'r> {
        fn new(registry: &'r BlockRegistry, target: ChunkPos, min_y: i32, max_y: i32) -> Self {
            Self {
                registry,
                target,
                min_y,
                max_y,
                blocks: HashMap::new(),
                random: LegacyRandom::new(0),
            }
        }
    }

    impl LevelAccess for ProtoChunkLevel<'_> {
        fn block_state(&self, pos: BlockPos) -> BlockStateId {
            self.blocks.get(&pos).copied().unwrap_or(
                if pos.y == 64 {
                    GRASS
                } else if (60..64).contains(&pos.y) {
                    DIRT
                } else if pos.y < 60 {
                    STONE
                } else {
                    AIR
                },
            )
        }

        fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
            if pos.chunk() != self.target || pos.y < self.min_y || pos.y >= self.max_y {
                return None;
            }
            let prev = self.block_state(pos);
            self.blocks.insert(pos, state);
            Some(prev)
        }

        fn is_loaded(&self, pos: BlockPos) -> bool {
            pos.chunk() == self.target
        }

        fn min_y(&self) -> i32 {
            self.min_y
        }

        fn max_y(&self) -> i32 {
            self.max_y
        }

        fn random(&mut self) -> &mut LegacyRandom {
            &mut self.random
        }

        fn game_time(&self) -> i64 {
            0
        }

        fn block_light(&self, _pos: BlockPos) -> i32 {
            0
        }

        fn sky_light(&self, _pos: BlockPos) -> i32 {
            15
        }

        fn emit(&mut self, _event: LevelEvent) {}
    }

    impl Legacy1201Level for ProtoChunkLevel<'_> {
        fn collision_data_version(&self) -> i32 {
            LEGACY_1_20_1_DATA_VERSION
        }

        fn collision_block_registry(&self) -> &BlockRegistry {
            self.registry
        }
    }

    #[test]
    fn large_feature_seed_matches_java_worldgen_random_bytecode() {
        let mut rng = LegacyRandom::new(0);
        let mixed = legacy1201_set_large_feature_seed(&mut rng, 123456789, -3, 7);
        let mut manual = LegacyRandom::new(123456789);
        let a = manual.next_long();
        let b = manual.next_long();
        let expected = (-3_i64).wrapping_mul(a) ^ 7_i64.wrapping_mul(b) ^ 123456789_i64;
        assert_eq!(mixed, expected);
        manual.set_seed(expected);
        assert_eq!(rng.next_long(), manual.next_long());
    }

    #[test]
    fn apply_chunk_carvers_visits_289_origin_chunks_and_flushes_postprocess() {
        let registry = registry_24135();
        let mut tags = BTreeMap::new();
        tags.insert(
            "minecraft:overworld_carver_replaceables".into(),
            vec![
                "minecraft:stone".into(),
                "minecraft:grass_block".into(),
                "minecraft:dirt".into(),
            ],
        );
        let carver_json = serde_json::json!({
            "type": "minecraft:cave",
            "config": {
                "probability": 1.0,
                "y": { "absolute": 56 },
                "yScale": 1.0,
                "lava_level": { "above_bottom": 8 },
                "replaceable": "#minecraft:overworld_carver_replaceables",
                "horizontal_radius_multiplier": 1.5,
                "vertical_radius_multiplier": 1.5,
                "floor_level": -0.7
            }
        })
        .to_string();
        let carver = Legacy1201Carver::from_json(&carver_json, registry, &tags).unwrap();
        let context = WorldGenerationContext {
            min_y: -64,
            height: 384,
            sea_level: 63,
        };
        let policy = Legacy1201CarverPolicy::new(&carver, registry, context).unwrap();
        let policies = [policy];

        let target = ChunkPos::new(1, 1);
        let mut level = ProtoChunkLevel::new(registry, target, -64, 320);
        let mut mask = CarvingMask::new(-63, 319).unwrap();
        let picker = Legacy1201GlobalFluidPicker::new(registry, 63, WATER).unwrap();
        let factory = LegacyPositionalRandom::new(777);
        let noise_aquifer =
            Legacy1201NoiseAquifer::new(picker, &factory, target, -64, 384).unwrap();
        let mut chunk_aquifer = Legacy1201ChunkAquifer::noise(noise_aquifer, 8).unwrap();

        let mut initial_density = |pos: BlockPos| Ok(if pos.y <= 32 { 0.8 } else { 0.0 });
        let mut erosion = |_: BlockPos| Ok(0.0);
        let mut depth = |_: BlockPos| Ok(0.0);
        let mut flood = |_: BlockPos| Ok(1.0);
        let mut spread = |_: BlockPos| Ok(0.0);
        let mut lava = |_: BlockPos| Ok(0.0);
        let mut barrier = |_: BlockPos| Ok(0.0);
        let mut top_material = |_: &ProtoChunkLevel<'_>, _: BlockPos| Ok(Some(GRASS));
        let mut marked_positions = Vec::new();

        let summary = legacy1201_apply_chunk_carvers(
            &mut level,
            registry,
            context,
            2,
            target,
            &mut mask,
            &mut chunk_aquifer,
            |origin| {
                if origin == target {
                    Ok(&policies[..])
                } else {
                    Ok(&[])
                }
            },
            &mut initial_density,
            &mut erosion,
            &mut depth,
            &mut flood,
            &mut spread,
            &mut lava,
            &mut barrier,
            &mut top_material,
            |pos| {
                marked_positions.push(pos);
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(summary.origin_chunks_visited, 289);
        assert_eq!(summary.carvers_evaluated, 1);
        assert_eq!(summary.start_chunks_matched, 1);
        assert!(summary.blocks_carved > 0);
        assert_eq!(summary.fluid_postprocessing_marked, marked_positions.len());
        let _ = (LAVA, STONE);
    }
}
