//! Preliminary-surface survey for Minecraft Java 1.20.1 aquifer cells.
//!
//! This is the 13-column precheck performed before floodedness and spread
//! noises. It deliberately requests real 1.20.1 `preliminary_surface_level`
//! samples from the caller; no height map or 26.3 noise is substituted.
//! A survey can exit early with a global fluid status. Callers MUST honor
//! that return without sampling any further surface columns or local noise.

use crate::legacy_1_20_1_aquifer::Legacy1201FluidStatus;
use horizon_core::position::BlockPos;
use std::{collections::HashMap, io};

/// Original visit order: center first, then twelve offset columns, in chunks.
/// The offsets are applied to the randomized aquifer site's *block* position.
pub const LEGACY1201_SURFACE_OFFSETS_CHUNKS: [(i32, i32); 13] = [
    (0, 0),
    (-2, -1), (-1, -1), (0, -1), (1, -1),
    (-3, 0), (-2, 0), (-1, 0), (1, 0),
    (-2, 1), (-1, 1), (0, 1), (1, 1),
];

/// A global-fluid early exit cannot be treated as a partially flooded cell.
/// The local outcome supplies the raw minimum preliminary surface (without +8).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201SurfaceSurvey {
    Global(Legacy1201FluidStatus),
    Local {
        lowest_preliminary: i32,
        surface_under_global: bool,
    },
}

fn bad_coordinate() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, "1.20.1 aquifer surface coordinate overflow")
}

/// Survey the 13 preliminary-surface columns, preserving the center's special
/// rules, short-circuit priority, and original sampling order.
///
/// `sample_surface(x,z)` returns a 1.20.1 NoiseChunk preliminary height at
/// WORLD block coordinates. `global_at(y)` is the dimension's 1.20.1 global
/// fluid picker at Y. The latter is deliberately Y-only, not a 26.3 sampler.
/// Neither callback may silently return synthesized values for missing data.
///
/// Arithmetic rejects unrepresentable coordinates rather than wrapping to
/// a different chunk. This does not affect Minecraft's legal world border.
pub fn legacy1201_survey_aquifer_surface<S, G>(
    site: BlockPos,
    sample_surface: &mut S,
    global_at: &mut G,
) -> io::Result<Legacy1201SurfaceSurvey>
where
    S: FnMut(i32, i32) -> io::Result<i32>,
    G: FnMut(i32) -> Legacy1201FluidStatus,
{
    let top = site.y.checked_add(12).ok_or_else(bad_coordinate)?;
    let bottom = site.y.checked_sub(12).ok_or_else(bad_coordinate)?;
    let mut lowest = i32::MAX;
    let mut center_under_global = false;

    for (index, (dx, dz)) in LEGACY1201_SURFACE_OFFSETS_CHUNKS.iter().copied().enumerate() {
        let x = site.x.checked_add(dx * 16).ok_or_else(bad_coordinate)?;
        let z = site.z.checked_add(dz * 16).ok_or_else(bad_coordinate)?;
        let preliminary = sample_surface(x, z)?;
        let adjusted = preliminary.checked_add(8).ok_or_else(bad_coordinate)?;
        let center = index == 0;

        // When the site lies well ABOVE its own preliminary surface, the
        // global picker alone determines its table (no peripheral samples).
        if center && bottom > adjusted {
            return Ok(Legacy1201SurfaceSurvey::Global(global_at(site.y)));
        }

        let near_surface = top > adjusted;
        if center || near_surface {
            let global = global_at(adjusted);
            if adjusted < global.level {
                if center {
                    center_under_global = true;
                }
                // An exposed flooded column wins even when it is not center.
                if near_surface {
                    return Ok(Legacy1201SurfaceSurvey::Global(global));
                }
            }
        }
        lowest = lowest.min(preliminary);
    }

    Ok(Legacy1201SurfaceSurvey::Local {
        lowest_preliminary: lowest,
        surface_under_global: center_under_global,
    })
}

/// A chunk-scoped memo for PRELIMINARY surface samples, keyed by the exact
/// world-column coordinates passed to `sample_surface`. The underlying 1.20.1
/// NoiseChunk remains responsible for any quart-position normalization.
/// No failure is cached; reset this cache when the seed/router changes.
pub struct Legacy1201PreliminarySurfaceCache<F>
where
    F: FnMut(i32, i32) -> io::Result<i32>,
{
    sample: F,
    columns: HashMap<(i32, i32), i32>,
}

