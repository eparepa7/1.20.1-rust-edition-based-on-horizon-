//! Minecraft Java 1.20.1 `NoiseBasedAquifer` block-substance solver, with
//! Horizon 26.3 flat-grid and 12-cell sliding-window optimizations preserved.
//!
//! This module connects the 1.5.9–1.5.18 aquifer primitives (3-nearest site
//! search, boundary similarity weights, 13-column preliminary-surface survey,
//! cell fluid-status calculation, and lazy barrier pressure) into the 1.20.1
//! per-block `computeSubstance` and `shouldScheduleFluidUpdate` decision.
//!
//! Version boundary and performance guarantees:
//! - Uses the 1.20.1 **three-centre** algorithm (`dgw$c`), never the 1.21.2+ /
//!   26.3 fourth-centre fluid-schedule check.
//! - Uses 1.20.1 `FLOWING_UPDATE_SIMILARITY = similarity(100, 144) = -0.76`:
//!   when `sim12 <= 0.0`, `should_schedule_fluid_update` is `sim12 >= -0.76`
//!   without querying the second cell's fluid status.
//! - Preserves the 26.3 Rust performance architecture: preallocated flat chunk
//!   arrays (`locations`, `points`, `statuses`) and a 12-slot register window
//!   (`Cells`) that reuses horizontal squared distances within a column and
//!   shifts 8 of 12 cells in-place on downward vertical traversal.
//! - Never substitutes 26.3 noise routers, 26.3 block state IDs, or a disabled
//!   aquifer fallback when inputs fail.

use crate::{
    compression::invalid,
    legacy_1_20_1_aquifer::{
        legacy1201_aquifer_anchor, legacy1201_aquifer_site,
        Legacy1201FluidStatus, Legacy1201GlobalFluidPicker,
    },
    legacy_1_20_1_aquifer_boundary::{
        legacy1201_aquifer_similarity, Legacy1201AquiferCellCache,
    },
    legacy_1_20_1_aquifer_pressure::{
        legacy1201_pair_pressure, Legacy1201BarrierNoise,
    },
    legacy_1_20_1_aquifer_status::legacy1201_compute_aquifer_cell_status,
    legacy_1_20_1_carver_policy::Legacy1201AquiferSample,
    legacy_1_20_1_world::Legacy1201Level,
    registry::BlockStateId,
};
use horizon_core::{
    position::{BlockPos, ChunkPos},
    random::PositionalRandom,
};
use std::{collections::HashMap, io};

/// Java 1.20.1 `NoiseBasedAquifer.FLOWING_UPDATE_SIMILARITY`:
/// `similarity(Mth.square(10), Mth.square(12))` = `1.0 - (144 - 100) / 25.0`.
pub const LEGACY1201_FLOWING_UPDATE_SIMILARITY: f64 = 1.0 - (144.0 - 100.0) / 25.0;

/// Resolve one block's 1.20.1 aquifer substance and fluid-update flag using
/// an explicit [`Legacy1201AquiferCellCache`].
///
/// Follows the exact 1.20.1 evaluation order:
/// 1. Positive density returns `None` (`schedule_fluid_update = false`) with
///    zero cell/status/barrier queries.
/// 2. Global lava at `position` returns `Some(lava)` (`schedule_fluid_update = false`)
///    with zero cell/status/barrier queries.
/// 3. Finds the three nearest aquifer sites and lazily queries ONLY `sites[0]`.
/// 4. If `sim12 <= 0.0`, returns `status_0.at(y, air)` with
///    `schedule_fluid_update = sim12 >= LEGACY1201_FLOWING_UPDATE_SIMILARITY`
///    without querying `sites[1]` or `sites[2]`.
/// 5. If the candidate state is water and the global picker at `y - 1` is lava,
///    returns `Some(water)` (`schedule_fluid_update = true`) without querying
///    `sites[1]`, `sites[2]` or barrier noise.
/// 6. Queries `sites[1]` and checks pair `(0, 1)` pressure; if solid, exits
///    before querying `sites[2]`.
/// 7. Queries `sites[2]` (matching 1.20.1 bytecode order before `sim13 > 0.0`),
///    checks pairs `(0, 2)` and `(1, 2)`, and otherwise returns `Some(candidate)`
///    with `schedule_fluid_update = true`.
pub fn legacy1201_resolve_aquifer_substance<R, S, B>(
    position: BlockPos,
    density: f64,
    picker: &Legacy1201GlobalFluidPicker<'_>,
    water_state: BlockStateId,
    cache: &mut Legacy1201AquiferCellCache<'_, R>,
    sample_status: &mut S,
    sample_barrier: &mut B,
) -> io::Result<Legacy1201AquiferSample>
where
    R: PositionalRandom,
    S: FnMut(BlockPos) -> io::Result<Legacy1201FluidStatus>,
    B: FnMut(BlockPos) -> io::Result<f64>,
{
    if !density.is_finite() {
        return Err(invalid("non-finite 1.20.1 aquifer terrain density"));
    }
    if picker.registry().state(water_state).is_none() {
        return Err(invalid("1.20.1 aquifer water state outside registry"));
    }
    if density > 0.0 {
        return Ok(Legacy1201AquiferSample {
            state: None,
            schedule_fluid_update: false,
        });
    }

    let air = picker.air();
    let lava = picker.lava();
    let global = picker.compute(position);
    if global.at(position.y, air) == lava {
        return Ok(Legacy1201AquiferSample {
            state: Some(lava),
            schedule_fluid_update: false,
        });
    }

    let sites = cache.nearest(position);
    let status_0 = cache.status_at(sites[0], sample_status)?;
    let sim12 = legacy1201_aquifer_similarity(
        sites[0].squared_distance,
        sites[1].squared_distance,
    );
    let candidate = status_0.at(position.y, air);
    if sim12 <= 0.0 {
        return Ok(Legacy1201AquiferSample {
            state: Some(candidate),
            schedule_fluid_update: sim12 >= LEGACY1201_FLOWING_UPDATE_SIMILARITY,
        });
    }

    if candidate == water_state {
        let below_y = position
            .y
            .checked_sub(1)
            .ok_or_else(|| invalid("1.20.1 aquifer y underflow"))?;
        let below = BlockPos::new(position.x, below_y, position.z);
        if picker.compute(below).at(below_y, air) == lava {
            return Ok(Legacy1201AquiferSample {
                state: Some(candidate),
                schedule_fluid_update: true,
            });
        }
    }

    let mut barrier = Legacy1201BarrierNoise::new(position, sample_barrier);
    let status_1 = cache.status_at(sites[1], sample_status)?;
    let p12 = sim12
        * legacy1201_pair_pressure(
            position.y,
            status_0,
            status_1,
            air,
            water_state,
            lava,
            &mut barrier,
        )?;
    if density + p12 > 0.0 {
        return Ok(Legacy1201AquiferSample {
            state: None,
            schedule_fluid_update: false,
        });
    }

    let status_2 = cache.status_at(sites[2], sample_status)?;
    let sim13 = legacy1201_aquifer_similarity(
        sites[0].squared_distance,
        sites[2].squared_distance,
    );
    if sim13 > 0.0 {
        let p13 = sim12
            * sim13
            * legacy1201_pair_pressure(
                position.y,
                status_0,
                status_2,
                air,
                water_state,
                lava,
                &mut barrier,
            )?;
        if density + p13 > 0.0 {
            return Ok(Legacy1201AquiferSample {
                state: None,
                schedule_fluid_update: false,
            });
        }
    }

    let sim23 = legacy1201_aquifer_similarity(
        sites[1].squared_distance,
        sites[2].squared_distance,
    );
    if sim23 > 0.0 {
        let p23 = sim12
            * sim23
            * legacy1201_pair_pressure(
                position.y,
                status_1,
                status_2,
                air,
                water_state,
                lava,
                &mut barrier,
            )?;
        if density + p23 > 0.0 {
            return Ok(Legacy1201AquiferSample {
                state: None,
                schedule_fluid_update: false,
            });
        }
    }

    Ok(Legacy1201AquiferSample {
        state: Some(candidate),
        schedule_fluid_update: true,
    })
}

