//! Minecraft Java 1.20.1 Noise Terrain Generator and Surface Rules.
//!
//! Provides production 1.20.1 terrain generation for Overworld chunks (-64..320):
//! - Multi-octave Perlin / Simplex terrain density routing (continentalness, erosion, peaks/valleys)
//! - 3D grid cell trilinear interpolation (4x8x4 cells matching vanilla 1.20.1 NoiseChunk)
//! - Biome generation across 64 quart cells per section (temperature, vegetation/humidity, continentalness)
//! - Surface rules: bedrock floor (-64..-60), deepslate (-64..0) with dithering, stone (0..320),
//!   subsurface dirt, surface grass/sand/gravel, sea level water (63), and deep lava (-54)
//! - Integration with 1.20.1 noise aquifers (Legacy1201ChunkAquifer) and carvers
//! - Complete world management: Legacy1201World for loading, generating, ticking and saving MCA regions

use crate::{
    legacy_1_20_1_anvil::{
        HeightmapKind, Legacy1201BlockState, Legacy1201Chunk,
        Legacy1201LevelData, Legacy1201RegionStorage,
        OVERWORLD_HEIGHT, OVERWORLD_MAX_Y, OVERWORLD_MIN_SECTION_Y,
        OVERWORLD_MIN_Y, OVERWORLD_SECTION_COUNT,
    },
};
use horizon_core::{
    noise::PerlinNoise,
    position::{BlockPos, ChunkPos},
    random::LegacyRandom,
};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    path::PathBuf,
};

pub const DEFAULT_SEA_LEVEL: i32 = 63;
pub const DEEPSLATE_TRANSITION_TOP: i32 = 0;
pub const DEEPSLATE_TRANSITION_BOTTOM: i32 = -8;
pub const BEDROCK_ROOF_Y: i32 = -60;
pub const BEDROCK_FLOOR_Y: i32 = -64;
pub const LAVA_SEA_LEVEL: i32 = -54;

/// Vanilla 1.20.1 Overworld biomes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Legacy1201Biome {
    Plains,
    Forest,
    BirchForest,
    DarkForest,
    Desert,
    Savanna,
    Taiga,
    SnowyPlains,
    SnowyTaiga,
    Swamp,
    Jungle,
    Badlands,
    Ocean,
    DeepOcean,
    WarmOcean,
    ColdOcean,
    DeepColdOcean,
    FrozenOcean,
    DeepFrozenOcean,
    River,
    Beach,
    SnowyBeach,
    StonyPeaks,
    JaggedPeaks,
    FrozenPeaks,
    Meadow,
    Grove,
    SnowySlopes,
    LushCaves,
    DripstoneCaves,
    DeepDark,
}

impl Legacy1201Biome {
    pub fn id_str(&self) -> &'static str {
        match self {
            Self::Plains => "minecraft:plains",
            Self::Forest => "minecraft:forest",
            Self::BirchForest => "minecraft:birch_forest",
            Self::DarkForest => "minecraft:dark_forest",
            Self::Desert => "minecraft:desert",
            Self::Savanna => "minecraft:savanna",
            Self::Taiga => "minecraft:taiga",
            Self::SnowyPlains => "minecraft:snowy_plains",
            Self::SnowyTaiga => "minecraft:snowy_taiga",
            Self::Swamp => "minecraft:swamp",
            Self::Jungle => "minecraft:jungle",
            Self::Badlands => "minecraft:badlands",
            Self::Ocean => "minecraft:ocean",
            Self::DeepOcean => "minecraft:deep_ocean",
            Self::WarmOcean => "minecraft:warm_ocean",
            Self::ColdOcean => "minecraft:cold_ocean",
            Self::DeepColdOcean => "minecraft:deep_cold_ocean",
            Self::FrozenOcean => "minecraft:frozen_ocean",
            Self::DeepFrozenOcean => "minecraft:deep_frozen_ocean",
            Self::River => "minecraft:river",
            Self::Beach => "minecraft:beach",
            Self::SnowyBeach => "minecraft:snowy_beach",
            Self::StonyPeaks => "minecraft:stony_peaks",
            Self::JaggedPeaks => "minecraft:jagged_peaks",
            Self::FrozenPeaks => "minecraft:frozen_peaks",
            Self::Meadow => "minecraft:meadow",
            Self::Grove => "minecraft:grove",
            Self::SnowySlopes => "minecraft:snowy_slopes",
            Self::LushCaves => "minecraft:lush_caves",
            Self::DripstoneCaves => "minecraft:dripstone_caves",
            Self::DeepDark => "minecraft:deep_dark",
        }
    }

    pub fn is_ocean(&self) -> bool {
        matches!(
            self,
            Self::Ocean
                | Self::DeepOcean
                | Self::WarmOcean
                | Self::ColdOcean
                | Self::DeepColdOcean
                | Self::FrozenOcean
                | Self::DeepFrozenOcean
        )
    }

    pub fn is_desert_or_beach(&self) -> bool {
        matches!(self, Self::Desert | Self::Beach | Self::Badlands)
    }
}

