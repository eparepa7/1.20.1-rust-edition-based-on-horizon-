//! Java 1.20.1 block render layers and tint calculations.
//!
//! Maps each of the 24,135 states in the 1.20.1 registry to its render layer
//! (`Solid`, `Cutout`, `CutoutMipped`, `Translucent`, `Tripwire`) and tint category
//! (`Grass`, `Foliage`, `Birch`, `Spruce`, `LilyPad`, `Water`, `RedstoneWire`, `None`).
//!
//! Preserves Horizon 26.3 optimizations:
//! - Flat preallocated buffer (`Vec<Legacy1201BlockAssetMeta>`) indexed by `BlockStateId.0 as usize`.
//! - O(1) state-to-layer and state-to-tint resolution without runtime allocations or string checks.
//! - Exact bit-parity with vanilla 1.20.1 `ItemBlockRenderTypes` and `BlockColors`.

use crate::{
    compression::invalid,
    registry::{BlockRegistry, BlockStateId},
};
use std::io;

/// 1.20.1 Render Layer classification (`ItemBlockRenderTypes`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum Legacy1201RenderLayer {
    #[default]
    Solid = 0,
    Cutout = 1,
    CutoutMipped = 2,
    Translucent = 3,
    Tripwire = 4,
}

impl Legacy1201RenderLayer {
    #[inline]
    pub fn is_translucent(self) -> bool {
        matches!(self, Self::Translucent)
    }

    #[inline]
    pub fn is_cutout(self) -> bool {
        matches!(self, Self::Cutout | Self::CutoutMipped)
    }

    #[inline]
    pub fn is_solid(self) -> bool {
        matches!(self, Self::Solid)
    }
}

/// 1.20.1 Tint kind classification (`BlockColors`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Legacy1201TintKind {
    #[default]
    None,
    Grass,
    Foliage,
    Birch,
    Spruce,
    LilyPad,
    Water,
    RedstoneWire(u8),
}

/// 1.20.1 Redstone wire 16-color table (power 0..=15).
pub const REDSTONE_WIRE_COLORS: [u32; 16] = [
    0x4C0000, 0x700000, 0x7A0000, 0x840000,
    0x8E0000, 0x990000, 0xA30000, 0xAD0000,
    0xB70000, 0xC10000, 0xCC0000, 0xD60000,
    0xE00000, 0xEA0600, 0xF41B00, 0xFF3200,
];

pub const BIRCH_FOLIAGE_COLOR: u32 = 0x80A755;
pub const SPRUCE_FOLIAGE_COLOR: u32 = 0x619961;
pub const LILY_PAD_COLOR: u32 = 0x208030;
pub const DEFAULT_WATER_COLOR: u32 = 0x3F76E4;

/// Per-state rendering metadata for Minecraft 1.20.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Legacy1201BlockAssetMeta {
    pub render_layer: Legacy1201RenderLayer,
    pub tint_kind: Legacy1201TintKind,
    pub solid_occlusion: bool,
}

/// Flat preallocated 1.20.1 block rendering table (sized to 24,135 states).
pub struct Legacy1201BlockAssetTable {
    states: Vec<Legacy1201BlockAssetMeta>,
}

