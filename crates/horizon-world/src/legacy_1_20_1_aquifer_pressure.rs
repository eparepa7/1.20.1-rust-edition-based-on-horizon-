//! Pressure-barrier calculation for the three-centre Minecraft Java 1.20.1
//! aquifer. The 1.21.2+ fourth-centre fluid-update rule is intentionally NOT
//! used here. This module does not synthesize fluid levels, decide fluid ticks,
//! or enable the 1.20.1 generator. It is a pure pressure component awaiting
//! differential comparison with a real 1.20.1 server.
//!
//! Each position has one lazily sampled `barrier` density value, shared by
//! the three possible pairs. A sample error propagates: it is never replaced
//! with zero or cached as a successful lookup.

use crate::{
    legacy_1_20_1_aquifer::{Legacy1201AquiferSite, Legacy1201FluidStatus},
    legacy_1_20_1_aquifer_boundary::legacy1201_aquifer_boundary_weights,
    registry::BlockStateId,
};
use horizon_core::position::BlockPos;
use std::io;

/// The final result of pressure checks only. `Open` still needs the nearest
/// fluid table, the global-lava override and the version-specific fluid-tick
/// decision; it must NOT be interpreted as a ready-to-write air block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201PressureOutcome {
    Solid,
    Open,
}

/// One noise value per query position, reused across all three pairs. This
/// cache is intentionally short-lived; create it for each queried block.
pub struct Legacy1201BarrierNoise<'a, F>
where
    F: FnMut(BlockPos) -> io::Result<f64>,
{
    position: BlockPos,
    sampler: &'a mut F,
    cached: Option<f64>,
}

impl<'a, F> Legacy1201BarrierNoise<'a, F>
where
    F: FnMut(BlockPos) -> io::Result<f64>,
{
    pub fn new(position: BlockPos, sampler: &'a mut F) -> Self {
        Self { position, sampler, cached: None }
    }

    pub fn sampled(&self) -> bool {
        self.cached.is_some()
    }

    pub fn get(&mut self) -> io::Result<f64> {
        if let Some(value) = self.cached {
            return Ok(value);
        }
        let value = (self.sampler)(self.position)?;
        if !value.is_finite() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "non-finite 1.20.1 aquifer barrier noise",
            ));
        }
        self.cached = Some(value);
        Ok(value)
    }
}

/// Pressure from two 1.20.1 fluid tables at the queried block's Y.
///
/// Water against lava at the same Y is a fixed, strong barrier. Otherwise,
/// equal fluid levels have no pressure and do not sample noise. For differing
/// levels, the vertical gradient is asymmetric: above the midpoint there is
/// a thin lid, below it a thicker floor. Noise contributes only when that
/// gradient is in the closed range [-2, 2].
///
/// `air`, `water` and `lava` must all come from the same validated 1.20.1
/// block registry as the statuses (this pure helper cannot check identity).
pub fn legacy1201_pair_pressure<F>(
    y: i32,
    first: Legacy1201FluidStatus,
    second: Legacy1201FluidStatus,
    air: BlockStateId,
    water: BlockStateId,
    lava: BlockStateId,
    barrier: &mut Legacy1201BarrierNoise<'_, F>,
) -> io::Result<f64>
where
    F: FnMut(BlockPos) -> io::Result<f64>,
{
    let a = first.at(y, air);
    let b = second.at(y, air);
    if (a == water && b == lava) || (a == lava && b == water) {
        return Ok(2.0);
    }

    // Use wide arithmetic so malformed fixtures cannot overflow in debug
    // builds. For valid 1.20.1 world heights these are exact Java doubles.
    let level_difference = (i64::from(first.level) - i64::from(second.level)).abs();
    if level_difference == 0 {
        return Ok(0.0);
    }
    let centre = 0.5 * (f64::from(first.level) + f64::from(second.level));
    let relative_y = (f64::from(y) + 0.5) - centre;
    let remaining = level_difference as f64 * 0.5 - relative_y.abs();
    let gradient = if relative_y > 0.0 {
        if remaining > 0.0 { remaining / 1.5 } else { remaining / 2.5 }
    } else {
        let below = remaining + 3.0;
        if below > 0.0 { below / 3.0 } else { below / 10.0 }
    };

    let noise = if (-2.0..=2.0).contains(&gradient) {
        barrier.get()?
    } else {
        0.0
    };
    Ok(2.0 * (gradient + noise))
}

