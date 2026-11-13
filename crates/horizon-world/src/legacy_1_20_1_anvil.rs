//! Vanilla Java 1.20.1 Anvil/NBT chunk, section, region and world format.
//!
//! Provides DataVersion 3465 chunk and section data structures for Minecraft Java 1.20.1:
//! - Full Overworld height range (-64..320, 24 sections Y = -4..=19)
//! - Bit-exact NBT parity: Capitalized 'Name' and 'Properties' in block palettes
//! - Packed 4-12 bit block states and 1-6 bit biomes with PackedStorage (vanilla BitStorage)
//! - 9-bit heightmaps (MOTION_BLOCKING, MOTION_BLOCKING_NO_LEAVES, OCEAN_FLOOR, WORLD_SURFACE)
//! - 4-bit nibble BlockLight and SkyLight (2048 bytes per section)
//! - Superflat world generator with vanilla 1.20.1 classic preset
//! - level.dat reading, generation and saving with DataVersion 3465
//! - Bi-directional RegionFile (.mca) chunk load/save

use crate::{
    compression::{invalid, Compression},
    nbt::{self, Compound, JavaString, Limits, NamedTag, Tag},
    region::{ChunkRecord, RegionFile},
};
use horizon_core::{
    position::{BlockPos, ChunkPos},
    storage::PackedStorage,
};
use std::{
    collections::BTreeMap,
    fs,
    io,
    path::{Path, PathBuf},
};

pub const LEGACY_1_20_1_DATA_VERSION: i32 = 3465;
pub const OVERWORLD_MIN_SECTION_Y: i8 = -4;
pub const OVERWORLD_MAX_SECTION_Y: i8 = 19;
pub const OVERWORLD_SECTION_COUNT: usize = 24;
pub const OVERWORLD_MIN_Y: i32 = -64;
pub const OVERWORLD_MAX_Y: i32 = 320;
pub const OVERWORLD_HEIGHT: i32 = 384;
pub const BLOCKS_PER_SECTION: usize = 4096;
pub const BIOMES_PER_SECTION: usize = 64;
pub const LIGHT_BYTES_PER_SECTION: usize = 2048;
pub const HEIGHTMAP_BITS_OVERWORLD: u8 = 9;
pub const HEIGHTMAP_ENTRIES: usize = 256;

#[inline]
fn key(val: &str) -> JavaString {
    JavaString::from(val)
}

#[inline]
fn bits_for_palette(len: usize, min: u8) -> u8 {
    if len <= 1 {
        0
    } else {
        ((usize::BITS - (len - 1).leading_zeros()) as u8).max(min)
    }
}

/// Vanilla 1.20.1 block state representation in chunk palettes.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Legacy1201BlockState {
    pub name: String,
    pub properties: BTreeMap<String, String>,
}

impl Legacy1201BlockState {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            properties: BTreeMap::new(),
        }
    }

    pub fn with_property(mut self, key: impl Into<String>, val: impl Into<String>) -> Self {
        self.properties.insert(key.into(), val.into());
        self
    }

    pub fn air() -> Self {
        Self::new("minecraft:air")
    }

    pub fn stone() -> Self {
        Self::new("minecraft:stone")
    }

    pub fn dirt() -> Self {
        Self::new("minecraft:dirt")
    }

    pub fn grass_block() -> Self {
        let mut props = BTreeMap::new();
        props.insert("snowy".to_string(), "false".to_string());
        Self {
            name: "minecraft:grass_block".to_string(),
            properties: props,
        }
    }

    pub fn bedrock() -> Self {
        Self::new("minecraft:bedrock")
    }

    pub fn water() -> Self {
        let mut props = BTreeMap::new();
        props.insert("level".to_string(), "0".to_string());
        Self {
            name: "minecraft:water".to_string(),
            properties: props,
        }
    }

    pub fn lava() -> Self {
        let mut props = BTreeMap::new();
        props.insert("level".to_string(), "0".to_string());
        Self {
            name: "minecraft:lava".to_string(),
            properties: props,
        }
    }

    pub fn is_air(&self) -> bool {
        self.name == "minecraft:air"
            || self.name == "minecraft:cave_air"
            || self.name == "minecraft:void_air"
    }

    pub fn is_fluid(&self) -> bool {
        self.name == "minecraft:water"
            || self.name == "minecraft:flowing_water"
            || self.name == "minecraft:lava"
            || self.name == "minecraft:flowing_lava"
    }

    pub fn is_leaves(&self) -> bool {
        self.name.ends_with("_leaves") || self.name == "minecraft:mangrove_roots"
    }

    pub fn blocks_movement(&self) -> bool {
        if self.is_air() || self.is_fluid() {
            return false;
        }
        !matches!(
            self.name.as_str(),
            "minecraft:short_grass"
                | "minecraft:grass"
                | "minecraft:tall_grass"
                | "minecraft:fern"
                | "minecraft:large_fern"
                | "minecraft:dandelion"
                | "minecraft:poppy"
                | "minecraft:blue_orchid"
                | "minecraft:allium"
                | "minecraft:azure_bluet"
                | "minecraft:red_tulip"
                | "minecraft:orange_tulip"
                | "minecraft:white_tulip"
                | "minecraft:pink_tulip"
                | "minecraft:oxeye_daisy"
                | "minecraft:cornflower"
                | "minecraft:lily_of_the_valley"
                | "minecraft:wither_rose"
                | "minecraft:sunflower"
                | "minecraft:lilac"
                | "minecraft:rose_bush"
                | "minecraft:peony"
                | "minecraft:dead_bush"
                | "minecraft:seagrass"
                | "minecraft:tall_seagrass"
                | "minecraft:kelp"
                | "minecraft:kelp_plant"
                | "minecraft:sugar_cane"
                | "minecraft:vine"
                | "minecraft:glow_lichen"
                | "minecraft:crimson_roots"
                | "minecraft:warped_roots"
                | "minecraft:nether_sprouts"
                | "minecraft:hanging_roots"
                | "minecraft:torch"
                | "minecraft:wall_torch"
                | "minecraft:soul_torch"
                | "minecraft:soul_wall_torch"
                | "minecraft:redstone_torch"
                | "minecraft:redstone_wall_torch"
                | "minecraft:redstone_wire"
                | "minecraft:tripwire"
                | "minecraft:tripwire_hook"
                | "minecraft:lever"
                | "minecraft:rail"
                | "minecraft:powered_rail"
                | "minecraft:detector_rail"
                | "minecraft:activator_rail"
                | "minecraft:light"
        )
    }

    pub fn to_nbt(&self) -> Tag {
        let mut map = Compound::new();
        map.insert(key("Name"), Tag::String(key(&self.name)));
        if !self.properties.is_empty() {
            let mut props = Compound::new();
            for (k, v) in &self.properties {
                props.insert(key(k), Tag::String(key(v)));
            }
            map.insert(key("Properties"), Tag::Compound(props));
        }
        Tag::Compound(map)
    }

    pub fn from_nbt(tag: &Tag) -> io::Result<Self> {
        if let Tag::String(name) = tag {
            return Ok(Self::new(name.to_string().map_err(|e| invalid(e.to_string()))?));
        }
        let Tag::Compound(map) = tag else {
            return Err(invalid("block state is not a compound"));
        };
        let name_tag = map
            .get(&key("Name"))
            .or_else(|| map.get(&key("id")))
            .ok_or_else(|| invalid("block state is missing Name/id"))?;
        let Tag::String(jname) = name_tag else {
            return Err(invalid("block state Name/id is not a string"));
        };
        let name = jname.to_string().map_err(|e| invalid(e.to_string()))?;

        let mut properties = BTreeMap::new();
        if let Some(prop_tag) = map.get(&key("Properties")).or_else(|| map.get(&key("properties"))) {
            if let Tag::Compound(pmap) = prop_tag {
                for (k, v) in pmap {
                    if let Tag::String(jv) = v {
                        properties.insert(
                            k.to_string().map_err(|e| invalid(e.to_string()))?,
                            jv.to_string().map_err(|e| invalid(e.to_string()))?,
                        );
                    }
                }
            }
        }
        Ok(Self { name, properties })
    }

    pub fn to_string_repr(&self) -> String {
        if self.properties.is_empty() {
            self.name.clone()
        } else {
            let mut s = format!("{}[", self.name);
            let mut first = true;
            for (k, v) in &self.properties {
                if !first {
                    s.push(',');
                }
                first = false;
                s.push_str(k);
                s.push('=');
                s.push_str(v);
            }
            s.push(']');
            s
        }
    }

    pub fn parse(s: &str) -> io::Result<Self> {
        let s = s.trim();
        if let Some(bracket) = s.find('[') {
            if !s.ends_with(']') {
                return Err(invalid(format!("invalid block state syntax: {s}")));
            }
            let name = &s[..bracket];
            let props_str = &s[bracket + 1..s.len() - 1];
            let mut properties = BTreeMap::new();
            if !props_str.is_empty() {
                for pair in props_str.split(',') {
                    let mut parts = pair.splitn(2, '=');
                    let k = parts.next().unwrap().trim();
                    let v = parts.next().ok_or_else(|| invalid("missing '=' in property"))?.trim();
                    properties.insert(k.to_string(), v.to_string());
                }
            }
            Ok(Self {
                name: name.to_string(),
                properties,
            })
        } else {
            Ok(Self::new(s))
        }
    }
}