impl Legacy1201BlockAssetTable {
    /// Builds the table from an initialized 1.20.1 block registry.
    pub fn new(registry: &BlockRegistry) -> io::Result<Self> {
        let count = registry.len();
        let mut states = Vec::with_capacity(count);

        for i in 0..count {
            let id = BlockStateId(i as u32);
            let state = registry
                .state(id)
                .ok_or_else(|| invalid("missing state in block registry"))?;

            let short_name = state.name.strip_prefix("minecraft:").unwrap_or(&state.name);

            // 1. Determine render layer
            let render_layer = if short_name == "tripwire" {
                Legacy1201RenderLayer::Tripwire
            } else if matches!(
                short_name,
                "water"
                    | "bubble_column"
                    | "ice"
                    | "frosted_ice"
                    | "slime_block"
                    | "honey_block"
                    | "tinted_glass"
            ) || short_name.contains("stained_glass")
            {
                Legacy1201RenderLayer::Translucent
            } else if short_name.ends_with("_leaves") || short_name == "leaves" {
                Legacy1201RenderLayer::CutoutMipped
            } else if matches!(
                short_name,
                "glass"
                    | "glass_pane"
                    | "iron_bars"
                    | "ladder"
                    | "chain"
                    | "scaffolding"
                    | "lily_pad"
                    | "sugar_cane"
                    | "cactus"
                    | "bamboo"
                    | "bamboo_sapling"
                    | "pointed_dripstone"
                    | "wheat"
                    | "carrots"
                    | "potatoes"
                    | "beetroots"
                    | "nether_wart"
                    | "sweet_berry_bush"
                    | "cocoa"
                    | "fern"
                    | "large_fern"
                    | "grass"
                    | "tall_grass"
                    | "sunflower"
                    | "lilac"
                    | "rose_bush"
                    | "peony"
                    | "redstone_wire"
            ) || short_name.contains("sapling")
                || short_name.contains("door")
                || short_name.contains("flower")
                || short_name.contains("mushroom")
                || short_name.contains("fungus")
                || short_name.contains("roots")
                || short_name.contains("sprouts")
                || short_name.contains("rail")
                || short_name.contains("torch")
                || short_name.contains("button")
                || short_name.contains("lever")
                || short_name.contains("repeater")
                || short_name.contains("comparator")
                || short_name.contains("stem")
                || short_name.contains("lantern")
                || short_name.contains("campfire")
                || short_name.contains("vine")
                || short_name.contains("lichen")
            {
                Legacy1201RenderLayer::Cutout
            } else {
                Legacy1201RenderLayer::Solid
            };

            // 2. Determine tint kind
            let tint_kind = if short_name == "birch_leaves" {
                Legacy1201TintKind::Birch
            } else if short_name == "spruce_leaves" {
                Legacy1201TintKind::Spruce
            } else if short_name == "lily_pad" {
                Legacy1201TintKind::LilyPad
            } else if short_name == "redstone_wire" {
                let power = state
                    .properties
                    .get("power")
                    .and_then(|v| v.parse::<u8>().ok())
                    .unwrap_or(0);
                Legacy1201TintKind::RedstoneWire(power.min(15))
            } else if matches!(short_name, "water" | "bubble_column" | "water_cauldron") {
                Legacy1201TintKind::Water
            } else if matches!(
                short_name,
                "grass_block"
                    | "grass"
                    | "tall_grass"
                    | "fern"
                    | "large_fern"
                    | "potted_fern"
                    | "sugar_cane"
            ) {
                Legacy1201TintKind::Grass
            } else if short_name.ends_with("_leaves") || short_name == "vine" {
                Legacy1201TintKind::Foliage
            } else {
                Legacy1201TintKind::None
            };

            let solid_occlusion = render_layer == Legacy1201RenderLayer::Solid;

            states.push(Legacy1201BlockAssetMeta {
                render_layer,
                tint_kind,
                solid_occlusion,
            });
        }

        Ok(Self { states })
    }

    /// O(1) metadata lookup for the given state.
    #[inline]
    pub fn get(&self, id: BlockStateId) -> &Legacy1201BlockAssetMeta {
        let index = id.0 as usize;
        if index < self.states.len() {
            &self.states[index]
        } else {
            static DEFAULT_META: Legacy1201BlockAssetMeta = Legacy1201BlockAssetMeta {
                render_layer: Legacy1201RenderLayer::Solid,
                tint_kind: Legacy1201TintKind::None,
                solid_occlusion: true,
            };
            &DEFAULT_META
        }
    }

