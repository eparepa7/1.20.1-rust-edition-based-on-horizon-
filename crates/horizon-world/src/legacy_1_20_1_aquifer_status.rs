//! Minecraft Java 1.20.1 noise-aquifer CELL status assembly.
//!
//! Connects the preliminary-surface survey to the 1.5.16 floodedness,
//! fluid-level spread and lava noise primitives. The caller must supply the
//! version-correct noise router and global fluid picker. This produces a
//! fluid table for one randomized aquifer SITE; it does not perform the final
//! block-density/pressure decision or enable 1.20.1 world generation.

use crate::{
    legacy_1_20_1_aquifer::{
        legacy1201_aquifer_floodedness, legacy1201_aquifer_fluid_type,
        legacy1201_randomized_fluid_surface, Legacy1201Floodedness,
        Legacy1201FluidStatus, LEGACY1201_DRY_FLUID_LEVEL,
    },
    legacy_1_20_1_aquifer_surface::{
        legacy1201_survey_aquifer_surface, Legacy1201SurfaceSurvey,
    },
    registry::BlockStateId,
};
use horizon_core::position::BlockPos;
use std::io;

/// Compute a single 1.20.1 aquifer site's fluid table from real 1.20.1 data.
///
/// `global_at(y)` is the same validated dimension picker used for carving.
/// `sample_surface(x,z)` is the NoiseChunk preliminary-surface calculation at
/// *world block coordinates*. `deep_dark`, `floodedness` sample the site in
/// world block coordinates; `spread` and `lava_noise` sample the quantized
/// noise-grid coordinates prepared by the 1.5.16 helpers.
///
/// Order is important: no density noise is sampled on a survey global exit;
/// deep-dark excludes floodedness; spread is only needed for Randomized;
/// lava is only sampled when the 1.5.16 fluid-type rule requires it.
///
/// This pure composition assumes `lava_state` and global-status state IDs
/// belong to the SAME validated 1.20.1 registry. It must not be called using
/// the 26.3 registry, block IDs or noise router.
#[allow(clippy::too_many_arguments)]
pub fn legacy1201_compute_aquifer_cell_status<P, G, D, F, S, L>(
    site: BlockPos,
    lava_state: BlockStateId,
    sample_surface: &mut P,
    global_at: &mut G,
    deep_dark: &mut D,
    floodedness: &mut F,
    spread: &mut S,
    lava_noise: &mut L,
) -> io::Result<Legacy1201FluidStatus>
where
    P: FnMut(i32, i32) -> io::Result<i32>,
    G: FnMut(i32) -> Legacy1201FluidStatus,
    D: FnMut(BlockPos) -> io::Result<bool>,
    F: FnMut(BlockPos) -> io::Result<f64>,
    S: FnMut(BlockPos) -> io::Result<f64>,
    L: FnMut(BlockPos) -> io::Result<f64>,
{
    let (lowest_preliminary, surface_under_global) =
        match legacy1201_survey_aquifer_surface(site, sample_surface, global_at)? {
            Legacy1201SurfaceSurvey::Global(status) => return Ok(status),
            Legacy1201SurfaceSurvey::Local {
                lowest_preliminary,
                surface_under_global,
            } => (lowest_preliminary, surface_under_global),
        };

    let global = global_at(site.y);
    let is_deep_dark = deep_dark(site)?;
    let floodedness_value = if is_deep_dark {
        // The helper exits for Deep Dark before reading this value.
        f64::NAN
    } else {
        floodedness(site)?
    };
    let mode = legacy1201_aquifer_floodedness(
        site.y, lowest_preliminary, surface_under_global,
        is_deep_dark, floodedness_value,
    )?;
    let level = match mode {
        Legacy1201Floodedness::Flooded => global.level,
        Legacy1201Floodedness::Randomized =>
            legacy1201_randomized_fluid_surface(site, lowest_preliminary, spread)?,
        Legacy1201Floodedness::Dry => LEGACY1201_DRY_FLUID_LEVEL,
    };
    let state = legacy1201_aquifer_fluid_type(
        site, level, global, lava_state, lava_noise,
    )?;
    Ok(Legacy1201FluidStatus { level, state })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::BlockStateId;

    const WATER: BlockStateId = BlockStateId(1);
    const LAVA: BlockStateId = BlockStateId(2);

    fn global(y: i32) -> Legacy1201FluidStatus {
        if y < -54 {
            Legacy1201FluidStatus { level: -54, state: LAVA }
        } else {
            Legacy1201FluidStatus { level: 63, state: WATER }
        }
    }

    #[test]
    fn global_surface_short_circuit_samples_no_local_noise() {
        let mut surfaces = 0;
        let mut preliminary = |_, _| { surfaces += 1; Ok(-40) };
        let mut no_dark = |_| -> io::Result<bool> { panic!("no deep dark probe") };
        let mut no_flood = |_| -> io::Result<f64> { panic!("no floodedness") };
        let mut no_spread = |_| -> io::Result<f64> { panic!("no spread") };
        let mut no_lava = |_| -> io::Result<f64> { panic!("no lava") };
        let mut picker = global;
        let actual = legacy1201_compute_aquifer_cell_status(
            BlockPos::new(7, 32, -5), LAVA,
            &mut preliminary, &mut picker, &mut no_dark,
            &mut no_flood, &mut no_spread, &mut no_lava,
        ).unwrap();
        assert_eq!(actual, global(32));
        drop(preliminary);
        assert_eq!(surfaces, 1);
    }

    #[test]
    fn flooded_mode_uses_global_level_without_spread_or_lava_noise() {
        let mut surface = |_, _| Ok(100);
        let mut dark = |_| Ok(false);
        let mut flood = |_| Ok(1.0);
        let mut no_spread = |_| -> io::Result<f64> { panic!("flooded must skip spread") };
        let mut no_lava = |_| -> io::Result<f64> { panic!("above -10 must skip lava") };
        let mut picker = global;
        let result = legacy1201_compute_aquifer_cell_status(
            BlockPos::new(-5, 0, 8), LAVA, &mut surface, &mut picker,
            &mut dark, &mut flood, &mut no_spread, &mut no_lava,
        ).unwrap();
        assert_eq!(result, Legacy1201FluidStatus { level: 63, state: WATER });
    }

    #[test]
    fn randomized_mode_uses_quantized_noise_cells_and_lava_noise() {
        let mut surface = |_, _| Ok(50);
        let mut dark = |_| Ok(false);
        let mut floodedness = |_| Ok(0.5);
        let mut spread_calls = 0;
        let mut spread = |pos: BlockPos| {
            assert_eq!(pos, BlockPos::new(-1, -1, 0));
            spread_calls += 1;
            Ok(0.0)
        };
        let mut lava_calls = 0;
        let mut lava = |pos: BlockPos| {
            assert_eq!(pos, BlockPos::new(-1, -1, 0));
            lava_calls += 1;
            Ok(0.4)
        };
        let mut picker = global;
        let result = legacy1201_compute_aquifer_cell_status(
            BlockPos::new(-5, -33, 8), LAVA, &mut surface, &mut picker,
            &mut dark, &mut floodedness, &mut spread, &mut lava,
        ).unwrap();
        assert_eq!(result, Legacy1201FluidStatus { level: -20, state: LAVA });
        drop(spread);
        drop(lava);
        assert_eq!(spread_calls, 1);
        assert_eq!(lava_calls, 1);
    }

    #[test]
    fn deep_dark_dry_mode_skips_floodedness_spread_and_lava() {
        let mut surface = |_, _| Ok(100);
        let mut dark = |_| Ok(true);
        let mut no_flood = |_| -> io::Result<f64> { panic!("deep dark skips floodedness") };
        let mut no_spread = |_| -> io::Result<f64> { panic!("dry skips spread") };
        let mut no_lava = |_| -> io::Result<f64> { panic!("dry skips lava") };
        let mut picker = global;
        let result = legacy1201_compute_aquifer_cell_status(
            BlockPos::new(0, -33, 0), LAVA, &mut surface, &mut picker,
            &mut dark, &mut no_flood, &mut no_spread, &mut no_lava,
        ).unwrap();
        assert_eq!(result, Legacy1201FluidStatus {
            level: LEGACY1201_DRY_FLUID_LEVEL, state: WATER,
        });
    }

    #[test]
    fn missing_surface_or_floodedness_is_not_hidden() {
        let mut bad_surface = |_, _| Err(io::Error::new(io::ErrorKind::NotFound, "surface"));
        let mut surface = |_, _| Ok(100);
        let mut dark = |_| Ok(false);
        let mut bad_flood = |_| Err(io::Error::new(io::ErrorKind::WouldBlock, "flood"));
        let mut no_spread = |_| -> io::Result<f64> { panic!("must not sample spread") };
        let mut no_lava = |_| -> io::Result<f64> { panic!("must not sample lava") };
        let site = BlockPos::new(0, 0, 0);
        let mut picker = global;
        assert_eq!(legacy1201_compute_aquifer_cell_status(
            site, LAVA, &mut bad_surface, &mut picker,
            &mut dark, &mut bad_flood, &mut no_spread, &mut no_lava,
        ).unwrap_err().kind(), io::ErrorKind::NotFound);
        assert_eq!(legacy1201_compute_aquifer_cell_status(
            site, LAVA, &mut surface, &mut picker,
            &mut dark, &mut bad_flood, &mut no_spread, &mut no_lava,
        ).unwrap_err().kind(), io::ErrorKind::WouldBlock);
    }
}