/// Vanilla 1.20.1 heightmap types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HeightmapKind {
    WorldSurface,
    OceanFloor,
    MotionBlocking,
    MotionBlockingNoLeaves,
}

impl HeightmapKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WorldSurface => "WORLD_SURFACE",
            Self::OceanFloor => "OCEAN_FLOOR",
            Self::MotionBlocking => "MOTION_BLOCKING",
            Self::MotionBlockingNoLeaves => "MOTION_BLOCKING_NO_LEAVES",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "WORLD_SURFACE" => Some(Self::WorldSurface),
            "OCEAN_FLOOR" => Some(Self::OceanFloor),
            "MOTION_BLOCKING" => Some(Self::MotionBlocking),
            "MOTION_BLOCKING_NO_LEAVES" => Some(Self::MotionBlockingNoLeaves),
            _ => None,
        }
    }
}

/// 9-bit heightmaps for 1.20.1 overworld (256 columns: x + z * 16).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201Heightmaps {
    pub world_surface: [i32; 256],
    pub ocean_floor: [i32; 256],
    pub motion_blocking: [i32; 256],
    pub motion_blocking_no_leaves: [i32; 256],
}

impl Legacy1201Heightmaps {
    pub fn new(min_y: i32) -> Self {
        Self {
            world_surface: [min_y; 256],
            ocean_floor: [min_y; 256],
            motion_blocking: [min_y; 256],
            motion_blocking_no_leaves: [min_y; 256],
        }
    }

    #[inline]
    pub fn column_index(x: usize, z: usize) -> usize {
        (z & 15) * 16 + (x & 15)
    }

    pub fn get(&self, x: usize, z: usize, kind: HeightmapKind) -> i32 {
        let idx = Self::column_index(x, z);
        match kind {
            HeightmapKind::WorldSurface => self.world_surface[idx],
            HeightmapKind::OceanFloor => self.ocean_floor[idx],
            HeightmapKind::MotionBlocking => self.motion_blocking[idx],
            HeightmapKind::MotionBlockingNoLeaves => self.motion_blocking_no_leaves[idx],
        }
    }

    pub fn set(&mut self, x: usize, z: usize, kind: HeightmapKind, y: i32) {
        let idx = Self::column_index(x, z);
        match kind {
            HeightmapKind::WorldSurface => self.world_surface[idx] = y,
            HeightmapKind::OceanFloor => self.ocean_floor[idx] = y,
            HeightmapKind::MotionBlocking => self.motion_blocking[idx] = y,
            HeightmapKind::MotionBlockingNoLeaves => self.motion_blocking_no_leaves[idx] = y,
        }
    }

    fn pack_array(arr: &[i32; 256], min_y: i32) -> Tag {
        let mut storage = PackedStorage::new(HEIGHTMAP_BITS_OVERWORLD, HEIGHTMAP_ENTRIES)
            .expect("valid 9-bit storage");
        for idx in 0..256 {
            let rel = (arr[idx] - min_y + 1).max(0) as u32;
            let _ = storage.set(idx, rel);
        }
        Tag::LongArray(storage.words().iter().map(|&w| w as i64).collect())
    }

    fn unpack_array(words: &[i64], min_y: i32) -> io::Result<[i32; 256]> {
        let uwords: Vec<u64> = words.iter().map(|&w| w as u64).collect();
        let storage = PackedStorage::from_words(HEIGHTMAP_BITS_OVERWORLD, HEIGHTMAP_ENTRIES, uwords)
            .map_err(|e| invalid(format!("invalid heightmap storage: {e}")))?;
        let mut result = [min_y; 256];
        for idx in 0..256 {
            let val = storage.get(idx).map_err(|e| invalid(format!("{e}")))?;
            result[idx] = if val > 0 {
                val as i32 + min_y - 1
            } else {
                min_y
            };
        }
        Ok(result)
    }

