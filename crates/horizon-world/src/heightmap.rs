//! Packed column heights and the six Java 26.3 heightmap predicates.
//!
//! Block behavior supplies air/fluid facts and resolved block-tag membership.
//! Heightmaps do not infer those facts from collision shapes or block names.
use crate::{
    compression::invalid,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockTable},
};
use horizon_core::storage::{PackedStorage, StorageError};
use std::{io, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum HeightmapType {
    WorldSurfaceWg,
    WorldSurface,
    OceanFloorWg,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

impl HeightmapType {
    pub const ALL: [Self; 6] = [
        Self::WorldSurfaceWg,
        Self::WorldSurface,
        Self::OceanFloorWg,
        Self::OceanFloor,
        Self::MotionBlocking,
        Self::MotionBlockingNoLeaves,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Self::WorldSurfaceWg => "WORLD_SURFACE_WG",
            Self::WorldSurface => "WORLD_SURFACE",
            Self::OceanFloorWg => "OCEAN_FLOOR_WG",
            Self::OceanFloor => "OCEAN_FLOOR",
            Self::MotionBlocking => "MOTION_BLOCKING",
            Self::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
        }
    }

    pub fn send_to_client(self) -> bool {
        matches!(
            self,
            Self::WorldSurface | Self::MotionBlocking | Self::MotionBlockingNoLeaves
        )
    }

    pub fn keep_after_worldgen(self) -> bool {
        !matches!(self, Self::WorldSurfaceWg | Self::OceanFloorWg)
    }

    pub fn is_opaque(self, state: impl HeightmapState) -> bool {
        state.opaque_for_heightmap(self)
    }
}

pub trait HeightmapState: Copy {
    fn is_air(self) -> bool;
    /// Exact AIR block identity, distinct from cave/void air.
    fn is_air_block(self) -> bool;
    fn has_fluid(self) -> bool;
    fn blocks_motion_in_heightmap(self) -> bool;
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool;

    /// Override only for a version whose heightmap predicates differ.
    fn opaque_for_heightmap(self, kind: HeightmapType) -> bool {
        match kind {
            HeightmapType::WorldSurfaceWg | HeightmapType::WorldSurface => !self.is_air(),
            HeightmapType::OceanFloorWg | HeightmapType::OceanFloor => {
                self.blocks_motion_in_heightmap()
            }
            HeightmapType::MotionBlocking => self.blocks_motion_in_heightmap() || self.has_fluid(),
            HeightmapType::MotionBlockingNoLeaves => {
                self.blocks_motion_in_heightmap_no_leaves() || self.has_fluid()
            }
        }
    }
}

/// Per-state facts from a 1.20.1 registry. `leaves_block` is Java's
/// `instanceof LeavesBlock`, rather than membership in a newer block tag.
#[derive(Clone, Copy, Debug)]
pub struct Legacy1201HeightmapState {
    pub air: bool,
    pub air_block: bool,
    pub fluid: bool,
    pub blocks_motion: bool,
    pub leaves_block: bool,
}

impl HeightmapState for Legacy1201HeightmapState {
    fn is_air(self) -> bool {
        self.air
    }
    fn is_air_block(self) -> bool {
        self.air_block
    }
    fn has_fluid(self) -> bool {
        self.fluid
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        self.blocks_motion
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        self.blocks_motion && !self.leaves_block
    }
    fn opaque_for_heightmap(self, kind: HeightmapType) -> bool {
        match kind {
            HeightmapType::WorldSurfaceWg | HeightmapType::WorldSurface => !self.air,
            HeightmapType::OceanFloorWg | HeightmapType::OceanFloor => self.blocks_motion,
            HeightmapType::MotionBlocking => self.blocks_motion || self.fluid,
            // port: net.minecraft.world.level.levelgen.Heightmap$Types#<clinit> (1.20.1)
            HeightmapType::MotionBlockingNoLeaves => {
                (self.blocks_motion || self.fluid) && !self.leaves_block
            }
        }
    }
}

/// State-indexed 1.20.1 heightmap facts bound to the registry they describe.
#[derive(Clone)]
pub struct Legacy1201HeightmapRegistry<'r> {
    registry: &'r BlockRegistry,
    states: Arc<[Legacy1201HeightmapState]>,
}