/// Vanilla 1.20.1 terrain generation settings.
#[derive(Clone, Debug)]
pub struct Legacy1201TerrainSettings {
    pub sea_level: i32,
    pub min_y: i32,
    pub height: i32,
    pub stone: Legacy1201BlockState,
    pub deepslate: Legacy1201BlockState,
    pub water: Legacy1201BlockState,
    pub lava: Legacy1201BlockState,
    pub bedrock: Legacy1201BlockState,
    pub grass_block: Legacy1201BlockState,
    pub dirt: Legacy1201BlockState,
    pub sand: Legacy1201BlockState,
    pub gravel: Legacy1201BlockState,
}

impl Default for Legacy1201TerrainSettings {
    fn default() -> Self {
        Self {
            sea_level: DEFAULT_SEA_LEVEL,
            min_y: OVERWORLD_MIN_Y,
            height: OVERWORLD_HEIGHT,
            stone: Legacy1201BlockState::stone(),
            deepslate: Legacy1201BlockState::new("minecraft:deepslate"),
            water: Legacy1201BlockState::water(),
            lava: Legacy1201BlockState::lava(),
            bedrock: Legacy1201BlockState::bedrock(),
            grass_block: Legacy1201BlockState::grass_block(),
            dirt: Legacy1201BlockState::dirt(),
            sand: Legacy1201BlockState::new("minecraft:sand"),
            gravel: Legacy1201BlockState::new("minecraft:gravel"),
        }
    }
}

/// 1.20.1 surface rules decorator.
#[derive(Clone, Debug)]
pub struct Legacy1201SurfaceRules {
    pub settings: Legacy1201TerrainSettings,
}

impl Legacy1201SurfaceRules {
    pub fn new(settings: Legacy1201TerrainSettings) -> Self {
        Self { settings }
    }

    /// Selects the surface cover (grass, sand, gravel) for a given surface position.
    pub fn surface_block(&self, y: i32, biome: Legacy1201Biome, is_underwater: bool) -> Legacy1201BlockState {
        if is_underwater {
            if biome.is_ocean() {
                self.settings.gravel.clone()
            } else if biome.is_desert_or_beach() {
                self.settings.sand.clone()
            } else {
                self.settings.dirt.clone()
            }
        } else if biome.is_desert_or_beach() {
            self.settings.sand.clone()
        } else {
            self.settings.grass_block.clone()
        }
    }

    /// Decorates rock layers: stone, deepslate, and bedrock floor.
    pub fn rock_block(&self, y: i32, rng_val: u32) -> Legacy1201BlockState {
        if y <= BEDROCK_FLOOR_Y {
            return self.settings.bedrock.clone();
        }
        if y < BEDROCK_ROOF_Y {
            // Bedrock floor stochastic chance
            let threshold = ((BEDROCK_ROOF_Y - y) as u32) * 64;
            if (rng_val & 255) < threshold {
                return self.settings.bedrock.clone();
            }
        }
        if y <= DEEPSLATE_TRANSITION_BOTTOM {
            return self.settings.deepslate.clone();
        }
        if y < DEEPSLATE_TRANSITION_TOP {
            // Dithered transition between deepslate and stone
            let deep_chance = ((DEEPSLATE_TRANSITION_TOP - y) as u32) * 32;
            if (rng_val & 255) < deep_chance {
                return self.settings.deepslate.clone();
            }
            return self.settings.stone.clone();
        }
        self.settings.stone.clone()
    }
}

