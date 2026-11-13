//! Geometry only, using the exact 1.20.1 path RNG order and trig indexing.
use super::super::{CanyonShape, CarverError};
use super::{Legacy1201Carver, Legacy1201CarverShape, trig};
use crate::worldgen::{
    anchor::WorldGenerationContext, height_provider::HeightProvider, value_provider::FloatProvider,
};
use horizon_core::{
    carving::{CarverOutput, can_reach, carve_ellipsoid_with_budget},
    position::ChunkPos,
    random::{LegacyRandom, RandomSource},
};

fn sin(angle: f64) -> f32 {
    trig::sin(angle as f32)
}
fn cos(angle: f64) -> f32 {
    trig::cos(angle as f32)
}

impl Legacy1201Carver<'_> {
    pub fn is_start_chunk(&self, random: &mut impl RandomSource) -> bool {
        random.next_float() <= self.config.probability
    }
    /// CaveWorldCarver.carve / CanyonWorldCarver.carve (1.20.1).
    /// Emits candidate cells in their visit order. The sink owns the carving
    /// mask, debug repeat policy and terrain writes; no blocks are changed here.
    /// Its bounds must encode WorldCarver's min+1 / top-7 (or upgrading) clamps.
    pub fn carve_geometry<O: CarverOutput>(
        &self,
        context: WorldGenerationContext,
        random: &mut impl RandomSource,
        target: ChunkPos,
        origin: ChunkPos,
        output: &mut O,
    ) -> Result<bool, CarverError<O::Error>> {
        if !(1..=4064).contains(&context.height)
            || output.min_y() < context.min_y.wrapping_add(1)
            || output.max_y() > context.min_y.wrapping_add(context.height).wrapping_sub(1)
            || output.min_y() > output.max_y()
        {
            return Err(CarverError::InvalidBounds);
        }
        match &self.shape {
            Legacy1201CarverShape::Canyon {
                rotation,
                distance,
                thickness,
                width_smoothness,
                horizontal,
                vertical_default,
                vertical_center,
            } => CanyonGeometry {
                y: self.config.y.clone(),
                vertical_rotation: rotation.clone(),
                shape: CanyonShape {
                    distance_factor: distance.clone(),
                    thickness: thickness.clone(),
                    width_smoothness: *width_smoothness,
                    horizontal_radius_factor: horizontal.clone(),
                    vertical_radius_default_factor: *vertical_default,
                    vertical_radius_center_factor: *vertical_center,
                    y_scale: self.config.y_scale.clone(),
                },
            }
            .carve(context, random, target, origin, output),
            Legacy1201CarverShape::Cave {
                nether,
                horizontal,
                vertical,
                floor,
            } => {
                let bound = random.next_int_bound(if *nether { 10 } else { 15 })?;
                let bound = random.next_int_bound(bound + 1)?;
                let count = random.next_int_bound(bound + 1)?;
                let mut budget = 1 << 24;
                for _ in 0..count {
                    let x = f64::from(
                        origin
                            .x
                            .wrapping_shl(4)
                            .wrapping_add(random.next_int_bound(16)?),
                    );
                    let y = f64::from(self.config.y.sample(random, context)?);
                    let z = f64::from(
                        origin
                            .z
                            .wrapping_shl(4)
                            .wrapping_add(random.next_int_bound(16)?),
                    );
                    let mut run = CaveRun {
                        target,
                        output,
                        budget: &mut budget,
                        horizontal: f64::from(horizontal.sample(random)),
                        vertical: f64::from(vertical.sample(random)),
                        floor: f64::from(floor.sample(random)),
                    };
                    let mut tunnels = 1;
                    if random.next_int_bound(4)? == 0 {
                        let multiplier = f64::from(self.config.y_scale.sample(random));
                        let thickness = 1. + random.next_float() * 6.;
                        let radius =
                            1.5 + f64::from(trig::sin(std::f32::consts::FRAC_PI_2) * thickness);
                        run.ellipsoid([x + 1., y, z], radius, radius * multiplier)?;
                        tunnels += random.next_int_bound(4)?;
                    }
                    for _ in 0..tunnels {
                        let yaw = random.next_float() * std::f32::consts::TAU;
                        let pitch = (random.next_float() - 0.5) / 4.;
                        let mut thickness = random.next_float() * 2. + random.next_float();
                        if *nether {
                            thickness *= 2.;
                        } else if random.next_int_bound(10)? == 0 {
                            thickness *= random.next_float() * random.next_float() * 3. + 1.;
                        }
                        let end = 112 - random.next_int_bound(28)?;
                        run.tunnel(Tunnel {
                            seed: random.next_long(),
                            position: [x, y, z],
                            thickness,
                            yaw,
                            pitch,
                            start: 0,
                            end,
                            vertical_scale: if *nether { 5. } else { 1. },
                        })?;
                    }
                }
                Ok(true)
            }
        }
    }
}