    pub fn to_nbt(&self, min_y: i32) -> Tag {
        let mut map = Compound::new();
        map.insert(key(HeightmapKind::WorldSurface.as_str()), Self::pack_array(&self.world_surface, min_y));
        map.insert(key(HeightmapKind::OceanFloor.as_str()), Self::pack_array(&self.ocean_floor, min_y));
        map.insert(key(HeightmapKind::MotionBlocking.as_str()), Self::pack_array(&self.motion_blocking, min_y));
        map.insert(
            key(HeightmapKind::MotionBlockingNoLeaves.as_str()),
            Self::pack_array(&self.motion_blocking_no_leaves, min_y),
        );
        Tag::Compound(map)
    }

    pub fn from_nbt(tag: &Tag, min_y: i32) -> io::Result<Self> {
        let mut heightmaps = Self::new(min_y);
        let Tag::Compound(map) = tag else {
            return Ok(heightmaps);
        };
        if let Some(Tag::LongArray(words)) = map.get(&key(HeightmapKind::WorldSurface.as_str())) {
            if let Ok(arr) = Self::unpack_array(words, min_y) {
                heightmaps.world_surface = arr;
            }
        }
        if let Some(Tag::LongArray(words)) = map.get(&key(HeightmapKind::OceanFloor.as_str())) {
            if let Ok(arr) = Self::unpack_array(words, min_y) {
                heightmaps.ocean_floor = arr;
            }
        }
        if let Some(Tag::LongArray(words)) = map.get(&key(HeightmapKind::MotionBlocking.as_str())) {
            if let Ok(arr) = Self::unpack_array(words, min_y) {
                heightmaps.motion_blocking = arr;
            }
        }
        if let Some(Tag::LongArray(words)) = map.get(&key(HeightmapKind::MotionBlockingNoLeaves.as_str())) {
            if let Ok(arr) = Self::unpack_array(words, min_y) {
                heightmaps.motion_blocking_no_leaves = arr;
            }
        }
        Ok(heightmaps)
    }
}

/// A 16x16x16 chunk section in Vanilla 1.20.1 Anvil format.
#[derive(Clone, Debug, PartialEq)]
pub struct Legacy1201Section {
    pub y: i8,
    pub block_palette: Vec<Legacy1201BlockState>,
    pub block_storage: Option<PackedStorage>,
    pub biome_palette: Vec<String>,
    pub biome_storage: Option<PackedStorage>,
    pub block_light: Option<[u8; LIGHT_BYTES_PER_SECTION]>,
    pub sky_light: Option<[u8; LIGHT_BYTES_PER_SECTION]>,
}

impl Legacy1201Section {
    pub fn uniform(y: i8, block: Legacy1201BlockState, biome: &str) -> Self {
        Self {
            y,
            block_palette: vec![block],
            block_storage: None,
            biome_palette: vec![biome.to_string()],
            biome_storage: None,
            block_light: None,
            sky_light: None,
        }
    }

    #[inline]
    pub fn block_index(x: usize, y: usize, z: usize) -> usize {
        ((y & 15) << 8) | ((z & 15) << 4) | (x & 15)
    }

    #[inline]
    pub fn quart_index(quart_x: usize, quart_y: usize, quart_z: usize) -> usize {
        ((quart_y & 3) << 4) | ((quart_z & 3) << 2) | (quart_x & 3)
    }

    pub fn get_block(&self, x: usize, y: usize, z: usize) -> &Legacy1201BlockState {
        let idx = Self::block_index(x, y, z);
        let palette_idx = match &self.block_storage {
            Some(storage) => storage.get(idx).unwrap_or(0) as usize,
            None => 0,
        };
        self.block_palette.get(palette_idx).unwrap_or(&self.block_palette[0])
    }

    pub fn set_block(&mut self, x: usize, y: usize, z: usize, state: Legacy1201BlockState) -> Legacy1201BlockState {
        let idx = Self::block_index(x, y, z);
        let old = self.get_block(x, y, z).clone();
        if old == state {
            return old;
        }

        let palette_idx = if let Some(pos) = self.block_palette.iter().position(|s| *s == state) {
            pos
        } else {
            self.block_palette.push(state.clone());
            self.block_palette.len() - 1
        };

        let bits = bits_for_palette(self.block_palette.len(), 4);
        if let Some(storage) = &mut self.block_storage {
            if bits > storage.bits() {
                let mut new_storage = PackedStorage::new(bits, BLOCKS_PER_SECTION).expect("valid bits");
                for i in 0..BLOCKS_PER_SECTION {
                    let val = storage.get(i).unwrap_or(0);
                    let _ = new_storage.set(i, val);
                }
                let _ = new_storage.set(idx, palette_idx as u32);
                self.block_storage = Some(new_storage);
            } else {
                let _ = storage.set(idx, palette_idx as u32);
            }
        } else {
            let mut storage = PackedStorage::new(bits.max(4), BLOCKS_PER_SECTION).expect("valid bits");
            let _ = storage.set(idx, palette_idx as u32);
            self.block_storage = Some(storage);
        }
        old
    }

    pub fn get_biome(&self, quart_x: usize, quart_y: usize, quart_z: usize) -> &str {
        let idx = Self::quart_index(quart_x, quart_y, quart_z);
        let palette_idx = match &self.biome_storage {
            Some(storage) => storage.get(idx).unwrap_or(0) as usize,
            None => 0,
        };
        self.biome_palette.get(palette_idx).map(|s| s.as_str()).unwrap_or("minecraft:plains")
    }

    pub fn set_biome(&mut self, quart_x: usize, quart_y: usize, quart_z: usize, biome: String) {
        let idx = Self::quart_index(quart_x, quart_y, quart_z);
        let palette_idx = if let Some(pos) = self.biome_palette.iter().position(|b| *b == biome) {
            pos
        } else {
            self.biome_palette.push(biome);
            self.biome_palette.len() - 1
        };

        let bits = bits_for_palette(self.biome_palette.len(), 1);
        if let Some(storage) = &mut self.biome_storage {
            if bits > storage.bits() {
                let mut new_storage = PackedStorage::new(bits, BIOMES_PER_SECTION).expect("valid bits");
                for i in 0..BIOMES_PER_SECTION {
                    let val = storage.get(i).unwrap_or(0);
                    let _ = new_storage.set(i, val);
                }
                let _ = new_storage.set(idx, palette_idx as u32);
                self.biome_storage = Some(new_storage);
            } else {
                let _ = storage.set(idx, palette_idx as u32);
            }
        } else if self.biome_palette.len() > 1 {
            let mut storage = PackedStorage::new(bits.max(1), BIOMES_PER_SECTION).expect("valid bits");
            let _ = storage.set(idx, palette_idx as u32);
            self.biome_storage = Some(storage);
        }
    }