impl<'r> Legacy1201HeightmapRegistry<'r> {
    pub fn from_nbt(tag: &Tag, registry: &'r BlockRegistry) -> io::Result<Self> {
        Self::from_table(BlockTable::check(tag, registry, "1.20.1 heightmap")?)
    }

    pub fn from_table(table: BlockTable<'_, 'r>) -> io::Result<Self> {
        if table.get("DataVersion") != Some(&Tag::Int(3465)) {
            return Err(invalid("1.20.1 heightmap DataVersion mismatch"));
        }
        let Some(Tag::ByteArray(flags)) = table.get("flags") else {
            return Err(invalid("missing 1.20.1 heightmap flags"));
        };
        let registry = table.registry();
        if flags.len() != registry.len() {
            return Err(invalid("1.20.1 heightmap flag count mismatch"));
        }
        let mut states = Vec::with_capacity(flags.len());
        for (i, &bits) in flags.iter().enumerate() {
            if bits & !0x1f != 0 {
                return Err(invalid("invalid 1.20.1 heightmap flags"));
            }
            let state = Legacy1201HeightmapState {
                air: bits & 1 != 0,
                air_block: bits & 2 != 0,
                fluid: bits & 4 != 0,
                blocks_motion: bits & 8 != 0,
                leaves_block: bits & 16 != 0,
            };
            let descriptor = registry
                .state(BlockStateId(i as u32))
                .expect("validated length");
            if state.air_block != (descriptor.name == "minecraft:air")
                || (state.air_block && !state.air)
            {
                return Err(invalid("1.20.1 heightmap air identity mismatch"));
            }
            states.push(state);
        }
        Ok(Self {
            registry,
            states: states.into(),
        })
    }

    pub fn state(&self, id: BlockStateId) -> Option<Legacy1201HeightmapState> {
        self.states.get(id.0 as usize).copied()
    }

    pub fn registry(&self) -> &'r BlockRegistry {
        self.registry
    }
}

pub trait HeightmapChunk {
    type State: HeightmapState;
    fn min_y(&self) -> i32;
    fn height(&self) -> i32;
    fn highest_section_y(&self) -> i32;
    fn block_state(&self, x: i32, y: i32, z: i32) -> Self::State;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HeightmapError {
    InvalidBounds,
    MismatchedChunk,
    Storage(StorageError),
}
impl From<StorageError> for HeightmapError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}
impl std::fmt::Display for HeightmapError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for HeightmapError {}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Heightmap {
    kind: HeightmapType,
    min_y: i32,
    height: i32,
    data: PackedStorage,
}

impl Heightmap {
    /// The heap bytes of its packed heights.
    pub fn heap_bytes(&self) -> usize {
        self.data.heap_bytes()
    }
    /// Bounded to the vanilla codec's positive, aligned vertical ranges.
    pub fn new(kind: HeightmapType, min_y: i32, height: i32) -> Result<Self, HeightmapError> {
        if !(-2032..=2031).contains(&min_y)
            || !(16..=4064).contains(&height)
            || min_y % 16 != 0
            || height % 16 != 0
            || min_y + height > 2032
        {
            return Err(HeightmapError::InvalidBounds);
        }
        let bits = (32 - (height as u32).leading_zeros()) as u8;
        Ok(Self {
            kind,
            min_y,
            height,
            data: PackedStorage::new(bits, 256)?,
        })
    }

    pub fn kind(&self) -> HeightmapType {
        self.kind
    }
    pub fn raw_data(&self) -> &[u64] {
        self.data.words()
    }

    fn index(x: i32, z: i32) -> usize {
        x.wrapping_add(z.wrapping_mul(16)) as usize
    }