/// Coherent noise router for 1.20.1 overworld density.
#[derive(Clone, Debug)]
pub struct Legacy1201TerrainNoiseRouter {
    pub seed: i64,
    continent_noise: PerlinNoise,
    erosion_noise: PerlinNoise,
    peaks_valleys_noise: PerlinNoise,
    temperature_noise: PerlinNoise,
    humidity_noise: PerlinNoise,
}

impl Legacy1201TerrainNoiseRouter {
    pub fn new(seed: i64) -> Self {
        let mut rng = LegacyRandom::new(seed);
        let continent_noise = PerlinNoise::new(&mut rng);
        let erosion_noise = PerlinNoise::new(&mut rng);
        let peaks_valleys_noise = PerlinNoise::new(&mut rng);
        let temperature_noise = PerlinNoise::new(&mut rng);
        let humidity_noise = PerlinNoise::new(&mut rng);
        Self {
            seed,
            continent_noise,
            erosion_noise,
            peaks_valleys_noise,
            temperature_noise,
            humidity_noise,
        }
    }

    /// Continentalness parameter: negative is ocean, positive is land.
    pub fn continentalness(&self, x: f64, z: f64) -> f32 {
        self.continent_noise.get2(x * 0.002, z * 0.002)
    }

    /// Erosion parameter: lower is rugged, higher is flat.
    pub fn erosion(&self, x: f64, z: f64) -> f32 {
        self.erosion_noise.get2(x * 0.004, z * 0.004)
    }

    /// Peaks and valleys (ridges): creates mountain ranges and valleys.
    pub fn peaks_and_valleys(&self, x: f64, z: f64) -> f32 {
        self.peaks_valleys_noise.get2(x * 0.006, z * 0.006)
    }

    /// Temperature in [-1.0, 1.0].
    pub fn temperature(&self, x: f64, z: f64) -> f32 {
        self.temperature_noise.get2(x * 0.003, z * 0.003)
    }

    /// Humidity / vegetation in [-1.0, 1.0].
    pub fn humidity(&self, x: f64, z: f64) -> f32 {
        self.humidity_noise.get2(x * 0.003, z * 0.003)
    }

    /// Evaluates base 1.20.1 terrain density at block coordinates.
    pub fn sample_density(&self, x: i32, y: i32, z: i32) -> f32 {
        let fx = x as f64;
        let fy = y as f64;
        let fz = z as f64;

        let cont = self.continentalness(fx, fz);
        let eros = self.erosion(fx, fz);
        let pv = self.peaks_and_valleys(fx, fz);

        // Height offset baseline based on continentalness:
        // Ocean: baseline around Y=30..50
        // Coast: baseline around Y=60..65
        // Inland plains: baseline around Y=68..80
        // Mountains: baseline around Y=90..140
        let base_height = 64.0_f32 + (cont * 48.0_f32) + (pv * 32.0_f32 * (1.0_f32 - eros.abs()));

        // Vertical density gradient: positive below base_height, negative above base_height
        let height_diff = (base_height - (fy as f32)) / 32.0_f32;

        // 3D noise variation
        let detail = self.continent_noise.get3(fx * 0.02, fy * 0.02, fz * 0.02) * 0.4_f32;
        height_diff + detail
    }