struct CanyonGeometry {
    y: HeightProvider,
    vertical_rotation: FloatProvider,
    shape: CanyonShape,
}

impl CanyonGeometry {
    pub fn carve<O: CarverOutput>(
        &self,
        context: WorldGenerationContext,
        random: &mut impl RandomSource,
        target: ChunkPos,
        origin: ChunkPos,
        output: &mut O,
    ) -> Result<bool, CarverError<O::Error>> {
        let mut x = f64::from(
            origin
                .x
                .wrapping_shl(4)
                .wrapping_add(random.next_int_bound(16)?),
        );
        let mut y = f64::from(self.y.sample(random, context)?);
        let mut z = f64::from(
            origin
                .z
                .wrapping_shl(4)
                .wrapping_add(random.next_int_bound(16)?),
        );
        let mut yaw = random.next_float() * std::f32::consts::TAU;
        let mut pitch = self.vertical_rotation.sample(random);
        let y_scale = f64::from(self.shape.y_scale.sample(random));
        let thickness = self.shape.thickness.sample(random);
        let end = (112f32 * self.shape.distance_factor.sample(random)) as i32;
        let mut local = LegacyRandom::new(random.next_long());
        if !(1..=4064).contains(&context.height)
            || output.min_y() < context.min_y
            || i64::from(output.max_y()) > i64::from(context.min_y) + i64::from(context.height)
        {
            return Err(CarverError::InvalidBounds);
        }
        if end > 4096 {
            return Err(CarverError::IterationLimit);
        }
        let mut width = Vec::with_capacity(context.height as usize);
        let mut factor = 1f32;
        for index in 0..context.height {
            if index == 0 || local.next_int_bound(self.shape.width_smoothness)? == 0 {
                factor = 1. + local.next_float() * local.next_float();
            }
            width.push(factor * factor);
        }
        let (mut yaw_change, mut pitch_change) = (0f32, 0f32);
        let mut budget = 1 << 24;
        for step in 0..end {
            let mut horizontal = 1.5
                + f64::from(
                    sin(f64::from(step as f32 * std::f32::consts::PI / end as f32)) * thickness,
                );
            let mut vertical = horizontal * y_scale;
            horizontal *= f64::from(self.shape.horizontal_radius_factor.sample(&mut local));
            let center = 1. - (0.5 - step as f32 / end as f32).abs() * 2.;
            let multiplier = self.shape.vertical_radius_default_factor
                + self.shape.vertical_radius_center_factor * center;
            vertical = (f64::from(multiplier) * vertical)
                * f64::from(local.next_float() * (1. - 0.75) + 0.75);
            let cos_pitch = cos(f64::from(pitch));
            let sin_pitch = sin(f64::from(pitch));
            x += f64::from(cos(f64::from(yaw)) * cos_pitch);
            y += f64::from(sin_pitch);
            z += f64::from(sin(f64::from(yaw)) * cos_pitch);
            pitch *= 0.7;
            pitch += pitch_change * 0.05;
            yaw += yaw_change * 0.05;
            pitch_change *= 0.8;
            yaw_change *= 0.5;
            pitch_change += (local.next_float() - local.next_float()) * local.next_float() * 2.;
            yaw_change += (local.next_float() - local.next_float()) * local.next_float() * 4.;
            if local.next_int_bound(4)? == 0 {
                continue;
            }
            if !can_reach(target, x, z, step, end, thickness) {
                return Ok(true);
            }
            carve_ellipsoid_with_budget(
                target,
                [x, y, z],
                horizontal,
                vertical,
                output,
                |[nx, ny, nz], by| {
                    let factor = width[by.wrapping_sub(context.min_y).wrapping_sub(1) as usize];
                    (nx * nx + nz * nz) * f64::from(factor) + ny * ny / 6. >= 1.
                },
                &mut budget,
            )?;
        }
        Ok(true)
    }
}