    /// Whether `update` at (`x`, `y`, `z`) returns at once, whatever the
    /// state: the block is two or more below the first available one. `x`
    /// and `z` are within 0..16. Most generation writes are (the surface
    /// rule turns every stone under y 8 to deepslate), and this skips the
    /// checks `update` does first.
    #[inline]
    pub fn below_update(&self, chunk: &impl HeightmapChunk, x: i32, y: i32, z: i32) -> bool {
        debug_assert!((0..16).contains(&x) && (0..16).contains(&z));
        let first = (self.data.read(Self::index(x, z)) as i32).wrapping_add(self.min_y);
        y <= first.wrapping_sub(2) && self.check_chunk(chunk).is_ok()
    }

    pub fn first_available(&self, x: i32, z: i32) -> Result<i32, HeightmapError> {
        Ok((self.data.get(Self::index(x, z))? as i32).wrapping_add(self.min_y))
    }

    /// Whether any column of the local square `x0..=x1` by `z0..=z1` (each
    /// within 0..16) has its first available y at or above `y`.
    pub fn any_first_available_at_least(
        &self,
        [x0, x1]: [i32; 2],
        [z0, z1]: [i32; 2],
        y: i32,
    ) -> bool {
        debug_assert!(0 <= x0 && x1 < 16 && 0 <= z0 && z1 < 16);
        (z0..=z1).any(|z| {
            (x0..=x1)
                .any(|x| (self.data.read(Self::index(x, z)) as i32).wrapping_add(self.min_y) >= y)
        })
    }

    pub fn highest_taken(&self, x: i32, z: i32) -> Result<i32, HeightmapError> {
        Ok(self.first_available(x, z)?.wrapping_sub(1))
    }

    fn set_height(&mut self, x: i32, z: i32, y: i32) -> Result<(), HeightmapError> {
        self.data
            .set(Self::index(x, z), y.wrapping_sub(self.min_y) as u32)?;
        Ok(())
    }

    fn check_chunk(&self, chunk: &impl HeightmapChunk) -> Result<(), HeightmapError> {
        if chunk.min_y() != self.min_y || chunk.height() != self.height {
            return Err(HeightmapError::MismatchedChunk);
        }
        Ok(())
    }