    pub fn get_block_light(&self, x: usize, y: usize, z: usize) -> u8 {
        let Some(bytes) = &self.block_light else {
            return 0;
        };
        let idx = Self::block_index(x, y, z);
        let byte = bytes[idx >> 1];
        if (idx & 1) == 0 {
            byte & 0x0F
        } else {
            (byte >> 4) & 0x0F
        }
    }

    pub fn set_block_light(&mut self, x: usize, y: usize, z: usize, level: u8) {
        let bytes = self.block_light.get_or_insert([0u8; LIGHT_BYTES_PER_SECTION]);
        let idx = Self::block_index(x, y, z);
        let byte_idx = idx >> 1;
        let shift = ((idx & 1) * 4) as u8;
        let mask = !(0x0F << shift);
        bytes[byte_idx] = (bytes[byte_idx] & mask) | ((level & 0x0F) << shift);
    }

    pub fn get_sky_light(&self, x: usize, y: usize, z: usize) -> u8 {
        let Some(bytes) = &self.sky_light else {
            return 0;
        };
        let idx = Self::block_index(x, y, z);
        let byte = bytes[idx >> 1];
        if (idx & 1) == 0 {
            byte & 0x0F
        } else {
            (byte >> 4) & 0x0F
        }
    }

    pub fn set_sky_light(&mut self, x: usize, y: usize, z: usize, level: u8) {
        let bytes = self.sky_light.get_or_insert([0u8; LIGHT_BYTES_PER_SECTION]);
        let idx = Self::block_index(x, y, z);
        let byte_idx = idx >> 1;
        let shift = ((idx & 1) * 4) as u8;
        let mask = !(0x0F << shift);
        bytes[byte_idx] = (bytes[byte_idx] & mask) | ((level & 0x0F) << shift);
    }

    pub fn is_empty(&self) -> bool {
        self.block_palette.len() == 1 && self.block_palette[0].is_air()
    }

    pub fn to_nbt(&self) -> Tag {
        let mut map = Compound::new();
        map.insert(key("Y"), Tag::Byte(self.y));

        let mut bs_map = Compound::new();
        let palette_tags: Vec<Tag> = self.block_palette.iter().map(Legacy1201BlockState::to_nbt).collect();
        bs_map.insert(key("palette"), Tag::List(palette_tags));
        if let Some(storage) = &self.block_storage {
            bs_map.insert(
                key("data"),
                Tag::LongArray(storage.words().iter().map(|&w| w as i64).collect()),
            );
        }
        map.insert(key("block_states"), Tag::Compound(bs_map));

        let mut biomes_map = Compound::new();
        let biome_tags: Vec<Tag> = self.biome_palette.iter().map(|b| Tag::String(key(b))).collect();
        biomes_map.insert(key("palette"), Tag::List(biome_tags));
        if let Some(storage) = &self.biome_storage {
            biomes_map.insert(
                key("data"),
                Tag::LongArray(storage.words().iter().map(|&w| w as i64).collect()),
            );
        }
        map.insert(key("biomes"), Tag::Compound(biomes_map));

        if let Some(bl) = &self.block_light {
            map.insert(key("BlockLight"), Tag::ByteArray(bl.to_vec()));
        }
        if let Some(sl) = &self.sky_light {
            map.insert(key("SkyLight"), Tag::ByteArray(sl.to_vec()));
        }

        Tag::Compound(map)
    }

    pub fn from_nbt(map: &Compound) -> io::Result<Self> {
        let Some(Tag::Byte(y)) = map.get(&key("Y")) else {
            return Err(invalid("section missing byte Y"));
        };
        let y = *y;

        let mut block_palette = vec![Legacy1201BlockState::air()];
        let mut block_storage = None;
        if let Some(Tag::Compound(bs_map)) = map.get(&key("block_states")) {
            if let Some(Tag::List(list)) = bs_map.get(&key("palette")) {
                let resolved: Vec<Legacy1201BlockState> = list
                    .iter()
                    .map(Legacy1201BlockState::from_nbt)
                    .collect::<io::Result<Vec<_>>>()?;
                if !resolved.is_empty() {
                    block_palette = resolved;
                }
            }
            if let Some(Tag::LongArray(words)) = bs_map.get(&key("data")) {
                let bits = bits_for_palette(block_palette.len(), 4);
                let uwords: Vec<u64> = words.iter().map(|&w| w as u64).collect();
                if let Ok(storage) = PackedStorage::from_words(bits, BLOCKS_PER_SECTION, uwords) {
                    block_storage = Some(storage);
                }
            }
        }

        let mut biome_palette = vec!["minecraft:plains".to_string()];
        let mut biome_storage = None;
        if let Some(Tag::Compound(b_map)) = map.get(&key("biomes")) {
            if let Some(Tag::List(list)) = b_map.get(&key("palette")) {
                let mut b_list = Vec::new();
                for tag in list {
                    if let Tag::String(js) = tag {
                        if let Ok(s) = js.to_string() {
                            b_list.push(s);
                        }
                    }
                }
                if !b_list.is_empty() {
                    biome_palette = b_list;
                }
            }
            if let Some(Tag::LongArray(words)) = b_map.get(&key("data")) {
                let bits = bits_for_palette(biome_palette.len(), 1);
                let uwords: Vec<u64> = words.iter().map(|&w| w as u64).collect();
                if let Ok(storage) = PackedStorage::from_words(bits, BIOMES_PER_SECTION, uwords) {
                    biome_storage = Some(storage);
                }
            }
        }

        let block_light = if let Some(Tag::ByteArray(bl)) = map.get(&key("BlockLight")) {
            if bl.len() == LIGHT_BYTES_PER_SECTION {
                let mut arr = [0u8; LIGHT_BYTES_PER_SECTION];
                arr.copy_from_slice(bl);
                Some(arr)
            } else {
                None
            }
        } else {
            None
        };

        let sky_light = if let Some(Tag::ByteArray(sl)) = map.get(&key("SkyLight")) {
            if sl.len() == LIGHT_BYTES_PER_SECTION {
                let mut arr = [0u8; LIGHT_BYTES_PER_SECTION];
                arr.copy_from_slice(sl);
                Some(arr)
            } else {
                None
            }
        } else {
            None
        };

        Ok(Self {
            y,
            block_palette,
            block_storage,
            biome_palette,
            biome_storage,
            block_light,
            sky_light,
        })
    }
}