impl<F> Legacy1201PreliminarySurfaceCache<F>
where
    F: FnMut(i32, i32) -> io::Result<i32>,
{
    pub fn new(sample: F) -> Self {
        Self { sample, columns: HashMap::new() }
    }

    pub fn len(&self) -> usize { self.columns.len() }

    pub fn is_empty(&self) -> bool { self.columns.is_empty() }

    pub fn clear(&mut self) { self.columns.clear(); }

    pub fn get(&mut self, x: i32, z: i32) -> io::Result<i32> {
        if let Some(height) = self.columns.get(&(x, z)) {
            return Ok(*height);
        }
        let value = (self.sample)(x, z)?;
        self.columns.insert((x, z), value);
        Ok(value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::BlockStateId;

    const WATER: BlockStateId = BlockStateId(1);
    const LAVA: BlockStateId = BlockStateId(2);
    fn global(_y: i32) -> Legacy1201FluidStatus {
        Legacy1201FluidStatus { level: 63, state: WATER }
    }

    #[test]
    fn samples_all_thirteen_offsets_in_order_including_negative_coordinates() {
        let site = BlockPos::new(-7, 0, -11);
        let mut called = Vec::new();
        let mut surface = |x, z| { called.push((x, z)); Ok(30) };
        let mut global = |_| Legacy1201FluidStatus { level: -10_000, state: LAVA };
        let survey = legacy1201_survey_aquifer_surface(site, &mut surface, &mut global).unwrap();
        assert_eq!(survey, Legacy1201SurfaceSurvey::Local {
            lowest_preliminary: 30, surface_under_global: false,
        });
        drop(surface);
        let expected: Vec<_> = LEGACY1201_SURFACE_OFFSETS_CHUNKS.iter().copied()
            .map(|(dx, dz)| (site.x + dx * 16, site.z + dz * 16)).collect();
        assert_eq!(called, expected);
    }

    #[test]
    fn center_above_surface_exits_without_other_samples() {
        let mut count = 0;
        let mut surface = |_, _| { count += 1; Ok(-20) };
        let mut picker = global;
        let status = legacy1201_survey_aquifer_surface(
            BlockPos::new(4, 45, 8), &mut surface, &mut picker,
        ).unwrap();
        assert_eq!(status, Legacy1201SurfaceSurvey::Global(global(45)));
        drop(surface);
        assert_eq!(count, 1);
    }

    #[test]
    fn off_center_flooded_column_exits_on_second_sample() {
        let mut calls = 0;
        let mut surface = |_, _| {
            calls += 1;
            Ok(if calls == 1 { 30 } else { -20 })
        };
        let mut picker = global;
        let status = legacy1201_survey_aquifer_surface(
            BlockPos::new(8, 0, 3), &mut surface, &mut picker,
        ).unwrap();
        assert_eq!(status, Legacy1201SurfaceSurvey::Global(global(-12)));
        drop(surface);
        assert_eq!(calls, 2);
    }

    #[test]
    fn center_under_global_is_not_set_by_peripheral_columns() {
        let mut calls = 0;
        let mut surface = |_, _| {
            calls += 1;
            Ok(if calls == 1 { 30 } else { 31 })
        };
        let mut picker = global;
        let status = legacy1201_survey_aquifer_surface(
            BlockPos::new(8, 0, 3), &mut surface, &mut picker,
        ).unwrap();
        assert_eq!(status, Legacy1201SurfaceSurvey::Local {
            lowest_preliminary: 30,
            surface_under_global: true,
        });
        drop(surface);
        assert_eq!(calls, 13);
    }

    #[test]
    fn cache_never_stores_an_error() {
        let mut calls = 0;
        let mut cache = Legacy1201PreliminarySurfaceCache::new(|_, _| {
            calls += 1;
            if calls == 1 { Err(io::Error::new(io::ErrorKind::WouldBlock, "retry")) }
            else { Ok(42) }
        });
        assert_eq!(cache.get(-16, 15).unwrap_err().kind(), io::ErrorKind::WouldBlock);
        assert!(cache.is_empty());
        assert_eq!(cache.get(-16, 15).unwrap(), 42);
        assert_eq!(cache.get(-16, 15).unwrap(), 42);
        assert_eq!(cache.len(), 1);
        cache.clear();
        assert_eq!(cache.len(), 0);
        assert_eq!(cache.get(-16, 15).unwrap(), 42);
        drop(cache);
        assert_eq!(calls, 3);
    }

    #[test]
    fn surface_sampling_errors_do_not_fall_back_to_zero_height() {
        let mut surface = |_, _| Err(io::Error::new(io::ErrorKind::NotFound, "missing noise"));
        let mut picker = global;
        assert_eq!(legacy1201_survey_aquifer_surface(
            BlockPos::new(0, 0, 0), &mut surface, &mut picker,
        ).unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn overflowing_coordinates_are_rejected_before_sampling() {
        let mut surface = |_, _| -> io::Result<i32> { panic!("must not sample") };
        let mut picker = global;
        assert_eq!(legacy1201_survey_aquifer_surface(
            BlockPos::new(i32::MAX, i32::MIN, 0), &mut surface, &mut picker,
        ).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
}