/// Decide only whether the terrain density becomes positive due to pressure.
/// Every pair is checked separately in closest-second, closest-third,
/// second-third order. The pressure values must NOT be summed or maxed before
/// adding density, since the first positive pair exits immediately.
///
/// The caller is responsible for the global-lava and water-over-lava early
/// returns *before* this helper, as well as the 1.20.1 fluid-update flag.
/// When the first two sites are separated by at least 25 squared-distance
/// units, no noise is read. This method does not infer fluid status from 26.3.
pub fn legacy1201_pressure_outcome<F>(
    position: BlockPos,
    density: f64,
    sites: &[Legacy1201AquiferSite; 3],
    statuses: [Legacy1201FluidStatus; 3],
    air: BlockStateId,
    water: BlockStateId,
    lava: BlockStateId,
    sample_barrier: &mut F,
) -> io::Result<Legacy1201PressureOutcome>
where
    F: FnMut(BlockPos) -> io::Result<f64>,
{
    if !sites.iter().all(|site| site.squared_distance >= 0)
        || sites[0].squared_distance > sites[1].squared_distance
        || sites[1].squared_distance > sites[2].squared_distance
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "1.20.1 aquifer sites must have ordered non-negative distances",
        ));
    }
    if density > 0.0 {
        return Ok(Legacy1201PressureOutcome::Solid);
    }

    let weights = legacy1201_aquifer_boundary_weights(sites);
    if weights.nearest_second <= 0.0 {
        return Ok(Legacy1201PressureOutcome::Open);
    }
    let mut barrier = Legacy1201BarrierNoise::new(position, sample_barrier);
    let pairs = [
        (0, 1, weights.nearest_second),
        (0, 2, weights.nearest_third),
        (1, 2, weights.second_third),
    ];
    for (a, b, weight) in pairs {
        if weight <= 0.0 {
            continue;
        }
        let pressure = legacy1201_pair_pressure(
            position.y, statuses[a], statuses[b], air, water, lava, &mut barrier,
        )?;
        if density + weight * pressure > 0.0 {
            return Ok(Legacy1201PressureOutcome::Solid);
        }
    }
    Ok(Legacy1201PressureOutcome::Open)
}

#[cfg(test)]
mod tests {
    use super::*;

    const AIR: BlockStateId = BlockStateId(0);
    const WATER: BlockStateId = BlockStateId(1);
    const LAVA: BlockStateId = BlockStateId(2);

    fn water(level: i32) -> Legacy1201FluidStatus {
        Legacy1201FluidStatus { level, state: WATER }
    }

    fn lava(level: i32) -> Legacy1201FluidStatus {
        Legacy1201FluidStatus { level, state: LAVA }
    }

    fn sites(distances: [i32; 3]) -> [Legacy1201AquiferSite; 3] {
        distances.map(|distance| Legacy1201AquiferSite {
            cell: BlockPos::default(), position: BlockPos::default(),
            squared_distance: distance,
        })
    }

    #[test]
    fn equal_levels_do_not_sample_barrier() {
        let mut count = 0;
        let mut sampler = |_| { count += 1; Ok(0.25) };
        let mut barrier = Legacy1201BarrierNoise::new(BlockPos::new(0, 60, 0), &mut sampler);
        assert_eq!(legacy1201_pair_pressure(60, water(63), water(63), AIR, WATER, LAVA, &mut barrier).unwrap(), 0.0);
        assert!(!barrier.sampled());
        assert_eq!(count, 0);
    }

    #[test]
    fn water_lava_interface_has_fixed_pressure_in_both_orders() {
        let mut sampler = |_| -> io::Result<f64> { panic!("barrier must not be sampled") };
        let mut cache = Legacy1201BarrierNoise::new(BlockPos::new(0, -50, 0), &mut sampler);
        assert_eq!(legacy1201_pair_pressure(-50, water(63), lava(-49), AIR, WATER, LAVA, &mut cache).unwrap(), 2.0);
        assert_eq!(legacy1201_pair_pressure(-50, lava(-49), water(63), AIR, WATER, LAVA, &mut cache).unwrap(), 2.0);
    }

    #[test]
    fn no_lava_interface_if_one_table_is_dry_here() {
        let mut sampler = |_| Ok(0.0);
        let mut cache = Legacy1201BarrierNoise::new(BlockPos::new(0, 0, 0), &mut sampler);
        assert_ne!(legacy1201_pair_pressure(0, water(0), lava(10), AIR, WATER, LAVA, &mut cache).unwrap(), 2.0);
    }