/// Vanilla 1.20.1 Anvil chunk representation.
#[derive(Clone, Debug, PartialEq)]
pub struct Legacy1201Chunk {
    pub x: i32,
    pub z: i32,
    pub min_section_y: i8,
    pub section_count: usize,
    pub status: String,
    pub last_update: i64,
    pub inhabited_time: i64,
    pub sections: Vec<Legacy1201Section>,
    pub heightmaps: Legacy1201Heightmaps,
    pub block_entities: Vec<Tag>,
    pub block_ticks: Vec<Tag>,
    pub fluid_ticks: Vec<Tag>,
    pub extra: Compound,
}

impl Legacy1201Chunk {
    pub fn new_overworld(x: i32, z: i32, default_block: Legacy1201BlockState, default_biome: &str) -> Self {
        let mut sections = Vec::with_capacity(OVERWORLD_SECTION_COUNT);
        for y in OVERWORLD_MIN_SECTION_Y..=OVERWORLD_MAX_SECTION_Y {
            sections.push(Legacy1201Section::uniform(y, default_block.clone(), default_biome));
        }
        let heightmaps = Legacy1201Heightmaps::new(OVERWORLD_MIN_Y);
        Self {
            x,
            z,
            min_section_y: OVERWORLD_MIN_SECTION_Y,
            section_count: OVERWORLD_SECTION_COUNT,
            status: "minecraft:full".to_string(),
            last_update: 0,
            inhabited_time: 0,
            sections,
            heightmaps,
            block_entities: Vec::new(),
            block_ticks: Vec::new(),
            fluid_ticks: Vec::new(),
            extra: Compound::new(),
        }
    }

    pub fn pos(&self) -> ChunkPos {
        ChunkPos::new(self.x, self.z)
    }

    pub fn min_y(&self) -> i32 {
        (self.min_section_y as i32) * 16
    }

    pub fn max_y(&self) -> i32 {
        self.min_y() + (self.section_count as i32) * 16
    }

    #[inline]
    fn section_index(&self, y: i32) -> Option<usize> {
        let sec_y = (y >> 4) as i8;
        let idx = (sec_y - self.min_section_y) as usize;
        if idx < self.sections.len() {
            Some(idx)
        } else {
            None
        }
    }

    pub fn get_block(&self, pos: BlockPos) -> Option<&Legacy1201BlockState> {
        let sec_idx = self.section_index(pos.y)?;
        let sec = &self.sections[sec_idx];
        let lx = (pos.x & 15) as usize;
        let ly = (pos.y & 15) as usize;
        let lz = (pos.z & 15) as usize;
        Some(sec.get_block(lx, ly, lz))
    }

    pub fn set_block(&mut self, pos: BlockPos, state: Legacy1201BlockState) -> io::Result<Option<Legacy1201BlockState>> {
        let Some(sec_idx) = self.section_index(pos.y) else {
            return Ok(None);
        };
        let sec = &mut self.sections[sec_idx];
        let lx = (pos.x & 15) as usize;
        let ly = (pos.y & 15) as usize;
        let lz = (pos.z & 15) as usize;
        let old = sec.set_block(lx, ly, lz, state);
        Ok(Some(old))
    }

    pub fn get_biome(&self, pos: BlockPos) -> Option<&str> {
        let sec_idx = self.section_index(pos.y)?;
        let sec = &self.sections[sec_idx];
        let qx = ((pos.x & 15) >> 2) as usize;
        let qy = ((pos.y & 15) >> 2) as usize;
        let qz = ((pos.z & 15) >> 2) as usize;
        Some(sec.get_biome(qx, qy, qz))
    }

    pub fn set_biome(&mut self, pos: BlockPos, biome: String) -> io::Result<()> {
        let Some(sec_idx) = self.section_index(pos.y) else {
            return Err(invalid("block position outside chunk height"));
        };
        let sec = &mut self.sections[sec_idx];
        let qx = ((pos.x & 15) >> 2) as usize;
        let qy = ((pos.y & 15) >> 2) as usize;
        let qz = ((pos.z & 15) >> 2) as usize;
        sec.set_biome(qx, qy, qz, biome);
        Ok(())
    }

    pub fn recompute_heightmaps(&mut self) {
        let min_y = self.min_y();
        let max_y = self.max_y();

        for z in 0..16 {
            for x in 0..16 {
                let mut ws = min_y;
                let mut of = min_y;
                let mut mb = min_y;
                let mut mbnl = min_y;

                for y in (min_y..max_y).rev() {
                    let sec_idx = ((y >> 4) as i8 - self.min_section_y) as usize;
                    let sec = &self.sections[sec_idx];
                    let ly = (y & 15) as usize;
                    let block = sec.get_block(x, ly, z);

                    if !block.is_air() {
                        if ws == min_y {
                            ws = y + 1;
                        }
                        if !block.is_fluid() && of == min_y {
                            of = y + 1;
                        }
                    }

                    if block.blocks_movement() || block.is_fluid() {
                        if mb == min_y {
                            mb = y + 1;
                        }
                        if !block.is_leaves() && mbnl == min_y {
                            mbnl = y + 1;
                        }
                    }

                    if ws > min_y && of > min_y && mb > min_y && mbnl > min_y {
                        break;
                    }
                }

                self.heightmaps.set(x, z, HeightmapKind::WorldSurface, ws);
                self.heightmaps.set(x, z, HeightmapKind::OceanFloor, of);
                self.heightmaps.set(x, z, HeightmapKind::MotionBlocking, mb);
                self.heightmaps.set(x, z, HeightmapKind::MotionBlockingNoLeaves, mbnl);
            }
        }
    }

