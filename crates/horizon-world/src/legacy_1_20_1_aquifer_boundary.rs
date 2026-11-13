//! Chunk-scoped positional/status cache and interface weights for the 1.20.1
//! noise-based aquifer. This is NOT an implementation of its surface sampler,
//! pressure calculation, or final `computeSubstance` decision.
//!
//! Both the site positions and the fluid tables belong to the 1.20.1 seed and
//! noise router. The caller must create a fresh cache for each noise chunk and
//! must supply a 1.20.1 fluid-status sampler. Nothing in this module reads the
//! 26.3 terrain pipeline, guesses statuses, or installs a fallback aquifer.

use crate::legacy_1_20_1_aquifer::{
    insert_nearest, legacy1201_aquifer_anchor, legacy1201_aquifer_site,
    Legacy1201AquiferSite, Legacy1201FluidStatus,
};
use horizon_core::{position::BlockPos, random::PositionalRandom};
use std::{collections::HashMap, io};

type CellKey = (i32, i32, i32);

fn cell_key(pos: BlockPos) -> CellKey {
    (pos.x, pos.y, pos.z)
}

/// The three pair weights that multiply the actual 1.20.1 pressure for each
/// neighboring fluid table. A weight of zero means no pressure query is needed.
/// This is a query PLAN, not a substitute for the pressure/noise calculation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Legacy1201AquiferBoundaryWeights {
    pub nearest_second: f64,
    pub nearest_third: f64,
    pub second_third: f64,
}

/// The 1.20.1 similarity convention for squared distances between aquifer
/// centers. Distances should be non-negative and ordered before use. At a
/// difference of 25, pressure between those two cells is no longer considered.
pub fn legacy1201_aquifer_similarity(first: i32, second: i32) -> f64 {
    1.0 - (i64::from(second) - i64::from(first)).abs() as f64 / 25.0
}

/// Plan the possible pressure calls for the closest three centers, in the
/// original 1.20.1 order (1-2, 1-3, 2-3). This function never samples a noise
/// function. The later pressure calculator must still compare the final
/// terrain density with the resulting barriers, preserving the same ordering.
pub fn legacy1201_aquifer_boundary_weights(
    nearest: &[Legacy1201AquiferSite; 3],
) -> Legacy1201AquiferBoundaryWeights {
    let sim12 = legacy1201_aquifer_similarity(
        nearest[0].squared_distance, nearest[1].squared_distance,
    );
    if sim12 <= 0.0 {
        return Legacy1201AquiferBoundaryWeights {
            nearest_second: 0.0,
            nearest_third: 0.0,
            second_third: 0.0,
        };
    }
    let sim13 = legacy1201_aquifer_similarity(
        nearest[0].squared_distance, nearest[2].squared_distance,
    );
    let sim23 = legacy1201_aquifer_similarity(
        nearest[1].squared_distance, nearest[2].squared_distance,
    );
    Legacy1201AquiferBoundaryWeights {
        nearest_second: sim12,
        nearest_third: if sim13 > 0.0 { sim12 * sim13 } else { 0.0 },
        second_third: if sim23 > 0.0 { sim12 * sim23 } else { 0.0 },
    }
}

/// Memory is deliberately bounded by the owning chunk's lifetime, not by a
/// process-global static. Call `clear` before reusing this cache with a different
/// seed or noise-router setup (or, preferably, create a new instance).
pub struct Legacy1201AquiferCellCache<'r, R: PositionalRandom> {
    random: &'r R,
    sites: HashMap<CellKey, BlockPos>,
    statuses: HashMap<CellKey, Legacy1201FluidStatus>,
}

impl<'r, R: PositionalRandom> Legacy1201AquiferCellCache<'r, R> {
    pub fn new(random: &'r R) -> Self {
        Self {
            random,
            sites: HashMap::new(),
            statuses: HashMap::new(),
        }
    }

    pub fn site_count(&self) -> usize { self.sites.len() }

    pub fn status_count(&self) -> usize { self.statuses.len() }

    pub fn clear(&mut self) {
        self.sites.clear();
        self.statuses.clear();
    }

    /// Return the site for this source grid cell, without reseeding the RNG on
    /// subsequent visits. A tuple key prevents packed-BlockPos aliasing when a
    /// caller inspects unusual coordinates; site coordinates retain Java's
    /// 64-bit BlockPos packing semantics from the base primitive.
    pub fn site(&mut self, cell: BlockPos) -> BlockPos {
        let key = cell_key(cell);
        if let Some(existing) = self.sites.get(&key) {
            return *existing;
        }
        let generated = legacy1201_aquifer_site(self.random, cell);
        self.sites.insert(key, generated);
        generated
    }