struct CaveRun<'a, O: CarverOutput> {
    target: ChunkPos,
    output: &'a mut O,
    budget: &'a mut usize,
    horizontal: f64,
    vertical: f64,
    floor: f64,
}
#[derive(Clone, Copy)]
struct Tunnel {
    seed: i64,
    position: [f64; 3],
    thickness: f32,
    yaw: f32,
    pitch: f32,
    start: i32,
    end: i32,
    vertical_scale: f64,
}
impl<O: CarverOutput> CaveRun<'_, O> {
    fn ellipsoid(
        &mut self,
        position: [f64; 3],
        horizontal: f64,
        vertical: f64,
    ) -> Result<(), CarverError<O::Error>> {
        let floor = self.floor;
        Ok(carve_ellipsoid_with_budget(
            self.target,
            position,
            horizontal,
            vertical,
            self.output,
            |[x, y, z], _| y <= floor || x * x + y * y + z * z >= 1.,
            self.budget,
        )?)
    }
    fn tunnel(&mut self, mut path: Tunnel) -> Result<(), CarverError<O::Error>> {
        let mut random = LegacyRandom::new(path.seed);
        let split = random.next_int_bound(path.end / 2)? + path.end / 4;
        let gentle = random.next_int_bound(6)? == 0;
        let (mut yaw_change, mut pitch_change) = (0f32, 0f32);
        for step in path.start..path.end {
            let radius = 1.5
                + f64::from(
                    sin(f64::from(
                        std::f32::consts::PI * step as f32 / path.end as f32,
                    )) * path.thickness,
                );
            let vertical = radius * path.vertical_scale;
            let cos_pitch = cos(f64::from(path.pitch));
            path.position[0] += f64::from(cos(f64::from(path.yaw)) * cos_pitch);
            path.position[1] += f64::from(sin(f64::from(path.pitch)));
            path.position[2] += f64::from(sin(f64::from(path.yaw)) * cos_pitch);
            path.pitch *= if gentle { 0.92 } else { 0.7 };
            path.pitch += pitch_change * 0.1;
            path.yaw += yaw_change * 0.1;
            pitch_change *= 0.9;
            yaw_change *= 0.75;
            pitch_change += (random.next_float() - random.next_float()) * random.next_float() * 2.;
            yaw_change += (random.next_float() - random.next_float()) * random.next_float() * 4.;
            if step == split && path.thickness > 1. {
                let seed = random.next_long();
                let thickness = random.next_float() * 0.5 + 0.5;
                self.tunnel(Tunnel {
                    seed,
                    thickness,
                    yaw: path.yaw - std::f32::consts::FRAC_PI_2,
                    pitch: path.pitch / 3.,
                    start: step,
                    vertical_scale: 1.,
                    ..path
                })?;
                let seed = random.next_long();
                let thickness = random.next_float() * 0.5 + 0.5;
                self.tunnel(Tunnel {
                    seed,
                    thickness,
                    yaw: path.yaw + std::f32::consts::FRAC_PI_2,
                    pitch: path.pitch / 3.,
                    start: step,
                    vertical_scale: 1.,
                    ..path
                })?;
                return Ok(());
            }
            if random.next_int_bound(4)? == 0 {
                continue;
            }
            if !can_reach(
                self.target,
                path.position[0],
                path.position[2],
                step,
                path.end,
                path.thickness,
            ) {
                return Ok(());
            }
            self.ellipsoid(
                path.position,
                radius * self.horizontal,
                vertical * self.vertical,
            )?;
        }
        Ok(())
    }
}