    /// The caller writes the new state to the chunk before updating its maps.
    pub fn update<C: HeightmapChunk>(
        &mut self,
        chunk: &C,
        x: i32,
        y: i32,
        z: i32,
        state: C::State,
    ) -> Result<bool, HeightmapError> {
        self.check_chunk(chunk)?;
        let first = self.first_available(x, z)?;
        if y <= first.wrapping_sub(2) {
            return Ok(false);
        }
        if self.kind.is_opaque(state) {
            if y >= first {
                self.set_height(x, z, y.wrapping_add(1))?;
                return Ok(true);
            }
        } else if y == first.wrapping_sub(1) {
            let mut scan_y = y.wrapping_sub(1);
            while scan_y >= self.min_y {
                if self.kind.is_opaque(chunk.block_state(x, scan_y, z)) {
                    self.set_height(x, z, scan_y.wrapping_add(1))?;
                    return Ok(true);
                }
                scan_y -= 1;
            }
            self.set_height(x, z, self.min_y)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Matching lengths preserve all bits, including padding and unused slots.
    /// A length mismatch primes this registered map from the chunk instead.
    pub fn set_raw_data(
        &mut self,
        chunk: &impl HeightmapChunk,
        words: &[u64],
    ) -> Result<(), HeightmapError> {
        self.check_chunk(chunk)?;
        if words.len() == self.data.words().len() {
            self.data = PackedStorage::from_words(self.data.bits(), 256, words.to_vec())?;
            Ok(())
        } else {
            Self::prime(chunk, &mut [self])
        }
    }

    /// Preserves caller map order and scans X, then Z, then descending Y.
    /// An unfound column retains its previous value, including during re-priming.
    pub fn prime(
        chunk: &impl HeightmapChunk,
        maps: &mut [&mut Self],
    ) -> Result<(), HeightmapError> {
        if maps.is_empty() {
            return Ok(());
        }
        for map in maps.iter() {
            map.check_chunk(chunk)?;
        }
        let upper = chunk.highest_section_y().wrapping_add(16);
        let mut remaining = Vec::with_capacity(maps.len());
        for x in 0..16 {
            for z in 0..16 {
                remaining.clear();
                remaining.extend(0..maps.len());
                let mut y = upper.wrapping_sub(1);
                while y >= chunk.min_y() {
                    let state = chunk.block_state(x, y, z);
                    if !state.is_air_block() {
                        let mut i = 0;
                        while i < remaining.len() {
                            let map = &mut maps[remaining[i]];
                            if map.kind.is_opaque(state) {
                                map.set_height(x, z, y.wrapping_add(1))?;
                                remaining.remove(i);
                            } else {
                                i += 1;
                            }
                        }
                        if remaining.is_empty() {
                            break;
                        }
                    }
                    y -= 1;
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod legacy_1201_tests {
    use super::*;

    const AIR: Legacy1201HeightmapState = Legacy1201HeightmapState {
        air: true,
        air_block: true,
        fluid: false,
        blocks_motion: false,
        leaves_block: false,
    };
    const STONE: Legacy1201HeightmapState = Legacy1201HeightmapState {
        air: false,
        air_block: false,
        fluid: false,
        blocks_motion: true,
        leaves_block: false,
    };
    const WET_LEAVES: Legacy1201HeightmapState = Legacy1201HeightmapState {
        air: false,
        air_block: false,
        fluid: true,
        blocks_motion: true,
        leaves_block: true,
    };

    struct Column;
    impl HeightmapChunk for Column {
        type State = Legacy1201HeightmapState;
        fn min_y(&self) -> i32 {
            0
        }
        fn height(&self) -> i32 {
            16
        }
        fn highest_section_y(&self) -> i32 {
            0
        }
        fn block_state(&self, _: i32, y: i32, _: i32) -> Self::State {
            match y {
                4 => STONE,
                6 => WET_LEAVES,
                _ => AIR,
            }
        }
    }

    #[derive(Clone, Copy)]
    struct CurrentWetLeaves;
    impl HeightmapState for CurrentWetLeaves {
        fn is_air(self) -> bool {
            false
        }
        fn is_air_block(self) -> bool {
            false
        }
        fn has_fluid(self) -> bool {
            true
        }
        fn blocks_motion_in_heightmap(self) -> bool {
            true
        }
        fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
            false
        }
    }

    #[test]
    fn wet_leaves_do_not_raise_legacy_no_leaves_heightmap() {
        let mut no_leaves = Heightmap::new(HeightmapType::MotionBlockingNoLeaves, 0, 16).unwrap();
        let mut motion = Heightmap::new(HeightmapType::MotionBlocking, 0, 16).unwrap();
        Heightmap::prime(&Column, &mut [&mut no_leaves, &mut motion]).unwrap();
        assert_eq!(no_leaves.first_available(0, 0).unwrap(), 5);
        assert_eq!(motion.first_available(0, 0).unwrap(), 7);
        assert!(HeightmapType::MotionBlockingNoLeaves.is_opaque(CurrentWetLeaves));
    }

    fn registry() -> BlockRegistry {
        let names = [
            "minecraft:air",
            "minecraft:void_air",
            "minecraft:stone",
            "minecraft:oak_leaves",
        ];
        BlockRegistry::from_nbt(&Tag::compound([
            ("DataVersion", Tag::Int(crate::DATA_VERSION)),
            (
                "states",
                Tag::List(
                    names
                        .iter()
                        .map(|name| Tag::compound([("id", Tag::String((*name).into()))]))
                        .collect(),
                ),
            ),
            (
                "defaults",
                Tag::compound(
                    names
                        .iter()
                        .enumerate()
                        .map(|(i, name)| (*name, Tag::Int(i as i32))),
                ),
            ),
        ]))
        .unwrap()
    }

    fn table(registry: &BlockRegistry, version: i32, flags: Vec<u8>) -> Tag {
        Tag::compound([
            ("DataVersion", Tag::Int(version)),
            (
                "states",
                Tag::List(
                    (0..registry.len())
                        .map(|i| registry.state(BlockStateId(i as u32)).unwrap().to_nbt())
                        .collect(),
                ),
            ),
            ("flags", Tag::ByteArray(flags)),
        ])
    }

    #[test]
    fn table_rejects_wrong_version_registry_and_invalid_flags() {
        let registry = registry();
        let flags = vec![3, 1, 8, 28];
        let good = table(&registry, 3465, flags.clone());
        assert!(Legacy1201HeightmapRegistry::from_nbt(&good, &registry).is_ok());
        assert!(
            Legacy1201HeightmapRegistry::from_nbt(
                &table(&registry, crate::DATA_VERSION, flags.clone()),
                &registry
            )
            .is_err()
        );
        assert!(
            Legacy1201HeightmapRegistry::from_nbt(&table(&registry, 3465, vec![3]), &registry)
                .is_err()
        );
        assert!(
            Legacy1201HeightmapRegistry::from_nbt(
                &table(&registry, 3465, vec![3, 1, 8, 0x80]),
                &registry
            )
            .is_err()
        );
        assert!(
            Legacy1201HeightmapRegistry::from_nbt(
                &table(&registry, 3465, vec![1, 1, 8, 28]),
                &registry
            )
            .is_err()
        );
        let mut different = good.clone();
        if let Tag::List(states) = different.get_mut("states").unwrap() {
            states.swap(2, 3);
        }
        assert!(Legacy1201HeightmapRegistry::from_nbt(&different, &registry).is_err());
    }

    #[test]
    fn verified_legacy_table_updates_a_terrain_chunk() {
        use crate::worldgen::terrain::TerrainChunk;
        use horizon_core::position::ChunkPos;
        let registry = registry();
        let other = self::registry();
        let tag = table(&registry, 3465, vec![3, 1, 8, 28]);
        let props = Legacy1201HeightmapRegistry::from_nbt(&tag, &registry).unwrap();
        assert!(TerrainChunk::new(ChunkPos::new(0, 0), 0, 16, &other, props.clone()).is_err());
        let mut chunk =
            TerrainChunk::new(ChunkPos::new(0, 0), 0, 16, &registry, props.clone()).unwrap();
        let mut second = TerrainChunk::new(ChunkPos::new(1, 0), 0, 16, &registry, props).unwrap();
        chunk.set_block_raw(3, 4, 3, BlockStateId(2)).unwrap();
        chunk.set_block_raw(3, 6, 3, BlockStateId(3)).unwrap();
        chunk.prime_heightmaps(&HeightmapType::ALL).unwrap();
        assert_eq!(
            chunk
                .heightmap(HeightmapType::MotionBlocking)
                .unwrap()
                .first_available(3, 3)
                .unwrap(),
            7
        );
        assert_eq!(
            chunk
                .heightmap(HeightmapType::MotionBlockingNoLeaves)
                .unwrap()
                .first_available(3, 3)
                .unwrap(),
            5
        );
        second.set_block_raw(3, 6, 3, BlockStateId(3)).unwrap();
        second.prime_heightmaps(&HeightmapType::ALL).unwrap();
        assert_eq!(
            second
                .heightmap(HeightmapType::MotionBlockingNoLeaves)
                .unwrap()
                .first_available(3, 3)
                .unwrap(),
            0
        );
        chunk.set_block_raw(3, 6, 3, BlockStateId(0)).unwrap();
        chunk
            .update_heightmap(HeightmapType::MotionBlocking, 3, 6, 3, BlockStateId(0))
            .unwrap();
        assert_eq!(
            chunk
                .heightmap(HeightmapType::MotionBlocking)
                .unwrap()
                .first_available(3, 3)
                .unwrap(),
            5
        );
    }
}