    /// The same 2 x 3 x 2 traversal and <= tie policy as the uncached 1.5.9
    /// implementation. The caller gets source cells, site positions and the
    /// squared distances, so status lookup cannot accidentally key by query Y.
    pub fn nearest(&mut self, position: BlockPos) -> [Legacy1201AquiferSite; 3] {
        let anchor = legacy1201_aquifer_anchor(position);
        let empty = Legacy1201AquiferSite {
            cell: BlockPos::default(),
            position: BlockPos::default(),
            squared_distance: i32::MAX,
        };
        let mut best = [empty; 3];
        for dx in 0..=1i32 {
            for dy in -1..=1i32 {
                for dz in 0..=1i32 {
                    let cell = BlockPos::new(
                        anchor.x.wrapping_add(dx),
                        anchor.y.wrapping_add(dy),
                        anchor.z.wrapping_add(dz),
                    );
                    let site = self.site(cell);
                    let vx = site.x.wrapping_sub(position.x);
                    let vy = site.y.wrapping_sub(position.y);
                    let vz = site.z.wrapping_sub(position.z);
                    let squared_distance = vx.wrapping_mul(vx)
                        .wrapping_add(vy.wrapping_mul(vy))
                        .wrapping_add(vz.wrapping_mul(vz));
                    insert_nearest(&mut best, Legacy1201AquiferSite {
                        cell, position: site, squared_distance,
                    });
                }
            }
        }
        best
    }

    /// Cache the *real* 1.20.1 fluid status of a site. A failed sample is never
    /// cached: retrying after missing noise/chunk data must not manufacture a
    /// dry cell. A mismatched site/cell pair is rejected rather than silently
    /// returning a cached status from some unrelated position.
    pub fn status_at<F>(
        &mut self,
        site: Legacy1201AquiferSite,
        sampler: &mut F,
    ) -> io::Result<Legacy1201FluidStatus>
    where
        F: FnMut(BlockPos) -> io::Result<Legacy1201FluidStatus>,
    {
        let actual = self.site(site.cell);
        if actual != site.position {
            return Err(io::Error::new(io::ErrorKind::InvalidInput,
                "1.20.1 aquifer site/cell mismatch"));
        }
        let key = cell_key(site.cell);
        if let Some(status) = self.statuses.get(&key) {
            return Ok(*status);
        }
        let computed = sampler(site.position)?;
        self.statuses.insert(key, computed);
        Ok(computed)
    }