    pub fn to_nbt(&self) -> NamedTag {
        let mut root = self.extra.clone();
        root.insert(key("DataVersion"), Tag::Int(LEGACY_1_20_1_DATA_VERSION));
        root.insert(key("xPos"), Tag::Int(self.x));
        root.insert(key("yPos"), Tag::Int(self.min_section_y as i32));
        root.insert(key("zPos"), Tag::Int(self.z));
        root.insert(key("Status"), Tag::String(key(&self.status)));
        root.insert(key("LastUpdate"), Tag::Long(self.last_update));
        root.insert(key("InhabitedTime"), Tag::Long(self.inhabited_time));

        let section_tags: Vec<Tag> = self.sections.iter().map(Legacy1201Section::to_nbt).collect();
        root.insert(key("sections"), Tag::List(section_tags));
        root.insert(key("Heightmaps"), self.heightmaps.to_nbt(self.min_y()));
        root.insert(key("block_entities"), Tag::List(self.block_entities.clone()));
        root.insert(key("block_ticks"), Tag::List(self.block_ticks.clone()));
        root.insert(key("fluid_ticks"), Tag::List(self.fluid_ticks.clone()));

        NamedTag {
            name: JavaString::default(),
            tag: Tag::Compound(root),
        }
    }

    pub fn from_nbt(named: &NamedTag) -> io::Result<Self> {
        let Tag::Compound(map) = &named.tag else {
            return Err(invalid("chunk root tag is not a compound"));
        };
        let Some(Tag::Int(ver)) = map.get(&key("DataVersion")) else {
            return Err(invalid("chunk missing DataVersion"));
        };
        if *ver != LEGACY_1_20_1_DATA_VERSION {
            return Err(invalid(format!(
                "expected 1.20.1 DataVersion {LEGACY_1_20_1_DATA_VERSION}, got {ver}"
            )));
        }
        let Some(Tag::Int(x)) = map.get(&key("xPos")) else {
            return Err(invalid("chunk missing xPos"));
        };
        let Some(Tag::Int(z)) = map.get(&key("zPos")) else {
            return Err(invalid("chunk missing zPos"));
        };
        let x = *x;
        let z = *z;

        let min_sec_y = if let Some(Tag::Int(y_pos)) = map.get(&key("yPos")) {
            *y_pos as i8
        } else {
            OVERWORLD_MIN_SECTION_Y
        };

        let status = if let Some(Tag::String(s)) = map.get(&key("Status")) {
            s.to_string().unwrap_or_else(|_| "minecraft:full".to_string())
        } else {
            "minecraft:full".to_string()
        };

        let last_update = if let Some(Tag::Long(lu)) = map.get(&key("LastUpdate")) {
            *lu
        } else {
            0
        };

        let inhabited_time = if let Some(Tag::Long(it)) = map.get(&key("InhabitedTime")) {
            *it
        } else {
            0
        };

        let mut sections = Vec::new();
        if let Some(Tag::List(list)) = map.get(&key("sections")) {
            for stag in list {
                if let Tag::Compound(smap) = stag {
                    sections.push(Legacy1201Section::from_nbt(smap)?);
                }
            }
        }
        sections.sort_by_key(|s| s.y);

        let section_count = sections.len().max(OVERWORLD_SECTION_COUNT);
        let min_y = (min_sec_y as i32) * 16;
        let heightmaps = if let Some(h_tag) = map.get(&key("Heightmaps")) {
            Legacy1201Heightmaps::from_nbt(h_tag, min_y)?
        } else {
            Legacy1201Heightmaps::new(min_y)
        };

        let block_entities = if let Some(Tag::List(l)) = map.get(&key("block_entities")) {
            l.clone()
        } else {
            Vec::new()
        };

        let block_ticks = if let Some(Tag::List(l)) = map.get(&key("block_ticks")) {
            l.clone()
        } else {
            Vec::new()
        };

        let fluid_ticks = if let Some(Tag::List(l)) = map.get(&key("fluid_ticks")) {
            l.clone()
        } else {
            Vec::new()
        };

        let mut extra = map.clone();
        for k in [
            "DataVersion", "xPos", "yPos", "zPos", "Status", "LastUpdate",
            "InhabitedTime", "sections", "Heightmaps", "block_entities", "block_ticks", "fluid_ticks"
        ] {
            extra.remove(&key(k));
        }

        Ok(Self {
            x,
            z,
            min_section_y: min_sec_y,
            section_count,
            status,
            last_update,
            inhabited_time,
            sections,
            heightmaps,
            block_entities,
            block_ticks,
            fluid_ticks,
            extra,
        })
    }

    pub fn to_chunk_record(&self, compression: Compression, timestamp: u32) -> io::Result<ChunkRecord> {
        let named = self.to_nbt();
        ChunkRecord::from_nbt(&named, compression, timestamp, Limits::saved())
    }

    pub fn from_chunk_record(record: &ChunkRecord) -> io::Result<Self> {
        let named = record.nbt(Limits::saved())?;
        Self::from_nbt(&named)
    }
}

/// Vanilla 1.20.1 Superflat world generator.
#[derive(Clone, Debug)]
pub struct Legacy1201SuperflatGenerator {
    pub layers: Vec<(Legacy1201BlockState, usize)>,
    pub biome: String,
}

impl Legacy1201SuperflatGenerator {
    pub fn classic() -> Self {
        Self {
            layers: vec![
                (Legacy1201BlockState::bedrock(), 1),
                (Legacy1201BlockState::dirt(), 2),
                (Legacy1201BlockState::grass_block(), 1),
            ],
            biome: "minecraft:plains".to_string(),
        }
    }

    pub fn custom(layers: Vec<(Legacy1201BlockState, usize)>, biome: impl Into<String>) -> Self {
        Self {
            layers,
            biome: biome.into(),
        }
    }

    pub fn generate_chunk(&self, chunk_x: i32, chunk_z: i32) -> Legacy1201Chunk {
        let mut chunk = Legacy1201Chunk::new_overworld(
            chunk_x,
            chunk_z,
            Legacy1201BlockState::air(),
            &self.biome,
        );

        let mut cur_y = OVERWORLD_MIN_Y;
        for (state, count) in &self.layers {
            for _ in 0..*count {
                if cur_y >= OVERWORLD_MAX_Y {
                    break;
                }
                let sec_idx = ((cur_y >> 4) as i8 - OVERWORLD_MIN_SECTION_Y) as usize;
                let ly = (cur_y & 15) as usize;
                for z in 0..16 {
                    for x in 0..16 {
                        chunk.sections[sec_idx].set_block(x, ly, z, state.clone());
                    }
                }
                cur_y += 1;
            }
        }

        let ground_top_y = cur_y - 1;
        for y in OVERWORLD_MIN_SECTION_Y..=OVERWORLD_MAX_SECTION_Y {
            let sec_idx = (y - OVERWORLD_MIN_SECTION_Y) as usize;
            let sec = &mut chunk.sections[sec_idx];
            let sec_base_y = (y as i32) * 16;
            for ly in 0..16 {
                let by = sec_base_y + ly as i32;
                let light = if by > ground_top_y { 15 } else { 0 };
                for z in 0..16 {
                    for x in 0..16 {
                        sec.set_sky_light(x, ly, z, light);
                    }
                }
            }
        }

        chunk.recompute_heightmaps();
        chunk
    }
}