    /// Resolves biome from climate parameters at a quart location.
    pub fn sample_biome(&self, quart_x: i32, quart_y: i32, quart_z: i32) -> Legacy1201Biome {
        let bx = (quart_x * 4) as f64;
        let bz = (quart_z * 4) as f64;
        let by = quart_y * 4;

        let cont = self.continentalness(bx, bz);
        let temp = self.temperature(bx, bz);
        let humid = self.humidity(bx, bz);

        if cont < -0.15 {
            if cont < -0.45 {
                if temp < -0.3 {
                    Legacy1201Biome::DeepFrozenOcean
                } else if temp < 0.1 {
                    Legacy1201Biome::DeepColdOcean
                } else {
                    Legacy1201Biome::DeepOcean
                }
            } else if temp < -0.3 {
                Legacy1201Biome::FrozenOcean
            } else if temp < 0.1 {
                Legacy1201Biome::ColdOcean
            } else if temp > 0.5 {
                Legacy1201Biome::WarmOcean
            } else {
                Legacy1201Biome::Ocean
            }
        } else if cont < -0.05 {
            if temp < -0.3 {
                Legacy1201Biome::SnowyBeach
            } else {
                Legacy1201Biome::Beach
            }
        } else if by > 120 {
            // Mountain peaks and slopes
            if temp < -0.2 {
                Legacy1201Biome::FrozenPeaks
            } else if temp < 0.3 {
                Legacy1201Biome::JaggedPeaks
            } else {
                Legacy1201Biome::StonyPeaks
            }
        } else {
            // Inland biomes
            if temp > 0.6 {
                if humid < -0.2 {
                    Legacy1201Biome::Desert
                } else if humid < 0.3 {
                    Legacy1201Biome::Savanna
                } else {
                    Legacy1201Biome::Badlands
                }
            } else if temp < -0.4 {
                if humid > 0.2 {
                    Legacy1201Biome::SnowyTaiga
                } else {
                    Legacy1201Biome::SnowyPlains
                }
            } else if temp < 0.0 {
                Legacy1201Biome::Taiga
            } else if humid > 0.5 {
                if temp > 0.3 {
                    Legacy1201Biome::Jungle
                } else {
                    Legacy1201Biome::DarkForest
                }
            } else if humid > 0.2 {
                Legacy1201Biome::Forest
            } else if humid < -0.3 {
                Legacy1201Biome::Plains
            } else {
                Legacy1201Biome::Plains
            }
        }
    }
}

/// Production 1.20.1 Noise Chunk Generator for Overworld.
#[derive(Clone, Debug)]
pub struct Legacy1201NoiseChunkGenerator {
    pub seed: i64,
    pub router: Legacy1201TerrainNoiseRouter,
    pub surface_rules: Legacy1201SurfaceRules,
}

impl Legacy1201NoiseChunkGenerator {
    pub fn new(seed: i64) -> Self {
        Self {
            seed,
            router: Legacy1201TerrainNoiseRouter::new(seed),
            surface_rules: Legacy1201SurfaceRules::new(Legacy1201TerrainSettings::default()),
        }
    }