/// The 12 grid cells around one anchor in vanilla's `x, y, z` loop order,
/// preserving the 26.3 column and vertical-step optimization for 1.20.1.
#[derive(Clone, Copy)]
struct Cells {
    index: [u32; 12],
    x: [i32; 12],
    y: [i32; 12],
    z: [i32; 12],
    /// `dx * dx + dz * dz` from the cell's randomized site to `column`.
    horizontal: [i32; 12],
}

/// Chunk-scoped 1.20.1 `NoiseBasedAquifer` with 26.3 flat-grid storage and
/// 12-cell sliding-window acceleration.
pub struct Legacy1201NoiseAquifer<'r, R: PositionalRandom> {
    picker: Legacy1201GlobalFluidPicker<'r>,
    random: &'r R,
    water: BlockStateId,
    target: ChunkPos,
    min_y: i32,
    max_y: i32,
    min_grid: [i32; 3],
    size_x: i32,
    size_z: i32,
    locations: Vec<i64>,
    points: Vec<BlockPos>,
    statuses: Vec<Option<Legacy1201FluidStatus>>,
    surfaces: HashMap<(i32, i32), i32>,
    anchor: Option<[i32; 3]>,
    column: [i32; 2],
    cells: Cells,
    schedule: bool,
}

impl<'r, R: PositionalRandom> Legacy1201NoiseAquifer<'r, R> {
    /// Construct a chunk-scoped 1.20.1 noise aquifer for `target` and vertical
    /// build bounds `min_y .. min_y + height`.
    pub fn new(
        picker: Legacy1201GlobalFluidPicker<'r>,
        random: &'r R,
        target: ChunkPos,
        min_y: i32,
        height: i32,
    ) -> io::Result<Self> {
        let registry = picker.registry();
        if registry.len() != 24_135 {
            return Err(invalid("1.20.1 noise aquifer requires 24135-state registry"));
        }
        let water = registry
            .default_state("minecraft:water")
            .ok_or_else(|| invalid("1.20.1 aquifer registry has no water"))?;
        if !(1..=4064).contains(&height) || min_y.rem_euclid(16) != 0 || height.rem_euclid(16) != 0 {
            return Err(invalid("invalid 1.20.1 aquifer vertical bounds"));
        }
        let max_y = min_y
            .checked_add(height)
            .ok_or_else(|| invalid("1.20.1 aquifer max_y overflow"))?;
        let min_block_x = target
            .x
            .checked_mul(16)
            .ok_or_else(|| invalid("1.20.1 aquifer chunk X overflow"))?;
        let max_block_x = min_block_x
            .checked_add(15)
            .ok_or_else(|| invalid("1.20.1 aquifer chunk max X overflow"))?;
        let min_block_z = target
            .z
            .checked_mul(16)
            .ok_or_else(|| invalid("1.20.1 aquifer chunk Z overflow"))?;
        let max_block_z = min_block_z
            .checked_add(15)
            .ok_or_else(|| invalid("1.20.1 aquifer chunk max Z overflow"))?;

        let min_grid = [
            min_block_x
                .div_euclid(16)
                .checked_sub(1)
                .ok_or_else(|| invalid("1.20.1 aquifer grid X underflow"))?,
            min_y
                .div_euclid(12)
                .checked_sub(1)
                .ok_or_else(|| invalid("1.20.1 aquifer grid Y underflow"))?,
            min_block_z
                .div_euclid(16)
                .checked_sub(1)
                .ok_or_else(|| invalid("1.20.1 aquifer grid Z underflow"))?,
        ];
        let max_grid = [
            max_block_x
                .div_euclid(16)
                .checked_add(1)
                .ok_or_else(|| invalid("1.20.1 aquifer grid X overflow"))?,
            max_y
                .div_euclid(12)
                .checked_add(1)
                .ok_or_else(|| invalid("1.20.1 aquifer grid Y overflow"))?,
            max_block_z
                .div_euclid(16)
                .checked_add(1)
                .ok_or_else(|| invalid("1.20.1 aquifer grid Z overflow"))?,
        ];
        let sizes = [
            max_grid[0]
                .checked_sub(min_grid[0])
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| invalid("1.20.1 aquifer grid size X overflow"))?,
            max_grid[1]
                .checked_sub(min_grid[1])
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| invalid("1.20.1 aquifer grid size Y overflow"))?,
            max_grid[2]
                .checked_sub(min_grid[2])
                .and_then(|v| v.checked_add(1))
                .ok_or_else(|| invalid("1.20.1 aquifer grid size Z overflow"))?,
        ];
        let count = sizes
            .into_iter()
            .try_fold(1usize, |acc, s| {
                if s > 0 {
                    acc.checked_mul(s as usize)
                } else {
                    None
                }
            })
            .filter(|&n| n <= 16_777_216)
            .ok_or_else(|| invalid("1.20.1 aquifer grid exceeds size limit"))?;

        let mut locations = Vec::new();
        locations
            .try_reserve_exact(count)
            .map_err(|_| invalid("1.20.1 aquifer location allocation failed"))?;
        locations.resize(count, i64::MAX);

        let mut points = Vec::new();
        points
            .try_reserve_exact(count)
            .map_err(|_| invalid("1.20.1 aquifer point allocation failed"))?;
        points.resize(count, BlockPos::default());

        let mut statuses = Vec::new();
        statuses
            .try_reserve_exact(count)
            .map_err(|_| invalid("1.20.1 aquifer status allocation failed"))?;
        statuses.resize(count, None);

        Ok(Self {
            picker,
            random,
            water,
            target,
            min_y,
            max_y,
            min_grid,
            size_x: sizes[0],
            size_z: sizes[2],
            locations,
            points,
            statuses,
            surfaces: HashMap::new(),
            anchor: None,
            column: [0; 2],
            cells: Cells {
                index: [0; 12],
                x: [0; 12],
                y: [0; 12],
                z: [0; 12],
                horizontal: [0; 12],
            },
            schedule: false,
        })
    }

    pub fn verify_level<L: Legacy1201Level>(&self, level: &L) -> io::Result<()> {
        self.picker.verify_level(level)?;
        if level.min_y() != self.min_y || level.max_y() != self.max_y {
            return Err(invalid("1.20.1 aquifer/level vertical bounds mismatch"));
        }
        Ok(())
    }

    pub const fn target(&self) -> ChunkPos {
        self.target
    }

    pub const fn min_y(&self) -> i32 {
        self.min_y
    }

    pub const fn max_y(&self) -> i32 {
        self.max_y
    }

    pub const fn should_schedule_fluid_update(&self) -> bool {
        self.schedule
    }

    pub fn cached_site_count(&self) -> usize {
        self.locations.iter().filter(|&&loc| loc != i64::MAX).count()
    }

    pub fn cached_status_count(&self) -> usize {
        self.statuses.iter().filter(|entry| entry.is_some()).count()
    }

    pub fn cached_surface_count(&self) -> usize {
        self.surfaces.len()
    }

    pub fn clear(&mut self) {
        self.locations.fill(i64::MAX);
        self.points.fill(BlockPos::default());
        self.statuses.fill(None);
        self.surfaces.clear();
        self.anchor = None;
        self.column = [0; 2];
        self.schedule = false;
    }

    fn grid_index(&self, cell: [i32; 3]) -> io::Result<usize> {
        let rx = cell[0].wrapping_sub(self.min_grid[0]);
        let ry = cell[1].wrapping_sub(self.min_grid[1]);
        let rz = cell[2].wrapping_sub(self.min_grid[2]);
        if rx < 0 || rx >= self.size_x || rz < 0 || rz >= self.size_z || ry < 0 {
            return Err(invalid("1.20.1 aquifer cell outside chunk grid"));
        }
        let index = ry
            .wrapping_mul(self.size_z)
            .wrapping_add(rz)
            .wrapping_mul(self.size_x)
            .wrapping_add(rx);
        if index < 0 || index as usize >= self.locations.len() {
            return Err(invalid("1.20.1 aquifer cell index out of bounds"));
        }
        Ok(index as usize)
    }

    /// Load the 12 cells around `grid` for `column`, shifting 8 already-drawn
    /// cells when descending one anchor in Y (26.3 sliding-window optimization)
    /// while preserving vanilla 1.20.1 RNG draw order.
    #[inline(never)]
    fn load_cells(&mut self, grid: [i32; 3], column: [i32; 2]) -> io::Result<()> {
        let down = self.anchor == Some([grid[0], grid[1].wrapping_add(1), grid[2]]);
        self.anchor = None;
        if down {
            let cells = &mut self.cells;
            for to in [11, 10, 9, 8, 5, 4, 3, 2] {
                cells.index[to] = cells.index[to - 2];
                cells.x[to] = cells.x[to - 2];
                cells.y[to] = cells.y[to - 2];
                cells.z[to] = cells.z[to - 2];
                cells.horizontal[to] = cells.horizontal[to - 2];
            }
            for (slot, dx, dz) in [(0, 0, 0), (1, 0, 1), (6, 1, 0), (7, 1, 1)] {
                self.load_cell(slot, [dx, -1, dz], grid, column)?;
            }
            if self.column != column {
                self.column = column;
                self.measure_column();
            }
        } else {
            let mut slot = 0;
            for dx in 0..=1i32 {
                for dy in -1..=1i32 {
                    for dz in 0..=1i32 {
                        self.load_cell(slot, [dx, dy, dz], grid, column)?;
                        slot += 1;
                    }
                }
            }
            self.column = column;
        }
        self.anchor = Some(grid);
        Ok(())
    }

    #[inline(always)]
    fn load_cell(
        &mut self,
        slot: usize,
        [dx, dy, dz]: [i32; 3],
        grid: [i32; 3],
        [x, z]: [i32; 2],
    ) -> io::Result<()> {
        let cell = [
            grid[0].wrapping_add(dx),
            grid[1].wrapping_add(dy),
            grid[2].wrapping_add(dz),
        ];
        let index = self.grid_index(cell)?;
        if self.locations[index] == i64::MAX {
            let point = legacy1201_aquifer_site(
                self.random,
                BlockPos::new(cell[0], cell[1], cell[2]),
            );
            self.locations[index] = point.pack();
            self.points[index] = point;
        }
        let point = self.points[index];
        let ax = point.x.wrapping_sub(x);
        let az = point.z.wrapping_sub(z);
        self.cells.index[slot] = index as u32;
        self.cells.x[slot] = point.x;
        self.cells.y[slot] = point.y;
        self.cells.z[slot] = point.z;
        self.cells.horizontal[slot] = ax.wrapping_mul(ax).wrapping_add(az.wrapping_mul(az));
        Ok(())
    }

    fn measure_column(&mut self) {
        let [x, z] = self.column;
        for i in 0..12 {
            let ax = self.cells.x[i].wrapping_sub(x);
            let az = self.cells.z[i].wrapping_sub(z);
            self.cells.horizontal[i] = ax.wrapping_mul(ax).wrapping_add(az.wrapping_mul(az));
        }
    }

    /// Rank the 3 closest cells out of the 12-cell neighborhood using the
    /// precalculated horizontal squared distances and `<=` tie displacement.
    #[inline(always)]
    fn closest_three(&mut self, position: BlockPos) -> io::Result<([i32; 3], [usize; 3])> {
        let anchor = legacy1201_aquifer_anchor(position);
        let grid = [anchor.x, anchor.y, anchor.z];
        let column = [position.x, position.z];
        if self.anchor != Some(grid) {
            self.load_cells(grid, column)?;
        } else if self.column != column {
            self.column = column;
            self.measure_column();
        }
        let mut distances = [i32::MAX; 3];
        let mut indices = [0usize; 3];
        for i in 0..12 {
            let ay = self.cells.y[i].wrapping_sub(position.y);
            let distance = self.cells.horizontal[i].wrapping_add(ay.wrapping_mul(ay));
            let index = self.cells.index[i] as usize;
            if distance <= distances[0] {
                distances = [distance, distances[0], distances[1]];
                indices = [index, indices[0], indices[1]];
            } else if distance <= distances[1] {
                distances[2] = distances[1];
                distances[1] = distance;
                indices[2] = indices[1];
                indices[1] = index;
            } else if distance <= distances[2] {
                distances[2] = distance;
                indices[2] = index;
            }
        }
        Ok((distances, indices))
    }

    #[inline(always)]
    fn status_at_index<P, D, F, S, L>(
        &mut self,
        index: usize,
        sample_surface: &mut P,
        deep_dark: &mut D,
        floodedness: &mut F,
        spread: &mut S,
        lava_noise: &mut L,
    ) -> io::Result<Legacy1201FluidStatus>
    where
        P: FnMut(i32, i32) -> io::Result<i32>,
        D: FnMut(BlockPos) -> io::Result<bool>,
        F: FnMut(BlockPos) -> io::Result<f64>,
        S: FnMut(BlockPos) -> io::Result<f64>,
        L: FnMut(BlockPos) -> io::Result<f64>,
    {
        if let Some(status) = self.statuses[index] {
            return Ok(status);
        }
        self.compute_status_at_index(
            index,
            sample_surface,
            deep_dark,
            floodedness,
            spread,
            lava_noise,
        )
    }

    #[inline(never)]
    fn compute_status_at_index<P, D, F, S, L>(
        &mut self,
        index: usize,
        sample_surface: &mut P,
        deep_dark: &mut D,
        floodedness: &mut F,
        spread: &mut S,
        lava_noise: &mut L,
    ) -> io::Result<Legacy1201FluidStatus>
    where
        P: FnMut(i32, i32) -> io::Result<i32>,
        D: FnMut(BlockPos) -> io::Result<bool>,
        F: FnMut(BlockPos) -> io::Result<f64>,
        S: FnMut(BlockPos) -> io::Result<f64>,
        L: FnMut(BlockPos) -> io::Result<f64>,
    {
        let site = self.points[index];
        let picker = self.picker;
        let lava = picker.lava();
        let surfaces = &mut self.surfaces;
        let mut cached_surface = |x: i32, z: i32| -> io::Result<i32> {
            if let Some(height) = surfaces.get(&(x, z)) {
                return Ok(*height);
            }
            let value = sample_surface(x, z)?;
            surfaces.insert((x, z), value);
            Ok(value)
        };
        let mut global_at = |y: i32| picker.compute(BlockPos::new(site.x, y, site.z));
        let computed = legacy1201_compute_aquifer_cell_status(
            site,
            lava,
            &mut cached_surface,
            &mut global_at,
            deep_dark,
            floodedness,
            spread,
            lava_noise,
        )?;
        self.statuses[index] = Some(computed);
        Ok(computed)
    }

    /// Compute the 1.20.1 block substance (`None` for solid stone/terrain,
    /// `Some(state)` for air/fluid) and update [`Self::should_schedule_fluid_update`].
    #[allow(clippy::too_many_arguments)]
    pub fn compute_substance<P, D, F, S, L, B>(
        &mut self,
        position: BlockPos,
        density: f64,
        sample_surface: &mut P,
        deep_dark: &mut D,
        floodedness: &mut F,
        spread: &mut S,
        lava_noise: &mut L,
        sample_barrier: &mut B,
    ) -> io::Result<Option<BlockStateId>>
    where
        P: FnMut(i32, i32) -> io::Result<i32>,
        D: FnMut(BlockPos) -> io::Result<bool>,
        F: FnMut(BlockPos) -> io::Result<f64>,
        S: FnMut(BlockPos) -> io::Result<f64>,
        L: FnMut(BlockPos) -> io::Result<f64>,
        B: FnMut(BlockPos) -> io::Result<f64>,
    {
        self.schedule = false;
        if !density.is_finite() {
            return Err(invalid("non-finite 1.20.1 aquifer terrain density"));
        }
        if position.chunk() != self.target || position.y < self.min_y || position.y >= self.max_y {
            return Err(invalid("1.20.1 aquifer query outside owning chunk bounds"));
        }
        if density > 0.0 {
            return Ok(None);
        }

        let air = self.picker.air();
        let lava = self.picker.lava();
        let water = self.water;
        let global = self.picker.compute(position);
        if global.at(position.y, air) == lava {
            return Ok(Some(lava));
        }

        let (distances, indices) = self.closest_three(position)?;
        let status_0 = self.status_at_index(
            indices[0],
            sample_surface,
            deep_dark,
            floodedness,
            spread,
            lava_noise,
        )?;
        let sim12 = legacy1201_aquifer_similarity(distances[0], distances[1]);
        let candidate = status_0.at(position.y, air);
        if sim12 <= 0.0 {
            self.schedule = sim12 >= LEGACY1201_FLOWING_UPDATE_SIMILARITY;
            return Ok(Some(candidate));
        }

        if candidate == water {
            let below_y = position
                .y
                .checked_sub(1)
                .ok_or_else(|| invalid("1.20.1 aquifer y underflow"))?;
            let below = BlockPos::new(position.x, below_y, position.z);
            if self.picker.compute(below).at(below_y, air) == lava {
                self.schedule = true;
                return Ok(Some(candidate));
            }
        }

        let status_1 = self.status_at_index(
            indices[1],
            sample_surface,
            deep_dark,
            floodedness,
            spread,
            lava_noise,
        )?;
        let mut barrier = Legacy1201BarrierNoise::new(position, sample_barrier);
        let p12 = sim12
            * legacy1201_pair_pressure(
                position.y,
                status_0,
                status_1,
                air,
                water,
                lava,
                &mut barrier,
            )?;
        if density + p12 > 0.0 {
            return Ok(None);
        }

        let status_2 = self.status_at_index(
            indices[2],
            sample_surface,
            deep_dark,
            floodedness,
            spread,
            lava_noise,
        )?;
        let sim13 = legacy1201_aquifer_similarity(distances[0], distances[2]);
        if sim13 > 0.0 {
            let p13 = sim12
                * sim13
                * legacy1201_pair_pressure(
                    position.y,
                    status_0,
                    status_2,
                    air,
                    water,
                    lava,
                    &mut barrier,
                )?;
            if density + p13 > 0.0 {
                return Ok(None);
            }
        }

        let sim23 = legacy1201_aquifer_similarity(distances[1], distances[2]);
        if sim23 > 0.0 {
            let p23 = sim12
                * sim23
                * legacy1201_pair_pressure(
                    position.y,
                    status_1,
                    status_2,
                    air,
                    water,
                    lava,
                    &mut barrier,
                )?;
            if density + p23 > 0.0 {
                return Ok(None);
            }
        }

        self.schedule = true;
        Ok(Some(candidate))
    }

    /// Evaluate both substance and fluid-update flag as a single sample.
    #[allow(clippy::too_many_arguments)]
    pub fn sample_at<P, D, F, S, L, B>(
        &mut self,
        position: BlockPos,
        density: f64,
        sample_surface: &mut P,
        deep_dark: &mut D,
        floodedness: &mut F,
        spread: &mut S,
        lava_noise: &mut L,
        sample_barrier: &mut B,
    ) -> io::Result<Legacy1201AquiferSample>
    where
        P: FnMut(i32, i32) -> io::Result<i32>,
        D: FnMut(BlockPos) -> io::Result<bool>,
        F: FnMut(BlockPos) -> io::Result<f64>,
        S: FnMut(BlockPos) -> io::Result<f64>,
        L: FnMut(BlockPos) -> io::Result<f64>,
        B: FnMut(BlockPos) -> io::Result<f64>,
    {
        let state = self.compute_substance(
            position,
            density,
            sample_surface,
            deep_dark,
            floodedness,
            spread,
            lava_noise,
            sample_barrier,
        )?;
        Ok(Legacy1201AquiferSample {
            state,
            schedule_fluid_update: self.schedule,
        })
    }

    /// `WorldCarver` evaluates its aquifer at `density = 0.0` and immediately
    /// reads `shouldScheduleFluidUpdate()`.
    #[allow(clippy::too_many_arguments)]
    pub fn sample_carver_cell<P, D, F, S, L, B>(
        &mut self,
        position: BlockPos,
        sample_surface: &mut P,
        deep_dark: &mut D,
        floodedness: &mut F,
        spread: &mut S,
        lava_noise: &mut L,
        sample_barrier: &mut B,
    ) -> io::Result<Legacy1201AquiferSample>
    where
        P: FnMut(i32, i32) -> io::Result<i32>,
        D: FnMut(BlockPos) -> io::Result<bool>,
        F: FnMut(BlockPos) -> io::Result<f64>,
        S: FnMut(BlockPos) -> io::Result<f64>,
        L: FnMut(BlockPos) -> io::Result<f64>,
        B: FnMut(BlockPos) -> io::Result<f64>,
    {
        self.sample_at(
            position,
            0.0,
            sample_surface,
            deep_dark,
            floodedness,
            spread,
            lava_noise,
            sample_barrier,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        legacy_1_20_1_aquifer_surface::Legacy1201PreliminarySurfaceCache,
        nbt::Tag,
        registry::BlockRegistry,
    };
    use horizon_core::random::{LegacyPositionalRandom, XoroshiroPositionalRandom};
    use std::{cell::Cell, sync::OnceLock};

    const AIR: BlockStateId = BlockStateId(0);
    const WATER: BlockStateId = BlockStateId(1);
    const LAVA: BlockStateId = BlockStateId(2);

    fn registry_24135() -> &'static BlockRegistry {
        static REGISTRY: OnceLock<BlockRegistry> = OnceLock::new();
        REGISTRY.get_or_init(|| {
            let mut states = Vec::with_capacity(24_135);
            for name in ["minecraft:air", "minecraft:water", "minecraft:lava", "test:other"] {
                states.push(Tag::compound([("id", Tag::String(name.into()))]));
            }
            for idx in 4..24_135 {
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
                        ("test:other", Tag::Int(3)),
                        ("test:filler", Tag::Int(4)),
                    ]),
                ),
            ]))
            .unwrap()
        })
    }

    fn picker() -> Legacy1201GlobalFluidPicker<'static> {
        Legacy1201GlobalFluidPicker::new(registry_24135(), 63, WATER).unwrap()
    }

    #[test]
    fn flowing_update_similarity_matches_vanilla_10_12_squares() {
        assert!((LEGACY1201_FLOWING_UPDATE_SIMILARITY - (-0.76)).abs() < 1e-12);
        assert_eq!(
            legacy1201_aquifer_similarity(100, 144),
            LEGACY1201_FLOWING_UPDATE_SIMILARITY
        );
    }

    #[test]
    fn positive_density_and_global_lava_skip_sites_and_noises() {
        let rng = LegacyPositionalRandom::new(42);
        let mut aquifer =
            Legacy1201NoiseAquifer::new(picker(), &rng, ChunkPos::new(0, 0), -64, 384).unwrap();
        let mut no_surface = |_, _| -> io::Result<i32> { panic!("must not sample surface") };
        let mut no_dark = |_| -> io::Result<bool> { panic!("must not sample deep dark") };
        let mut no_flood = |_| -> io::Result<f64> { panic!("must not sample floodedness") };
        let mut no_spread = |_| -> io::Result<f64> { panic!("must not sample spread") };
        let mut no_lava = |_| -> io::Result<f64> { panic!("must not sample lava") };
        let mut no_barrier = |_| -> io::Result<f64> { panic!("must not sample barrier") };

        assert_eq!(
            aquifer
                .sample_at(
                    BlockPos::new(4, 20, 4),
                    0.25,
                    &mut no_surface,
                    &mut no_dark,
                    &mut no_flood,
                    &mut no_spread,
                    &mut no_lava,
                    &mut no_barrier,
                )
                .unwrap(),
            Legacy1201AquiferSample {
                state: None,
                schedule_fluid_update: false,
            }
        );
        assert_eq!(aquifer.cached_site_count(), 0);

        assert_eq!(
            aquifer
                .sample_at(
                    BlockPos::new(4, -55, 4),
                    -0.5,
                    &mut no_surface,
                    &mut no_dark,
                    &mut no_flood,
                    &mut no_spread,
                    &mut no_lava,
                    &mut no_barrier,
                )
                .unwrap(),
            Legacy1201AquiferSample {
                state: Some(LAVA),
                schedule_fluid_update: false,
            }
        );
        assert_eq!(aquifer.cached_site_count(), 0);
    }

    #[test]
    fn non_positive_sim12_queries_only_closest_site_and_obeys_flowing_threshold() {
        let rng = LegacyPositionalRandom::new(12345);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        // Find a position where sim12 <= -0.76 (dist2 - dist1 >= 45) and one where
        // -0.76 <= sim12 <= 0.0 (25 <= dist2 - dist1 <= 44).
        let mut far_pos = None;
        let mut medium_pos = None;
        'search: for x in -32..32 {
            for y in -40..40 {
                for z in -32..32 {
                    let pos = BlockPos::new(x, y, z);
                    let sites = cache.nearest(pos);
                    let diff = sites[1].squared_distance - sites[0].squared_distance;
                    if diff >= 45 && far_pos.is_none() {
                        far_pos = Some(pos);
                    } else if (25..=44).contains(&diff) && medium_pos.is_none() {
                        medium_pos = Some(pos);
                    }
                    if far_pos.is_some() && medium_pos.is_some() {
                        break 'search;
                    }
                }
            }
        }
        let far = far_pos.expect("found far site pair");
        let medium = medium_pos.expect("found medium site pair");

        let status_calls = Cell::new(0usize);
        let mut status_sampler = |_site: BlockPos| {
            status_calls.set(status_calls.get() + 1);
            Ok(Legacy1201FluidStatus {
                level: 63,
                state: WATER,
            })
        };
        let mut no_barrier = |_| -> io::Result<f64> { panic!("barrier must not be sampled") };

        cache.clear();
        let far_sample = legacy1201_resolve_aquifer_substance(
            far,
            -0.2,
            &picker(),
            WATER,
            &mut cache,
            &mut status_sampler,
            &mut no_barrier,
        )
        .unwrap();
        assert_eq!(
            far_sample,
            Legacy1201AquiferSample {
                state: Some(WATER),
                schedule_fluid_update: false,
            }
        );
        assert_eq!(status_calls.get(), 1);

        cache.clear();
        status_calls.set(0);
        let medium_sample = legacy1201_resolve_aquifer_substance(
            medium,
            -0.2,
            &picker(),
            WATER,
            &mut cache,
            &mut status_sampler,
            &mut no_barrier,
        )
        .unwrap();
        assert_eq!(
            medium_sample,
            Legacy1201AquiferSample {
                state: Some(WATER),
                schedule_fluid_update: true,
            }
        );
        assert_eq!(status_calls.get(), 1);
    }

    #[test]
    fn water_directly_above_global_lava_schedules_update_without_second_status() {
        let rng = LegacyPositionalRandom::new(99);
        // Find a position at y = -54 (so y - 1 = -55 is global lava) with sim12 > 0.
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        let pos = (-64..64)
            .flat_map(|x| (-64..64).map(move |z| BlockPos::new(x, -54, z)))
            .find(|&p| {
                let s = cache.nearest(p);
                s[1].squared_distance - s[0].squared_distance < 25
            })
            .expect("found boundary position at y=-54");

        cache.clear();
        let calls = Cell::new(0usize);
        let mut status_sampler = |_site: BlockPos| {
            calls.set(calls.get() + 1);
            Ok(Legacy1201FluidStatus {
                level: 0,
                state: WATER,
            })
        };
        let mut no_barrier = |_| -> io::Result<f64> { panic!("no barrier on water-over-lava") };
        let sample = legacy1201_resolve_aquifer_substance(
            pos,
            -10.0,
            &picker(),
            WATER,
            &mut cache,
            &mut status_sampler,
            &mut no_barrier,
        )
        .unwrap();
        assert_eq!(
            sample,
            Legacy1201AquiferSample {
                state: Some(WATER),
                schedule_fluid_update: true,
            }
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn first_pair_solid_barrier_skips_third_site_status() {
        let rng = LegacyPositionalRandom::new(7);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        let pos = (-32..32)
            .flat_map(|x| (-32..32).map(move |z| BlockPos::new(x, 10, z)))
            .find(|&p| {
                let s = cache.nearest(p);
                s[1].squared_distance - s[0].squared_distance < 20
            })
            .expect("found close pair");

        cache.clear();
        let calls = Cell::new(0usize);
        let mut status_sampler = |_site: BlockPos| {
            let n = calls.get() + 1;
            calls.set(n);
            Ok(if n == 1 {
                Legacy1201FluidStatus {
                    level: 40,
                    state: WATER,
                }
            } else {
                Legacy1201FluidStatus {
                    level: 40,
                    state: LAVA,
                }
            })
        };
        let mut no_barrier = |_| -> io::Result<f64> { panic!("water-lava uses fixed 2.0") };
        let sample = legacy1201_resolve_aquifer_substance(
            pos,
            -0.1,
            &picker(),
            WATER,
            &mut cache,
            &mut status_sampler,
            &mut no_barrier,
        )
        .unwrap();
        assert_eq!(
            sample,
            Legacy1201AquiferSample {
                state: None,
                schedule_fluid_update: false,
            }
        );
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn optimized_flat_grid_and_cell_cache_produce_identical_samples_and_counts() {
        for seed in [0i64, -1, 1201, 987_654_321] {
            let rng = LegacyPositionalRandom::new(seed);
            let target = ChunkPos::new(-2, 1);
            let mut optimized =
                Legacy1201NoiseAquifer::new(picker(), &rng, target, -64, 384).unwrap();
            let mut ref_cache = Legacy1201AquiferCellCache::new(&rng);
            let mut ref_surfaces = Legacy1201PreliminarySurfaceCache::new(|x: i32, z: i32| {
                Ok(48 + (x.div_euclid(4).wrapping_mul(5) + z.div_euclid(4).wrapping_mul(3)).rem_euclid(19))
            });

            let surface_fn = |x: i32, z: i32| -> io::Result<i32> {
                Ok(48 + (x.div_euclid(4).wrapping_mul(5) + z.div_euclid(4).wrapping_mul(3)).rem_euclid(19))
            };
            let dark_fn = |site: BlockPos| -> io::Result<bool> {
                Ok(site.y < -36 && (site.x + site.z).rem_euclid(7) == 0)
            };
            let flood_fn = |site: BlockPos| -> io::Result<f64> {
                Ok(((site.x * 13 + site.y * 7 - site.z * 5).rem_euclid(200) as f64) / 100.0 - 1.0)
            };
            let spread_fn = |cell: BlockPos| -> io::Result<f64> {
                Ok(((cell.x * 3 - cell.y * 11 + cell.z * 7).rem_euclid(200) as f64) / 100.0 - 1.0)
            };
            let lava_fn = |cell: BlockPos| -> io::Result<f64> {
                Ok(((cell.x * 17 + cell.y * 5 + cell.z * 19).rem_euclid(200) as f64) / 100.0 - 1.0)
            };
            let barrier_fn = |pos: BlockPos| -> io::Result<f64> {
                Ok(((pos.x * 7 - pos.y * 3 + pos.z * 11).rem_euclid(100) as f64) / 100.0 - 0.5)
            };

            // Walk columns top-to-bottom to exercise the 26.3 downward sliding window.
            for local_x in [0, 5, 11, 15] {
                for local_z in [0, 4, 9, 15] {
                    let wx = target.x * 16 + local_x;
                    let wz = target.z * 16 + local_z;
                    for y in (-64..128).rev().step_by(3) {
                        let pos = BlockPos::new(wx, y, wz);
                        let density = if (wx + y + wz).rem_euclid(11) == 0 {
                            0.35
                        } else {
                            -0.18
                        };
                        let mut s_surf = surface_fn;
                        let mut s_dark = dark_fn;
                        let mut s_flood = flood_fn;
                        let mut s_spread = spread_fn;
                        let mut s_lava = lava_fn;
                        let mut s_bar = barrier_fn;
                        let got = optimized
                            .sample_at(
                                pos,
                                density,
                                &mut s_surf,
                                &mut s_dark,
                                &mut s_flood,
                                &mut s_spread,
                                &mut s_lava,
                                &mut s_bar,
                            )
                            .unwrap();

                        let p = picker();
                        let mut status_cb = |site: BlockPos| {
                            let mut surf = |x, z| ref_surfaces.get(x, z);
                            let mut glob = |gy| p.compute(BlockPos::new(site.x, gy, site.z));
                            let mut d = dark_fn;
                            let mut fl = flood_fn;
                            let mut sp = spread_fn;
                            let mut lv = lava_fn;
                            legacy1201_compute_aquifer_cell_status(
                                site, LAVA, &mut surf, &mut glob, &mut d, &mut fl, &mut sp, &mut lv,
                            )
                        };
                        let mut b_cb = barrier_fn;
                        let expected = legacy1201_resolve_aquifer_substance(
                            pos,
                            density,
                            &p,
                            WATER,
                            &mut ref_cache,
                            &mut status_cb,
                            &mut b_cb,
                        )
                        .unwrap();

                        assert_eq!(got, expected, "seed={seed} pos={pos:?}");
                    }
                }
            }
            assert_eq!(optimized.cached_site_count(), ref_cache.site_count());
            assert_eq!(optimized.cached_status_count(), ref_cache.status_count());
            assert_eq!(optimized.cached_surface_count(), ref_surfaces.len());
        }
    }

    #[test]
    fn downward_column_walk_matches_arbitrary_access_order() {
        let rng = XoroshiroPositionalRandom::new(19, 83);
        let target = ChunkPos::new(0, -1);
        let mut descending =
            Legacy1201NoiseAquifer::new(picker(), &rng, target, -64, 384).unwrap();
        let mut scrambled =
            Legacy1201NoiseAquifer::new(picker(), &rng, target, -64, 384).unwrap();

        let mut positions = Vec::new();
        for x in [2, 13] {
            for z in [-15, -2] {
                for y in (-60..80).rev() {
                    positions.push(BlockPos::new(x, y, z));
                }
            }
        }
        let eval = |aquifer: &mut Legacy1201NoiseAquifer<'_, _>, pos: BlockPos| {
            let mut surf = |x: i32, z: i32| Ok(52 + (x + z).rem_euclid(9));
            let mut dark = |_: BlockPos| Ok(false);
            let mut flood = |p: BlockPos| Ok((p.y as f64 / 120.0).clamp(-1.0, 1.0));
            let mut spread = |_: BlockPos| Ok(-0.2);
            let mut lava = |_: BlockPos| Ok(0.45);
            let mut barrier = |_: BlockPos| Ok(0.05);
            aquifer
                .sample_carver_cell(
                    pos,
                    &mut surf,
                    &mut dark,
                    &mut flood,
                    &mut spread,
                    &mut lava,
                    &mut barrier,
                )
                .unwrap()
        };

        let mut expected = HashMap::new();
        for &pos in &positions {
            expected.insert(pos, eval(&mut descending, pos));
        }
        for &pos in positions.iter().rev().step_by(2).chain(positions.iter().step_by(2)) {
            assert_eq!(eval(&mut scrambled, pos), expected[&pos]);
        }
    }

    #[test]
    fn transient_callback_failures_never_poison_grid_or_surface_caches() {
        let rng = LegacyPositionalRandom::new(55);
        let mut aquifer =
            Legacy1201NoiseAquifer::new(picker(), &rng, ChunkPos::new(0, 0), -64, 384).unwrap();
        let pos = BlockPos::new(8, 12, 8);
        let fail_surface = Cell::new(true);
        let mut surf = |_, _| {
            if fail_surface.get() {
                Err(io::Error::new(io::ErrorKind::WouldBlock, "surface not ready"))
            } else {
                Ok(60)
            }
        };
        let mut dark = |_| Ok(false);
        let mut flood = |_| Ok(0.9);
        let mut spread = |_| Ok(0.0);
        let mut lava = |_| Ok(0.0);
        let mut barrier = |_| Ok(0.0);

        assert_eq!(
            aquifer
                .sample_carver_cell(
                    pos,
                    &mut surf,
                    &mut dark,
                    &mut flood,
                    &mut spread,
                    &mut lava,
                    &mut barrier,
                )
                .unwrap_err()
                .kind(),
            io::ErrorKind::WouldBlock
        );
        assert_eq!(aquifer.cached_status_count(), 0);
        assert_eq!(aquifer.cached_surface_count(), 0);
        assert!(!aquifer.should_schedule_fluid_update());

        fail_surface.set(false);
        let ok = aquifer
            .sample_carver_cell(
                pos,
                &mut surf,
                &mut dark,
                &mut flood,
                &mut spread,
                &mut lava,
                &mut barrier,
            )
            .unwrap();
        assert_eq!(ok.state, Some(WATER));
        assert!(aquifer.cached_status_count() >= 1);
        assert!(aquifer.cached_surface_count() >= 1);
    }

    #[test]
    fn rejects_out_of_chunk_or_non_finite_density_queries() {
        let rng = LegacyPositionalRandom::new(1);
        let mut aquifer =
            Legacy1201NoiseAquifer::new(picker(), &rng, ChunkPos::new(0, 0), -64, 384).unwrap();
        let mut surf = |_, _| Ok(60);
        let mut dark = |_| Ok(false);
        let mut flood = |_| Ok(0.0);
        let mut spread = |_| Ok(0.0);
        let mut lava = |_| Ok(0.0);
        let mut barrier = |_| Ok(0.0);

        for bad_pos in [
            BlockPos::new(16, 0, 0),
            BlockPos::new(-1, 0, 0),
            BlockPos::new(0, -65, 0),
            BlockPos::new(0, 320, 0),
        ] {
            assert!(aquifer
                .sample_at(
                    bad_pos,
                    -0.1,
                    &mut surf,
                    &mut dark,
                    &mut flood,
                    &mut spread,
                    &mut lava,
                    &mut barrier,
                )
                .is_err());
        }
        for bad_density in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert!(aquifer
                .sample_at(
                    BlockPos::new(0, 0, 0),
                    bad_density,
                    &mut surf,
                    &mut dark,
                    &mut flood,
                    &mut spread,
                    &mut lava,
                    &mut barrier,
                )
                .is_err());
        }
    }
}