/// Vanilla 1.20.1 `level.dat` world metadata and settings.
#[derive(Clone, Debug, PartialEq)]
pub struct Legacy1201LevelData {
    pub level_name: String,
    pub generator_name: String,
    pub seed: i64,
    pub game_type: i32,
    pub difficulty: i8,
    pub hardcore: bool,
    pub spawn_x: i32,
    pub spawn_y: i32,
    pub spawn_z: i32,
    pub time: i64,
    pub day_time: i64,
    pub data_version: i32,
    pub extra: Compound,
}

impl Legacy1201LevelData {
    pub fn new(level_name: impl Into<String>, seed: i64, generator_name: impl Into<String>) -> Self {
        Self {
            level_name: level_name.into(),
            generator_name: generator_name.into(),
            seed,
            game_type: 0,
            difficulty: 2,
            hardcore: false,
            spawn_x: 0,
            spawn_y: -60,
            spawn_z: 0,
            time: 0,
            day_time: 0,
            data_version: LEGACY_1_20_1_DATA_VERSION,
            extra: Compound::new(),
        }
    }

    pub fn to_nbt(&self) -> NamedTag {
        let mut data = self.extra.clone();
        data.insert(key("DataVersion"), Tag::Int(self.data_version));
        data.insert(key("LevelName"), Tag::String(key(&self.level_name)));
        data.insert(key("generatorName"), Tag::String(key(&self.generator_name)));
        data.insert(key("generatorVersion"), Tag::Int(0));
        data.insert(key("RandomSeed"), Tag::Long(self.seed));
        data.insert(key("GameType"), Tag::Int(self.game_type));
        data.insert(key("Difficulty"), Tag::Byte(self.difficulty));
        data.insert(key("DifficultyLocked"), Tag::Byte(0));
        data.insert(key("hardcore"), Tag::Byte(if self.hardcore { 1 } else { 0 }));
        data.insert(key("SpawnX"), Tag::Int(self.spawn_x));
        data.insert(key("SpawnY"), Tag::Int(self.spawn_y));
        data.insert(key("SpawnZ"), Tag::Int(self.spawn_z));
        data.insert(key("Time"), Tag::Long(self.time));
        data.insert(key("DayTime"), Tag::Long(self.day_time));
        data.insert(key("MapFeatures"), Tag::Byte(1));
        data.insert(key("initialized"), Tag::Byte(1));
        data.insert(key("version"), Tag::Int(19133));

        let mut ver = Compound::new();
        ver.insert(key("Id"), Tag::Int(self.data_version));
        ver.insert(key("Name"), Tag::String(key("1.20.1")));
        ver.insert(key("Series"), Tag::String(key("main")));
        ver.insert(key("Snapshot"), Tag::Byte(0));
        data.insert(key("Version"), Tag::Compound(ver));

        let mut root = Compound::new();
        root.insert(key("Data"), Tag::Compound(data));
        NamedTag {
            name: JavaString::default(),
            tag: Tag::Compound(root),
        }
    }

    pub fn from_nbt(named: &NamedTag) -> io::Result<Self> {
        let Tag::Compound(root) = &named.tag else {
            return Err(invalid("level.dat root tag is not a compound"));
        };
        let Some(Tag::Compound(data)) = root.get(&key("Data")) else {
            return Err(invalid("level.dat missing Data compound"));
        };

        let data_version = if let Some(Tag::Int(v)) = data.get(&key("DataVersion")) {
            *v
        } else {
            LEGACY_1_20_1_DATA_VERSION
        };

        let level_name = if let Some(Tag::String(s)) = data.get(&key("LevelName")) {
            s.to_string().unwrap_or_default()
        } else {
            "Horizon 1.20.1".to_string()
        };

        let generator_name = if let Some(Tag::String(s)) = data.get(&key("generatorName")) {
            s.to_string().unwrap_or_else(|_| "flat".to_string())
        } else {
            "flat".to_string()
        };

        let seed = if let Some(Tag::Long(s)) = data.get(&key("RandomSeed")) {
            *s
        } else {
            0
        };

        let game_type = if let Some(Tag::Int(gt)) = data.get(&key("GameType")) {
            *gt
        } else {
            0
        };

        let difficulty = if let Some(Tag::Byte(d)) = data.get(&key("Difficulty")) {
            *d
        } else {
            2
        };

        let hardcore = if let Some(Tag::Byte(h)) = data.get(&key("hardcore")) {
            *h != 0
        } else {
            false
        };

        let spawn_x = if let Some(Tag::Int(x)) = data.get(&key("SpawnX")) { *x } else { 0 };
        let spawn_y = if let Some(Tag::Int(y)) = data.get(&key("SpawnY")) { *y } else { -60 };
        let spawn_z = if let Some(Tag::Int(z)) = data.get(&key("SpawnZ")) { *z } else { 0 };
        let time = if let Some(Tag::Long(t)) = data.get(&key("Time")) { *t } else { 0 };
        let day_time = if let Some(Tag::Long(dt)) = data.get(&key("DayTime")) { *dt } else { 0 };

        let mut extra = data.clone();
        for k in [
            "DataVersion", "LevelName", "generatorName", "generatorVersion", "RandomSeed",
            "GameType", "Difficulty", "DifficultyLocked", "hardcore", "SpawnX", "SpawnY",
            "SpawnZ", "Time", "DayTime", "MapFeatures", "initialized", "version", "Version"
        ] {
            extra.remove(&key(k));
        }

        Ok(Self {
            level_name,
            generator_name,
            seed,
            game_type,
            difficulty,
            hardcore,
            spawn_x,
            spawn_y,
            spawn_z,
            time,
            day_time,
            data_version,
            extra,
        })
    }

    pub fn write_to_file(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let named = self.to_nbt();
        let bytes = nbt::write_named(&named, Limits::saved())
            .map_err(|e| invalid(format!("failed to serialize level.dat: {e}")))?;
        let compressed = Compression::Gzip.encode(&bytes)?;
        fs::write(path, compressed)
    }