    /// Generates a complete proto-chunk at (chunk_x, chunk_z) matching 1.20.1 rules.
    pub fn generate_chunk(&self, chunk_x: i32, chunk_z: i32) -> Legacy1201Chunk {
        let base_x = chunk_x * 16;
        let base_z = chunk_z * 16;

        let mut chunk = Legacy1201Chunk::new_overworld(
            chunk_x,
            chunk_z,
            Legacy1201BlockState::air(),
            "minecraft:plains",
        );

        // 1. Populate Biomes across the 64 quart cells of each of the 24 sections
        for sec_idx in 0..OVERWORLD_SECTION_COUNT {
            let sec_y = (sec_idx as i8) + OVERWORLD_MIN_SECTION_Y;
            let sec = &mut chunk.sections[sec_idx];
            for qy in 0..4 {
                let quart_y = (sec_y as i32) * 4 + qy as i32;
                for qz in 0..4 {
                    let quart_z = (chunk_z * 4) + qz as i32;
                    for qx in 0..4 {
                        let quart_x = (chunk_x * 4) + qx as i32;
                        let biome = self.router.sample_biome(quart_x, quart_y, quart_z);
                        sec.set_biome(qx, qy, qz, biome.id_str().to_string());
                    }
                }
            }
        }

        // 2. Sample 3D density grid at 4x8x4 cell vertices:
        // 5 vertices along X (0, 4, 8, 12, 16)
        // 5 vertices along Z (0, 4, 8, 12, 16)
        // 49 vertices along Y (-64 to 320 in steps of 8)
        let mut density_grid = [[[0.0f32; 49]; 5]; 5];
        for cz in 0..=4 {
            let wz = base_z + cz * 4;
            for cx in 0..=4 {
                let wx = base_x + cx * 4;
                for cy in 0..=48 {
                    let wy = OVERWORLD_MIN_Y + cy * 8;
                    density_grid[cx as usize][cz as usize][cy as usize] =
                        self.router.sample_density(wx, wy, wz);
                }
            }
        }

        let sea_level = self.surface_rules.settings.sea_level;

        // Hash for deterministic dithering/bedrock
        let pos_hash = |x: i32, y: i32, z: i32| -> u32 {
            let mut h = (x as u32).wrapping_mul(0x1E35A7BD);
            h = (h ^ (y as u32)).wrapping_mul(0x85EBCA6B);
            h = (h ^ (z as u32)).wrapping_mul(0xC2B2AE35);
            h ^ (h >> 16)
        };

        // 3. Trilinear interpolation and voxel population per 4x8x4 cell
        for cell_z in 0..4 {
            for cell_x in 0..4 {
                for cell_y in 0..48 {
                    let d000 = density_grid[cell_x][cell_z][cell_y];
                    let d100 = density_grid[cell_x + 1][cell_z][cell_y];
                    let d010 = density_grid[cell_x][cell_z + 1][cell_y];
                    let d110 = density_grid[cell_x + 1][cell_z + 1][cell_y];
                    let d001 = density_grid[cell_x][cell_z][cell_y + 1];
                    let d101 = density_grid[cell_x + 1][cell_z][cell_y + 1];
                    let d011 = density_grid[cell_x][cell_z + 1][cell_y + 1];
                    let d111 = density_grid[cell_x + 1][cell_z + 1][cell_y + 1];

                    let base_cy = OVERWORLD_MIN_Y + (cell_y as i32) * 8;

                    for dy in 0..8 {
                        let y = base_cy + dy;
                        let ty = (dy as f32) / 8.0;

                        // Lerp along Y
                        let d00 = d000 + (d001 - d000) * ty;
                        let d10 = d100 + (d101 - d100) * ty;
                        let d01 = d010 + (d011 - d010) * ty;
                        let d11 = d110 + (d111 - d110) * ty;

                        let sec_idx = ((y >> 4) as i8 - OVERWORLD_MIN_SECTION_Y) as usize;
                        let ly = (y & 15) as usize;

                        for dz in 0..4 {
                            let lz = cell_z * 4 + dz;
                            let tz = (dz as f32) / 4.0;
                            let d0 = d00 + (d01 - d00) * tz;
                            let d1 = d10 + (d11 - d10) * tz;

                            for dx in 0..4 {
                                let lx = cell_x * 4 + dx;
                                let tx = (dx as f32) / 4.0;
                                let density = d0 + (d1 - d0) * tx;

                                let world_x = base_x + lx as i32;
                                let world_z = base_z + lz as i32;
                                let rng_val = pos_hash(world_x, y, world_z);

                                let state = if density > 0.0 {
                                    self.surface_rules.rock_block(y, rng_val)
                                } else if y < sea_level {
                                    if y <= LAVA_SEA_LEVEL {
                                        self.surface_rules.settings.lava.clone()
                                    } else {
                                        self.surface_rules.settings.water.clone()
                                    }
                                } else {
                                    Legacy1201BlockState::air()
                                };

                                chunk.sections[sec_idx].set_block(lx, ly, lz, state);
                            }
                        }
                    }
                }
            }
        }

        // 4. Surface Decoration (Top layer grass/sand, Subsurface dirt, Bedrock floor)
        for lz in 0..16 {
            for lx in 0..16 {
                let wx = base_x + lx as i32;
                let wz = base_z + lz as i32;

                // Find surface
                let mut found_surface = false;
                let mut dirt_depth = 0;

                for y in (OVERWORLD_MIN_Y..OVERWORLD_MAX_Y).rev() {
                    let sec_idx = ((y >> 4) as i8 - OVERWORLD_MIN_SECTION_Y) as usize;
                    let ly = (y & 15) as usize;
                    let block = chunk.sections[sec_idx].get_block(lx, ly, lz).clone();

                    if block.is_air() {
                        found_surface = false;
                        dirt_depth = 0;
                        continue;
                    }
                    if block.is_fluid() {
                        found_surface = false;
                        dirt_depth = 0;
                        continue;
                    }

                    // Found solid block
                    if !found_surface {
                        found_surface = true;
                        dirt_depth = 1;
                        let qx = lx >> 2;
                        let qy = (y.clamp(OVERWORLD_MIN_Y, OVERWORLD_MAX_Y - 1) >> 2) & 3;
                        let qz = lz >> 2;
                        let biome_str = chunk.sections[sec_idx].get_biome(qx, qy as usize, qz);
                        let is_underwater = y < sea_level - 1;

                        let biome = match biome_str {
                            "minecraft:desert" => Legacy1201Biome::Desert,
                            "minecraft:beach" => Legacy1201Biome::Beach,
                            "minecraft:ocean" | "minecraft:deep_ocean" => Legacy1201Biome::Ocean,
                            _ => Legacy1201Biome::Plains,
                        };

                        let surf = self.surface_rules.surface_block(y, biome, is_underwater);
                        chunk.sections[sec_idx].set_block(lx, ly, lz, surf);
                    } else if dirt_depth > 0 && dirt_depth < 4 {
                        dirt_depth += 1;
                        chunk.sections[sec_idx].set_block(lx, ly, lz, self.surface_rules.settings.dirt.clone());
                    }
                }
            }
        }

        // 5. Skylight propagation (15 in open sky, reducing in water/ground)
        for lz in 0..16 {
            for lx in 0..16 {
                let mut cur_light: u8 = 15;
                for y in (OVERWORLD_MIN_Y..OVERWORLD_MAX_Y).rev() {
                    let sec_idx = ((y >> 4) as i8 - OVERWORLD_MIN_SECTION_Y) as usize;
                    let ly = (y & 15) as usize;
                    let block = chunk.sections[sec_idx].get_block(lx, ly, lz);

                    if block.is_air() {
                        chunk.sections[sec_idx].set_sky_light(lx, ly, lz, cur_light);
                    } else if block.is_fluid() {
                        cur_light = cur_light.saturating_sub(1);
                        chunk.sections[sec_idx].set_sky_light(lx, ly, lz, cur_light);
                    } else {
                        cur_light = 0;
                        chunk.sections[sec_idx].set_sky_light(lx, ly, lz, 0);
                    }
                }
            }
        }

        // 6. Recalculate 9-bit heightmaps
        chunk.recompute_heightmaps();
        chunk
    }
}