    #[test]
    fn high_and_low_gradient_skip_noise() {
        let mut sampler = |_| -> io::Result<f64> { panic!("gradient outside band") };
        let mut cache = Legacy1201BarrierNoise::new(BlockPos::new(0, 0, 0), &mut sampler);
        let high = legacy1201_pair_pressure(30, water(4), water(14), AIR, WATER, LAVA, &mut cache).unwrap();
        let low = legacy1201_pair_pressure(-200, water(4), water(14), AIR, WATER, LAVA, &mut cache).unwrap();
        assert!(high < -2.0);
        assert!(low < -2.0);
        assert!(!cache.sampled());
    }

    #[test]
    fn barrier_noise_is_lazy_and_cached_within_position() {
        let mut count = 0;
        let mut sampler = |position: BlockPos| { count += 1; assert_eq!(position.x, -7); Ok(0.25) };
        let mut cache = Legacy1201BarrierNoise::new(BlockPos::new(-7, 14, 4), &mut sampler);
        let a = legacy1201_pair_pressure(14, water(12), water(20), AIR, WATER, LAVA, &mut cache).unwrap();
        let b = legacy1201_pair_pressure(14, water(10), water(17), AIR, WATER, LAVA, &mut cache).unwrap();
        assert!(a.is_finite() && b.is_finite());
        assert!(cache.sampled());
        assert_eq!(count, 1);
    }

    #[test]
    fn failed_samples_do_not_poison_barrier_cache() {
        let mut count = 0;
        let mut sampler = |_| {
            count += 1;
            if count == 1 { Err(io::Error::new(io::ErrorKind::WouldBlock, "not ready")) }
            else { Ok(0.0) }
        };
        let mut cache = Legacy1201BarrierNoise::new(BlockPos::new(1, 14, 1), &mut sampler);
        assert_eq!(legacy1201_pair_pressure(14, water(12), water(20), AIR, WATER, LAVA, &mut cache).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert!(!cache.sampled());
        assert!(legacy1201_pair_pressure(14, water(12), water(20), AIR, WATER, LAVA, &mut cache).is_ok());
        assert!(cache.sampled());
        assert_eq!(count, 2);
    }

    #[test]
    fn nonfinite_noise_is_an_error_not_a_dry_fallback() {
        let mut sampler = |_| Ok(f64::NAN);
        let mut cache = Legacy1201BarrierNoise::new(BlockPos::new(0, 14, 0), &mut sampler);
        assert_eq!(legacy1201_pair_pressure(14, water(12), water(20), AIR, WATER, LAVA, &mut cache).unwrap_err().kind(), io::ErrorKind::InvalidData);
        assert!(!cache.sampled());
    }

    #[test]
    fn density_and_distance_exits_are_lazy() {
        let mut sampler = |_| -> io::Result<f64> { panic!("no barrier expected") };
        let close = sites([10, 12, 15]);
        assert_eq!(legacy1201_pressure_outcome(BlockPos::default(), 0.1, &close, [water(4), water(5), water(6)], AIR, WATER, LAVA, &mut sampler).unwrap(), Legacy1201PressureOutcome::Solid);
        let far = sites([10, 35, 40]);
        assert_eq!(legacy1201_pressure_outcome(BlockPos::default(), -0.2, &far, [water(4), water(5), water(6)], AIR, WATER, LAVA, &mut sampler).unwrap(), Legacy1201PressureOutcome::Open);
    }

    #[test]
    fn pressure_can_make_stone_between_water_and_lava() {
        let mut sampler = |_| -> io::Result<f64> { panic!("lava-water does not read barrier") };
        assert_eq!(legacy1201_pressure_outcome(BlockPos::new(0, 5, 0), -0.25, &sites([10, 10, 10]), [water(20), lava(20), water(20)], AIR, WATER, LAVA, &mut sampler).unwrap(), Legacy1201PressureOutcome::Solid);
    }

    #[test]
    fn density_is_compared_to_each_weighted_pair_not_their_sum() {
        let mut sampler = |_| Ok(0.0);
        // A maximum pressure of 2 and sim12=0.4 gives 0.8; with density=-1.0
        // this must stay open even when two pairs each contribute 0.8.
        let s = sites([10, 25, 25]);
        let outcome = legacy1201_pressure_outcome(BlockPos::new(0, 5, 0), -1.0, &s,
            [water(20), lava(20), lava(20)], AIR, WATER, LAVA, &mut sampler).unwrap();
        assert_eq!(outcome, Legacy1201PressureOutcome::Open);
    }

    #[test]
    fn malformed_site_order_is_rejected() {
        let mut sampler = |_| Ok(0.0);
        let err = legacy1201_pressure_outcome(BlockPos::default(), 0.0, &sites([10, 9, 11]),
            [water(1), water(2), water(3)], AIR, WATER, LAVA, &mut sampler).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }
}