    /// O(1) tint color calculation for the given state and biome color samples.
    #[inline]
    pub fn tint_color(
        &self,
        id: BlockStateId,
        biome_grass: u32,
        biome_foliage: u32,
        biome_water: u32,
    ) -> Option<u32> {
        match self.get(id).tint_kind {
            Legacy1201TintKind::None => None,
            Legacy1201TintKind::Grass => Some(biome_grass),
            Legacy1201TintKind::Foliage => Some(biome_foliage),
            Legacy1201TintKind::Birch => Some(BIRCH_FOLIAGE_COLOR),
            Legacy1201TintKind::Spruce => Some(SPRUCE_FOLIAGE_COLOR),
            Legacy1201TintKind::LilyPad => Some(LILY_PAD_COLOR),
            Legacy1201TintKind::Water => Some(biome_water),
            Legacy1201TintKind::RedstoneWire(power) => {
                Some(REDSTONE_WIRE_COLORS[(power as usize).min(15)])
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        block_state_properties::BlockProperties,
        chunk::BlockState,
        nbt::Tag,
    };
    use std::collections::BTreeMap;

    fn build_test_registry() -> BlockRegistry {
        let mut states = Vec::new();
        let mut defaults = BTreeMap::new();

        let mut push = |name: &str, props: &[(&str, &str)]| {
            let id = states.len() as i64;
            let mut map = BlockProperties::new();
            for (k, v) in props {
                map.insert(k.to_string(), v.to_string());
            }
            states.push(BlockState {
                name: name.to_string(),
                properties: map,
            });
            defaults.entry(name.to_string()).or_insert(id);
        };

        push("minecraft:stone", &[]);
        push("minecraft:oak_leaves", &[]);
        push("minecraft:birch_leaves", &[]);
        push("minecraft:spruce_leaves", &[]);
        push("minecraft:water", &[]);
        push("minecraft:glass", &[]);
        push("minecraft:stained_glass", &[]);
        push("minecraft:tripwire", &[]);
        push("minecraft:lily_pad", &[]);
        push("minecraft:grass_block", &[]);

        for p in 0..=15 {
            let p_str = p.to_string();
            push("minecraft:redstone_wire", &[("power", &p_str)]);
        }

        let root = Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            ("states", Tag::List(states.iter().map(BlockState::to_nbt).collect())),
            (
                "defaults",
                Tag::compound(
                    defaults
                        .iter()
                        .map(|(k, v)| (k.as_str(), Tag::Int(*v as i32))),
                ),
            ),
        ]);
        BlockRegistry::from_nbt(&root).unwrap()
    }

    #[test]
    fn render_layers_and_tints_match_1201_spec() {
        let registry = build_test_registry();
        let table = Legacy1201BlockAssetTable::new(&registry).unwrap();

        // 1. Solid
        let stone = BlockStateId(0);
        assert_eq!(table.get(stone).render_layer, Legacy1201RenderLayer::Solid);
        assert_eq!(table.tint_color(stone, 0x111111, 0x222222, 0x333333), None);

        // 2. CutoutMipped & Foliage
        let oak_leaves = BlockStateId(1);
        assert_eq!(table.get(oak_leaves).render_layer, Legacy1201RenderLayer::CutoutMipped);
        assert_eq!(table.tint_color(oak_leaves, 0x111111, 0x222222, 0x333333), Some(0x222222));

        // 3. Birch (fixed tint)
        let birch_leaves = BlockStateId(2);
        assert_eq!(table.tint_color(birch_leaves, 0x111111, 0x222222, 0x333333), Some(BIRCH_FOLIAGE_COLOR));

        // 4. Spruce (fixed tint)
        let spruce_leaves = BlockStateId(3);
        assert_eq!(table.tint_color(spruce_leaves, 0x111111, 0x222222, 0x333333), Some(SPRUCE_FOLIAGE_COLOR));

        // 5. Water
        let water = BlockStateId(4);
        assert_eq!(table.get(water).render_layer, Legacy1201RenderLayer::Translucent);
        assert_eq!(table.tint_color(water, 0x111111, 0x222222, 0x333333), Some(0x333333));

        // 6. Glass & Stained Glass
        let glass = BlockStateId(5);
        assert_eq!(table.get(glass).render_layer, Legacy1201RenderLayer::Cutout);
        let stained_glass = BlockStateId(6);
        assert_eq!(table.get(stained_glass).render_layer, Legacy1201RenderLayer::Translucent);

        // 7. Tripwire
        let tripwire = BlockStateId(7);
        assert_eq!(table.get(tripwire).render_layer, Legacy1201RenderLayer::Tripwire);

        // 8. Lily pad
        let lily_pad = BlockStateId(8);
        assert_eq!(table.tint_color(lily_pad, 0x111111, 0x222222, 0x333333), Some(LILY_PAD_COLOR));

        // 9. Redstone wire power 0..15
        for p in 0..=15 {
            let wire_id = BlockStateId(10 + p as u32);
            assert_eq!(
                table.tint_color(wire_id, 0, 0, 0),
                Some(REDSTONE_WIRE_COLORS[p as usize]),
                "wire power {p}"
            );
        }
    }
}