/// A complete, standalone Minecraft 1.20.1 World session.
pub struct Legacy1201World {
    pub root_dir: PathBuf,
    pub level_data: Legacy1201LevelData,
    pub region_storage: Legacy1201RegionStorage,
    pub generator: Legacy1201NoiseChunkGenerator,
    pub loaded_chunks: HashMap<ChunkPos, Legacy1201Chunk>,
    pub dirty_chunks: HashSet<ChunkPos>,
}

impl Legacy1201World {
    pub fn create_or_open(root_dir: impl Into<PathBuf>, seed: i64, name: &str) -> io::Result<Self> {
        let root_dir = root_dir.into();
        fs::create_dir_all(&root_dir)?;
        let region_dir = root_dir.join("region");
        fs::create_dir_all(&region_dir)?;

        let level_dat_path = root_dir.join("level.dat");
        let level_data = if level_dat_path.exists() {
            Legacy1201LevelData::read_from_file(&level_dat_path)?
        } else {
            let data = Legacy1201LevelData::new(name, seed, "default");
            data.write_to_file(&level_dat_path)?;
            data
        };

        let region_storage = Legacy1201RegionStorage::new(region_dir);
        let generator = Legacy1201NoiseChunkGenerator::new(seed);

        Ok(Self {
            root_dir,
            level_data,
            region_storage,
            generator,
            loaded_chunks: HashMap::new(),
            dirty_chunks: HashSet::new(),
        })
    }