    /// Explicitly fetch all three tables for a candidate boundary. The final
    /// 1.20.1 pressure logic may instead fetch them lazily after its early
    /// returns; this helper is intended for differential probes and tests.
    pub fn nearest_with_statuses<F>(
        &mut self,
        position: BlockPos,
        sampler: &mut F,
    ) -> io::Result<[(Legacy1201AquiferSite, Legacy1201FluidStatus); 3]>
    where
        F: FnMut(BlockPos) -> io::Result<Legacy1201FluidStatus>,
    {
        let sites = self.nearest(position);
        Ok([
            (sites[0], self.status_at(sites[0], sampler)?),
            (sites[1], self.status_at(sites[1], sampler)?),
            (sites[2], self.status_at(sites[2], sampler)?),
        ])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::legacy_1_20_1_aquifer::legacy1201_nearest_aquifer_sites;
    use crate::registry::BlockStateId;
    use horizon_core::random::{LegacyPositionalRandom, XoroshiroPositionalRandom};

    fn synth(site: BlockPos) -> Legacy1201FluidStatus {
        Legacy1201FluidStatus {
            level: site.y.div_euclid(3),
            state: BlockStateId(site.x.rem_euclid(9) as u32),
        }
    }

    fn fake_site(distance: i32) -> Legacy1201AquiferSite {
        Legacy1201AquiferSite {
            cell: BlockPos::default(), position: BlockPos::default(),
            squared_distance: distance,
        }
    }

    #[test]
    fn cached_and_uncached_nearest_are_identical_on_negative_axes() {
        for seed in [0, -1, 12_345, i64::MIN, i64::MAX] {
            let rng = LegacyPositionalRandom::new(seed);
            let mut cache = Legacy1201AquiferCellCache::new(&rng);
            for x in [-64, -17, -16, -11, -1, 0, 5, 16, 71] {
                for y in [-64, -53, -1, 0, 63, 128, 319] {
                    for z in [-48, -11, -1, 0, 16, 57] {
                        let p = BlockPos::new(x, y, z);
                        assert_eq!(cache.nearest(p), legacy1201_nearest_aquifer_sites(&rng, p));
                    }
                }
            }
        }
    }

    #[test]
    fn source_cell_cache_reuses_sites_after_repeated_queries() {
        let rng = LegacyPositionalRandom::new(17);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        let p = BlockPos::new(-20, 60, 19);
        let once = cache.nearest(p);
        assert_eq!(cache.site_count(), 12);
        assert_eq!(cache.nearest(p), once);
        assert_eq!(cache.site_count(), 12);
        assert_eq!(cache.status_count(), 0);
    }

    #[test]
    fn failed_status_sampling_does_not_poison_the_cache() {
        let rng = LegacyPositionalRandom::new(27);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        let site = cache.nearest(BlockPos::new(8, 63, 8))[0];
        let mut attempts = 0;
        let mut flaky = |at: BlockPos| {
            attempts += 1;
            if attempts == 1 {
                Err(io::Error::new(io::ErrorKind::WouldBlock, "noise missing"))
            } else {
                Ok(synth(at))
            }
        };
        assert!(cache.status_at(site, &mut flaky).is_err());
        assert_eq!(cache.status_count(), 0);
        assert_eq!(cache.status_at(site, &mut flaky).unwrap(), synth(site.position));
        assert_eq!(cache.status_at(site, &mut flaky).unwrap(), synth(site.position));
        assert_eq!(attempts, 2);
    }

    #[test]
    fn mismatched_position_must_not_alias_cached_cell() {
        let rng = LegacyPositionalRandom::new(27);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        let mut site = cache.nearest(BlockPos::new(0, 60, 0))[0];
        let mut sampler = |at| Ok(synth(at));
        let good = cache.status_at(site, &mut sampler).unwrap();
        site.position = BlockPos::new(site.position.x, site.position.y + 1, site.position.z);
        assert_eq!(cache.status_at(site, &mut sampler).unwrap_err().kind(),
            io::ErrorKind::InvalidInput);
        assert_eq!(cache.status_count(), 1);
        assert_ne!(good.level, i32::MIN);
    }

    #[test]
    fn three_statuses_are_cached_by_cell_not_by_query_position() {
        let rng = LegacyPositionalRandom::new(0);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        let p = BlockPos::new(0, 63, 0);
        // A Cell lets the test inspect the counter while the sampler closure
        // is still alive, without conflicting mutable and immutable borrows.
        let calls = std::cell::Cell::new(0);
        let mut sampler = |pos| {
            calls.set(calls.get() + 1);
            Ok(synth(pos))
        };
        let first = cache.nearest_with_statuses(p, &mut sampler).unwrap();
        assert_eq!(calls.get(), 3);
        assert_eq!(cache.status_count(), 3);
        assert_eq!(cache.nearest_with_statuses(p, &mut sampler).unwrap(), first);
        assert_eq!(calls.get(), 3);
        for (site, status) in first { assert_eq!(status, synth(site.position)); }
    }

    #[test]
    fn repeated_query_after_clear_restarts_only_its_caches() {
        let rng = LegacyPositionalRandom::new(11);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        let a = cache.nearest(BlockPos::new(1, 2, 3));
        let mut sampler = |pos| Ok(synth(pos));
        let _ = cache.status_at(a[0], &mut sampler).unwrap();
        assert_eq!(cache.status_count(), 1);
        cache.clear();
        assert_eq!((cache.site_count(), cache.status_count()), (0, 0));
        assert_eq!(cache.nearest(BlockPos::new(1, 2, 3)), a);
    }

    #[test]
    fn xoroshiro_source_has_the_same_cache_contract() {
        let rng = XoroshiroPositionalRandom::new(7, 12);
        let mut cache = Legacy1201AquiferCellCache::new(&rng);
        for pos in [BlockPos::new(0, -64, 0), BlockPos::new(17, 63, -31)] {
            assert_eq!(cache.nearest(pos), legacy1201_nearest_aquifer_sites(&rng, pos));
        }
    }

    #[test]
    fn weights_are_zero_outside_the_25_squared_distance_band() {
        let w = legacy1201_aquifer_boundary_weights(&[
            fake_site(10), fake_site(35), fake_site(59),
        ]);
        assert_eq!(w, Legacy1201AquiferBoundaryWeights {
            nearest_second: 0.0, nearest_third: 0.0, second_third: 0.0,
        });
    }

    #[test]
    fn pressure_pair_weights_preserve_three_pair_order() {
        let w = legacy1201_aquifer_boundary_weights(&[
            fake_site(10), fake_site(20), fake_site(32),
        ]);
        assert!((w.nearest_second - 0.6).abs() < 1e-12);
        assert!((w.nearest_third - 0.072).abs() < 1e-12);
        assert!((w.second_third - 0.312).abs() < 1e-12);
    }

    #[test]
    fn tied_nearest_centers_give_full_pair_weight() {
        let w = legacy1201_aquifer_boundary_weights(&[
            fake_site(15), fake_site(15), fake_site(15),
        ]);
        assert_eq!(w.nearest_second, 1.0);
        assert_eq!(w.nearest_third, 1.0);
        assert_eq!(w.second_third, 1.0);
    }

    #[test]
    fn similarity_is_stable_at_the_inclusive_and_exclusive_limits() {
        for (a,b,expected) in [(10,10,1.0), (10,20,0.6), (10,34,0.04),
                               (10,35,0.0), (10,36,-0.04)] {
            assert!((legacy1201_aquifer_similarity(a,b) - expected).abs() < 1e-12);
        }
    }
}