    pub fn read_from_file(path: impl AsRef<Path>) -> io::Result<Self> {
        let compressed = fs::read(path)?;
        let mut bytes = Vec::new();
        Compression::Gzip.decode_into(&compressed, 64 * 1024 * 1024, &mut bytes)?;
        let (named, _) = nbt::read_named(&bytes, Limits::saved())
            .map_err(|e| invalid(format!("failed to deserialize level.dat: {e}")))?;
        Self::from_nbt(&named)
    }
}

/// Region file storage for a 1.20.1 world dimension.
pub struct Legacy1201RegionStorage {
    region_dir: PathBuf,
}

impl Legacy1201RegionStorage {
    pub fn new(region_dir: impl Into<PathBuf>) -> Self {
        Self {
            region_dir: region_dir.into(),
        }
    }

    pub fn region_coords(chunk: ChunkPos) -> (i32, i32) {
        (chunk.x.div_euclid(32), chunk.z.div_euclid(32))
    }

    pub fn region_path(&self, region_x: i32, region_z: i32) -> PathBuf {
        self.region_dir.join(format!("r.{region_x}.{region_z}.mca"))
    }

    pub fn load_chunk(&self, chunk_pos: ChunkPos) -> io::Result<Option<Legacy1201Chunk>> {
        let (rx, rz) = Self::region_coords(chunk_pos);
        let path = self.region_path(rx, rz);
        if !path.exists() {
            return Ok(None);
        }
        let mut region = RegionFile::open(&path)?;
        let Some(record) = region.read(chunk_pos)? else {
            return Ok(None);
        };
        Legacy1201Chunk::from_chunk_record(&record).map(Some)
    }

    pub fn save_chunk(&self, chunk: &Legacy1201Chunk, timestamp: u32) -> io::Result<()> {
        let pos = chunk.pos();
        let (rx, rz) = Self::region_coords(pos);
        fs::create_dir_all(&self.region_dir)?;
        let path = self.region_path(rx, rz);
        let mut region = RegionFile::open_writable(&path, true)?;
        let record = chunk.to_chunk_record(Compression::Zlib, timestamp)?;
        region.write(pos, &record)?;
        region.sync()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_legacy1201_block_state_nbt_roundtrip() {
        let state = Legacy1201BlockState::grass_block();
        let tag = state.to_nbt();
        let parsed = Legacy1201BlockState::from_nbt(&tag).unwrap();
        assert_eq!(state, parsed);
        assert_eq!(parsed.name, "minecraft:grass_block");
        assert_eq!(parsed.properties.get("snowy").map(|s| s.as_str()), Some("false"));
    }

    #[test]
    fn test_legacy1201_block_state_parse() {
        let parsed = Legacy1201BlockState::parse("minecraft:oak_stairs[facing=east,half=bottom,shape=straight,waterlogged=false]").unwrap();
        assert_eq!(parsed.name, "minecraft:oak_stairs");
        assert_eq!(parsed.properties.get("facing").unwrap(), "east");
        assert_eq!(parsed.properties.get("waterlogged").unwrap(), "false");
    }

    #[test]
    fn test_legacy1201_section_palette_growth() {
        let mut sec = Legacy1201Section::uniform(0, Legacy1201BlockState::air(), "minecraft:plains");
        assert_eq!(sec.block_palette.len(), 1);
        assert!(sec.block_storage.is_none());

        sec.set_block(0, 0, 0, Legacy1201BlockState::stone());
        assert_eq!(sec.block_palette.len(), 2);
        assert!(sec.block_storage.is_some());
        assert_eq!(sec.get_block(0, 0, 0), &Legacy1201BlockState::stone());
        assert_eq!(sec.get_block(1, 0, 0), &Legacy1201BlockState::air());
    }

    #[test]
    fn test_legacy1201_superflat_generation_and_nbt_roundtrip() {
        let generator = Legacy1201SuperflatGenerator::classic();
        let chunk = generator.generate_chunk(0, 0);

        assert_eq!(chunk.x, 0);
        assert_eq!(chunk.z, 0);
        assert_eq!(chunk.sections.len(), 24);

        assert_eq!(
            chunk.get_block(BlockPos::new(0, -64, 0)).unwrap(),
            &Legacy1201BlockState::bedrock()
        );
        assert_eq!(
            chunk.get_block(BlockPos::new(5, -63, 5)).unwrap(),
            &Legacy1201BlockState::dirt()
        );
        assert_eq!(
            chunk.get_block(BlockPos::new(5, -62, 5)).unwrap(),
            &Legacy1201BlockState::dirt()
        );
        assert_eq!(
            chunk.get_block(BlockPos::new(10, -61, 10)).unwrap(),
            &Legacy1201BlockState::grass_block()
        );
        assert_eq!(
            chunk.get_block(BlockPos::new(0, -60, 0)).unwrap(),
            &Legacy1201BlockState::air()
        );

        assert_eq!(chunk.get_biome(BlockPos::new(0, -64, 0)).unwrap(), "minecraft:plains");

        assert_eq!(chunk.heightmaps.get(0, 0, HeightmapKind::WorldSurface), -60);
        assert_eq!(chunk.heightmaps.get(0, 0, HeightmapKind::OceanFloor), -60);
        assert_eq!(chunk.heightmaps.get(0, 0, HeightmapKind::MotionBlocking), -60);
        assert_eq!(chunk.heightmaps.get(0, 0, HeightmapKind::MotionBlockingNoLeaves), -60);

        let named = chunk.to_nbt();
        let loaded = Legacy1201Chunk::from_nbt(&named).unwrap();
        assert_eq!(loaded.x, chunk.x);
        assert_eq!(loaded.z, chunk.z);
        assert_eq!(loaded.sections.len(), chunk.sections.len());
        assert_eq!(
            loaded.get_block(BlockPos::new(0, -64, 0)).unwrap(),
            &Legacy1201BlockState::bedrock()
        );
        assert_eq!(
            loaded.get_block(BlockPos::new(10, -61, 10)).unwrap(),
            &Legacy1201BlockState::grass_block()
        );
    }

    #[test]
    fn test_legacy1201_level_data_nbt_roundtrip() {
        let level = Legacy1201LevelData::new("TestWorld", 123456789, "flat");
        let named = level.to_nbt();
        let loaded = Legacy1201LevelData::from_nbt(&named).unwrap();
        assert_eq!(loaded.level_name, "TestWorld");
        assert_eq!(loaded.seed, 123456789);
        assert_eq!(loaded.generator_name, "flat");
        assert_eq!(loaded.data_version, LEGACY_1_20_1_DATA_VERSION);
    }
}