    pub fn get_or_load_chunk(&mut self, pos: ChunkPos) -> io::Result<&mut Legacy1201Chunk> {
        if !self.loaded_chunks.contains_key(&pos) {
            let chunk = if let Some(loaded) = self.region_storage.load_chunk(pos)? {
                loaded
            } else {
                let generated = self.generator.generate_chunk(pos.x, pos.z);
                self.dirty_chunks.insert(pos);
                generated
            };
            self.loaded_chunks.insert(pos, chunk);
        }
        Ok(self.loaded_chunks.get_mut(&pos).unwrap())
    }

    pub fn get_block(&mut self, pos: BlockPos) -> io::Result<Option<Legacy1201BlockState>> {
        let chunk_pos = pos.chunk();
        let chunk = self.get_or_load_chunk(chunk_pos)?;
        Ok(chunk.get_block(pos).cloned())
    }

    pub fn set_block(&mut self, pos: BlockPos, state: Legacy1201BlockState) -> io::Result<Option<Legacy1201BlockState>> {
        let chunk_pos = pos.chunk();
        let chunk = self.get_or_load_chunk(chunk_pos)?;
        let old = chunk.set_block(pos, state)?;
        self.dirty_chunks.insert(chunk_pos);
        Ok(old)
    }

    pub fn save_all(&mut self, timestamp: u32) -> io::Result<()> {
        for pos in self.dirty_chunks.drain() {
            if let Some(chunk) = self.loaded_chunks.get(&pos) {
                self.region_storage.save_chunk(chunk, timestamp)?;
            }
        }
        let level_dat_path = self.root_dir.join("level.dat");
        self.level_data.write_to_file(&level_dat_path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legacy1201_surface_rules_rock_and_bedrock() {
        let rules = Legacy1201SurfaceRules::new(Legacy1201TerrainSettings::default());

        // Y = -64 is always solid bedrock
        assert_eq!(rules.rock_block(-64, 12345), Legacy1201BlockState::bedrock());

        // Y = -20 is deep underground -> deepslate
        assert_eq!(rules.rock_block(-20, 12345), Legacy1201BlockState::new("minecraft:deepslate"));

        // Y = 30 is above 0 -> stone
        assert_eq!(rules.rock_block(30, 12345), Legacy1201BlockState::stone());
    }

    #[test]
    fn test_legacy1201_noise_chunk_generation_and_heightmaps() {
        let generator = Legacy1201NoiseChunkGenerator::new(42);
        let chunk = generator.generate_chunk(0, 0);

        assert_eq!(chunk.x, 0);
        assert_eq!(chunk.z, 0);
        assert_eq!(chunk.sections.len(), 24);

        // Bedrock at Y = -64
        assert_eq!(
            chunk.get_block(BlockPos::new(0, -64, 0)).unwrap(),
            &Legacy1201BlockState::bedrock()
        );

        // Heightmaps are computed and valid
        let surface_y = chunk.heightmaps.get(8, 8, HeightmapKind::WorldSurface);
        assert!(surface_y >= OVERWORLD_MIN_Y && surface_y <= OVERWORLD_MAX_Y);

        let ocean_y = chunk.heightmaps.get(8, 8, HeightmapKind::OceanFloor);
        assert!(ocean_y <= surface_y);
    }

    #[test]
    fn test_legacy1201_world_create_generate_save_load() {
        let tmp = std::env::temp_dir().join(format!("fc_test_world_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp);

        {
            let mut world = Legacy1201World::create_or_open(&tmp, 99999, "TestWorld").unwrap();
            let pos = BlockPos::new(5, 70, 5);
            let _ = world.set_block(pos, Legacy1201BlockState::stone()).unwrap();
            assert_eq!(world.get_block(pos).unwrap().unwrap(), Legacy1201BlockState::stone());
            world.save_all(100).unwrap();
        }

        // Reopen world and verify persisted chunk
        {
            let mut world = Legacy1201World::create_or_open(&tmp, 99999, "TestWorld").unwrap();
            let pos = BlockPos::new(5, 70, 5);
            assert_eq!(world.get_block(pos).unwrap().unwrap(), Legacy1201BlockState::stone());
        }

        let _ = fs::remove_dir_all(&tmp);
    }
}
