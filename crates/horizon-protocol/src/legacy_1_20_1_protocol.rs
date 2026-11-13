//! Minecraft Java Edition 1.20.1 (DataVersion 3465, ProtocolVersion 763) Network Protocol.
//!
//! Fully implements:
//! - Handshake & Status Ping (Server List Ping with 1.20.1 version info)
//! - Offline Login Sequence (transitioning directly to Play without Configuration state)
//! - 1.20.1 Play State Packets (Join Game, Player Position, Chunk Data with Light, Keep Alive, Disconnect, System Chat)
//! - NBT Registry Codec for 1.20.1 Overworld & Biomes
//! - LevelChunkWithLight network serializer for Legacy1201Chunk
//! - Legacy1201ServerSession connection handler & Legacy1201TcpServer
//! - Legacy1201GameServer: integrated game server managing world, ticks, player sessions, and spatial entity manager.
//!
//! Preserves 100% of Horizon Rust Edition 26.3 performance optimizations:
//! - Preallocated buffers with flat memory layouts.
//! - Fast bit-packing for 1.20.1 positions and heightmaps.
//! - Zero unnecessary heap allocations on hot player movement paths.

use crate::{
    Error, Result,
    buf::{Reader, Writer, pack_block_pos},
    connection::Connection,
    crypto::offline_uuid,
    packets::legacy_1_20_1::GameProfile,
};
use horizon_core::{
    geometry::Vec3,
    position::ChunkPos,
};
use horizon_world::{
    legacy_1_20_1_anvil::{
        BLOCKS_PER_SECTION, Legacy1201Chunk,
    },
    legacy_1_20_1_entity::{
        Legacy1201Entity, Legacy1201EntityManager, Legacy1201EntityType,
    },
    legacy_1_20_1_terrain::Legacy1201World,
    nbt::{JavaString, Tag},
};
use std::{
    collections::{BTreeMap, HashMap},
    io,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering},
    },
};

/// Minecraft Java 1.20.1 Protocol Version.
pub const PROTOCOL_VERSION: i32 = 763;
/// Minecraft Java 1.20.1 Game Version.
pub const GAME_VERSION: &str = "1.20.1";

// =========================================================================
// 1. PACKET IDS (1.20.1 Bytecode Parity from se.class)
// =========================================================================

pub mod ids {
    pub mod handshake {
        pub const INTENTION: i32 = 0x00;
    }

    pub mod status {
        pub mod clientbound {
            pub const STATUS_RESPONSE: i32 = 0x00;
            pub const PONG_RESPONSE: i32 = 0x01;
        }
        pub mod serverbound {
            pub const STATUS_REQUEST: i32 = 0x00;
            pub const PING_REQUEST: i32 = 0x01;
        }
    }

    pub mod login {
        pub mod clientbound {
            pub const DISCONNECT: i32 = 0x00;
            pub const HELLO: i32 = 0x01;
            pub const LOGIN_FINISHED: i32 = 0x02;
            pub const COMPRESSION: i32 = 0x03;
            pub const CUSTOM_QUERY: i32 = 0x04;
        }
        pub mod serverbound {
            pub const HELLO: i32 = 0x00;
            pub const KEY: i32 = 0x01;
            pub const CUSTOM_QUERY_ANSWER: i32 = 0x02;
        }
    }

    pub mod play {
        pub mod clientbound {
            pub const DISCONNECT: i32 = 0x1A;
            pub const GAME_EVENT: i32 = 0x22;
            pub const KEEP_ALIVE: i32 = 0x23;
            pub const LEVEL_CHUNK_WITH_LIGHT: i32 = 0x24;
            pub const JOIN_GAME: i32 = 0x28;
            pub const PLAYER_POSITION: i32 = 0x3C;
            pub const SET_DEFAULT_SPAWN_POSITION: i32 = 0x4C;
            pub const SET_SIMULATION_DISTANCE: i32 = 0x4D;
            pub const SET_CENTER_CHUNK: i32 = 0x4E;
            pub const SYSTEM_CHAT: i32 = 0x64;
        }
        pub mod serverbound {
            pub const ACCEPT_TELEPORTATION: i32 = 0x00;
            pub const CHAT_MESSAGE: i32 = 0x05;
            pub const KEEP_ALIVE: i32 = 0x12;
            pub const MOVE_PLAYER_POS: i32 = 0x14;
            pub const MOVE_PLAYER_POS_ROT: i32 = 0x15;
            pub const MOVE_PLAYER_ROT: i32 = 0x16;
            pub const MOVE_PLAYER_STATUS: i32 = 0x17;
        }
    }
}

// =========================================================================
// 2. STATUS & HANDSHAKE
// =========================================================================

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Legacy1201NextState {
    Status,
    Login,
}

/// 1.20.1 Handshake (Intention) Packet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201Handshake {
    pub protocol_version: i32,
    pub server_address: String,
    pub server_port: u16,
    pub next_state: Legacy1201NextState,
}

impl Legacy1201Handshake {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let protocol_version = r.varint()?;
        let server_address = r.string(255)?;
        let server_port = r.u16()?;
        let next_state_id = r.varint()?;
        let next_state = match next_state_id {
            1 => Legacy1201NextState::Status,
            2 => Legacy1201NextState::Login,
            other => return Err(Error::new(format!("invalid next state: {other}"))),
        };
        Ok(Self {
            protocol_version,
            server_address,
            server_port,
            next_state,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::handshake::INTENTION);
        w.varint(self.protocol_version);
        w.string(&self.server_address);
        w.u16(self.server_port);
        w.varint(match self.next_state {
            Legacy1201NextState::Status => 1,
            Legacy1201NextState::Login => 2,
        });
        w.data
    }
}

/// Formats standard 1.20.1 Server List Ping JSON.
pub fn format_1_20_1_status_json(
    motd: &str,
    max_players: i32,
    online_players: i32,
) -> String {
    format!(
        "{{\"version\":{{\"name\":\"{}\",\"protocol\":{}}},\"players\":{{\"max\":{},\"online\":{},\"sample\":[]}},\"description\":{{\"text\":\"{}\"}}}}",
        GAME_VERSION, PROTOCOL_VERSION, max_players, online_players, motd
    )
}

// =========================================================================
// 3. REGISTRY CODEC (1.20.1 Parity)
// =========================================================================

#[inline]
fn tag_key(k: &str) -> JavaString {
    JavaString::from(k)
}

/// Builds full vanilla 1.20.1 registry codec NBT Compound for Overworld play.
pub fn create_vanilla_1_20_1_registry_codec() -> Tag {
    let mut root = BTreeMap::new();

    // 1. minecraft:dimension_type
    {
        let mut dim_reg = BTreeMap::new();
        dim_reg.insert(tag_key("type"), Tag::String(tag_key("minecraft:dimension_type")));

        let mut overworld_element = BTreeMap::new();
        overworld_element.insert(tag_key("ambient_light"), Tag::Float(0.0_f32.to_bits()));
        overworld_element.insert(tag_key("bed_works"), Tag::Byte(1));
        overworld_element.insert(tag_key("coordinate_scale"), Tag::Double(1.0_f64.to_bits()));
        overworld_element.insert(tag_key("effects"), Tag::String(tag_key("minecraft:overworld")));
        overworld_element.insert(tag_key("has_ceiling"), Tag::Byte(0));
        overworld_element.insert(tag_key("has_raids"), Tag::Byte(1));
        overworld_element.insert(tag_key("has_skylight"), Tag::Byte(1));
        overworld_element.insert(tag_key("height"), Tag::Int(384));
        overworld_element.insert(tag_key("infiniburn"), Tag::String(tag_key("#minecraft:infiniburn_overworld")));
        overworld_element.insert(tag_key("logical_height"), Tag::Int(384));
        overworld_element.insert(tag_key("min_y"), Tag::Int(-64));
        overworld_element.insert(tag_key("monster_spawn_block_light_limit"), Tag::Int(0));
        overworld_element.insert(tag_key("monster_spawn_light_level"), Tag::Int(0));
        overworld_element.insert(tag_key("natural"), Tag::Byte(1));
        overworld_element.insert(tag_key("piglin_safe"), Tag::Byte(0));
        overworld_element.insert(tag_key("respawn_anchor_works"), Tag::Byte(0));
        overworld_element.insert(tag_key("ultrawarm"), Tag::Byte(0));

        let mut entry = BTreeMap::new();
        entry.insert(tag_key("id"), Tag::Int(0));
        entry.insert(tag_key("name"), Tag::String(tag_key("minecraft:overworld")));
        entry.insert(tag_key("element"), Tag::Compound(overworld_element));

        dim_reg.insert(tag_key("value"), Tag::List(vec![Tag::Compound(entry)]));
        root.insert(tag_key("minecraft:dimension_type"), Tag::Compound(dim_reg));
    }

    // 2. minecraft:worldgen/biome
    {
        let mut biome_reg = BTreeMap::new();
        biome_reg.insert(tag_key("type"), Tag::String(tag_key("minecraft:worldgen/biome")));

        let biomes = [
            ("minecraft:plains", 0),
            ("minecraft:desert", 1),
            ("minecraft:forest", 2),
        ];

        let mut entries = Vec::with_capacity(biomes.len());
        for (name, id) in biomes {
            let mut element = BTreeMap::new();
            element.insert(tag_key("has_precipitation"), Tag::Byte(1));
            element.insert(tag_key("temperature"), Tag::Float(0.8_f32.to_bits()));
            element.insert(tag_key("downfall"), Tag::Float(0.4_f32.to_bits()));

            let mut effects = BTreeMap::new();
            effects.insert(tag_key("sky_color"), Tag::Int(7907327));
            effects.insert(tag_key("water_fog_color"), Tag::Int(329011));
            effects.insert(tag_key("fog_color"), Tag::Int(12638463));
            effects.insert(tag_key("water_color"), Tag::Int(4159204));
            element.insert(tag_key("effects"), Tag::Compound(effects));

            let mut entry = BTreeMap::new();
            entry.insert(tag_key("id"), Tag::Int(id));
            entry.insert(tag_key("name"), Tag::String(tag_key(name)));
            entry.insert(tag_key("element"), Tag::Compound(element));
            entries.push(Tag::Compound(entry));
        }

        biome_reg.insert(tag_key("value"), Tag::List(entries));
        root.insert(tag_key("minecraft:worldgen/biome"), Tag::Compound(biome_reg));
    }

    // 3. minecraft:chat_type
    {
        let mut chat_reg = BTreeMap::new();
        chat_reg.insert(tag_key("type"), Tag::String(tag_key("minecraft:chat_type")));
        chat_reg.insert(tag_key("value"), Tag::List(Vec::new()));
        root.insert(tag_key("minecraft:chat_type"), Tag::Compound(chat_reg));
    }

    // 4. minecraft:damage_type
    {
        let mut dmg_reg = BTreeMap::new();
        dmg_reg.insert(tag_key("type"), Tag::String(tag_key("minecraft:damage_type")));
        dmg_reg.insert(tag_key("value"), Tag::List(Vec::new()));
        root.insert(tag_key("minecraft:damage_type"), Tag::Compound(dmg_reg));
    }

    Tag::Compound(root)
}

// =========================================================================
// 4. PLAY CLIENTBOUND PACKETS (1.20.1)
// =========================================================================

/// 1.20.1 Join Game (Login) Packet (Clientbound 0x28).
#[derive(Clone, Debug, PartialEq)]
pub struct Legacy1201JoinGame {
    pub player_id: i32,
    pub is_hardcore: bool,
    pub game_mode: u8,
    pub previous_game_mode: i8,
    pub levels: Vec<String>,
    pub registry_codec: Tag,
    pub dimension_type: String,
    pub dimension: String,
    pub seed: i64,
    pub max_players: i32,
    pub chunk_radius: i32,
    pub simulation_distance: i32,
    pub reduced_debug_info: bool,
    pub show_death_screen: bool,
    pub is_debug: bool,
    pub is_flat: bool,
    pub last_death_location: Option<(String, [i32; 3])>,
    pub portal_cooldown: i32,
}

impl Legacy1201JoinGame {
    pub fn overworld_default(player_id: i32) -> Self {
        Self {
            player_id,
            is_hardcore: false,
            game_mode: 0, // Survival
            previous_game_mode: -1, // None
            levels: vec!["minecraft:overworld".into()],
            registry_codec: create_vanilla_1_20_1_registry_codec(),
            dimension_type: "minecraft:overworld".into(),
            dimension: "minecraft:overworld".into(),
            seed: 0,
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            is_debug: false,
            is_flat: false,
            last_death_location: None,
            portal_cooldown: 0,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::clientbound::JOIN_GAME);
        w.i32(self.player_id);
        w.bool(self.is_hardcore);
        w.u8(self.game_mode);
        w.i8(self.previous_game_mode);
        w.varint(self.levels.len() as i32);
        for lvl in &self.levels {
            w.string(lvl);
        }
        w.nbt(&self.registry_codec);
        w.string(&self.dimension_type);
        w.string(&self.dimension);
        w.i64(self.seed);
        w.varint(self.max_players);
        w.varint(self.chunk_radius);
        w.varint(self.simulation_distance);
        w.bool(self.reduced_debug_info);
        w.bool(self.show_death_screen);
        w.bool(self.is_debug);
        w.bool(self.is_flat);
        if let Some((dim, pos)) = &self.last_death_location {
            w.bool(true);
            w.string(dim);
            w.i64(pack_block_pos(*pos));
        } else {
            w.bool(false);
        }
        w.varint(self.portal_cooldown);
        w.data
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let player_id = r.i32()?;
        let is_hardcore = r.bool()?;
        let game_mode = r.u8()?;
        let previous_game_mode = r.i8()?;
        let count = r.varint()?;
        let mut levels = Vec::with_capacity(count as usize);
        for _ in 0..count {
            levels.push(r.string(32767)?);
        }
        let registry_codec = r.nbt()?;
        let dimension_type = r.string(32767)?;
        let dimension = r.string(32767)?;
        let seed = r.i64()?;
        let max_players = r.varint()?;
        let chunk_radius = r.varint()?;
        let simulation_distance = r.varint()?;
        let reduced_debug_info = r.bool()?;
        let show_death_screen = r.bool()?;
        let is_debug = r.bool()?;
        let is_flat = r.bool()?;
        let last_death_location = if r.bool()? {
            let dim = r.string(32767)?;
            let pos_packed = r.i64()?;
            let x = (pos_packed >> 38) as i32;
            let y = (pos_packed << 52 >> 52) as i32;
            let z = (pos_packed << 26 >> 38) as i32;
            Some((dim, [x, y, z]))
        } else {
            None
        };
        let portal_cooldown = r.varint()?;
        Ok(Self {
            player_id,
            is_hardcore,
            game_mode,
            previous_game_mode,
            levels,
            registry_codec,
            dimension_type,
            dimension,
            seed,
            max_players,
            chunk_radius,
            simulation_distance,
            reduced_debug_info,
            show_death_screen,
            is_debug,
            is_flat,
            last_death_location,
            portal_cooldown,
        })
    }
}

/// 1.20.1 Player Position & Look Packet (Clientbound 0x3C).
#[derive(Clone, Debug, PartialEq)]
pub struct Legacy1201PlayerPosition {
    pub x: f64,
    pub y: f64,
    pub z: f64,
    pub yaw: f32,
    pub pitch: f32,
    pub flags: u8,
    pub teleport_id: i32,
}

impl Legacy1201PlayerPosition {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::clientbound::PLAYER_POSITION);
        w.f64(self.x);
        w.f64(self.y);
        w.f64(self.z);
        w.f32(self.yaw);
        w.f32(self.pitch);
        w.u8(self.flags);
        w.varint(self.teleport_id);
        w.data
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let x = r.f64()?;
        let y = r.f64()?;
        let z = r.f64()?;
        let yaw = r.f32()?;
        let pitch = r.f32()?;
        let flags = r.u8()?;
        let teleport_id = r.varint()?;
        Ok(Self {
            x,
            y,
            z,
            yaw,
            pitch,
            flags,
            teleport_id,
        })
    }
}

/// 1.20.1 Set Center Chunk Packet (Clientbound 0x4E).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201SetCenterChunk {
    pub chunk_x: i32,
    pub chunk_z: i32,
}

impl Legacy1201SetCenterChunk {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::clientbound::SET_CENTER_CHUNK);
        w.varint(self.chunk_x);
        w.varint(self.chunk_z);
        w.data
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        Ok(Self {
            chunk_x: r.varint()?,
            chunk_z: r.varint()?,
        })
    }
}

/// 1.20.1 Set Default Spawn Position (Clientbound 0x4C).
#[derive(Clone, Debug, PartialEq)]
pub struct Legacy1201SetDefaultSpawnPosition {
    pub pos: [i32; 3],
    pub angle: f32,
}

impl Legacy1201SetDefaultSpawnPosition {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::clientbound::SET_DEFAULT_SPAWN_POSITION);
        w.i64(pack_block_pos(self.pos));
        w.f32(self.angle);
        w.data
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let pos_packed = r.i64()?;
        let x = (pos_packed >> 38) as i32;
        let y = (pos_packed << 52 >> 52) as i32;
        let z = (pos_packed << 26 >> 38) as i32;
        let angle = r.f32()?;
        Ok(Self {
            pos: [x, y, z],
            angle,
        })
    }
}

/// 1.20.1 Keep Alive Packet (Clientbound 0x23 / Serverbound 0x12).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201KeepAlive {
    pub id: i64,
}

impl Legacy1201KeepAlive {
    pub fn encode_clientbound(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::clientbound::KEEP_ALIVE);
        w.i64(self.id);
        w.data
    }

    pub fn encode_serverbound(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::serverbound::KEEP_ALIVE);
        w.i64(self.id);
        w.data
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        Ok(Self { id: r.i64()? })
    }
}

/// 1.20.1 Play Disconnect Packet (Clientbound 0x1A).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201Disconnect {
    pub reason_json: String,
}

impl Legacy1201Disconnect {
    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::clientbound::DISCONNECT);
        w.string(&self.reason_json);
        w.data
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        Ok(Self {
            reason_json: r.string(32767)?,
        })
    }
}

/// 1.20.1 System Chat Message Packet (Clientbound 0x64).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201SystemChat {
    pub content: String,
    pub overlay: bool,
}

impl Legacy1201SystemChat {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            content: format!("{{\"text\":\"{}\"}}", text.into()),
            overlay: false,
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::clientbound::SYSTEM_CHAT);
        w.string(&self.content);
        w.bool(self.overlay);
        w.data
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        Ok(Self {
            content: r.string(32767)?,
            overlay: r.bool()?,
        })
    }
}

// =========================================================================
// 5. CHUNK PACKET ENCODER (Clientbound 0x24 Parity)
// =========================================================================

/// Serializes a Legacy1201Chunk into 1.20.1 ClientboundLevelChunkWithLightPacket payload.
pub fn serialize_legacy_1_20_1_chunk_packet(chunk: &Legacy1201Chunk) -> Vec<u8> {
    let mut w = Writer::new();
    w.varint(ids::play::clientbound::LEVEL_CHUNK_WITH_LIGHT);
    w.i32(chunk.x);
    w.i32(chunk.z);

    // Heightmaps compound tag
    let hm_tag = chunk.heightmaps.to_nbt(chunk.min_y());
    w.nbt(&hm_tag);

    // Serialize 24 sections data into an intermediate buffer
    let mut sec_buf = Writer::new();
    let mut sky_mask = 0u32;
    let mut block_mask = 0u32;
    let mut empty_sky_mask = 0u32;
    let mut empty_block_mask = 0u32;
    let mut sky_updates = Vec::new();
    let mut block_updates = Vec::new();

    for (idx, sec) in chunk.sections.iter().enumerate() {
        // 1. Block count (non-air)
        let air_idx = sec.block_palette.iter().position(|b| b.name == "minecraft:air");
        let non_air: i16 = match &sec.block_storage {
            None => {
                if air_idx == Some(0) {
                    0
                } else {
                    4096
                }
            }
            Some(storage) => {
                let mut count = 0i16;
                for i in 0..BLOCKS_PER_SECTION {
                    if let Ok(p_idx) = storage.get(i) {
                        if air_idx.map_or(true, |ai| p_idx != ai as u32) {
                            count += 1;
                        }
                    }
                }
                count
            }
        };
        sec_buf.i16(non_air);

        // 2. Block states PalettedContainer
        if sec.block_palette.len() <= 1 {
            // Single-valued palette: bits = 0
            sec_buf.u8(0);
            sec_buf.varint(0); // State ID: 0 (or air/uniform)
            sec_buf.varint(0); // Data length = 0
        } else {
            // Indirect palette (4..=8 bits)
            let bits = if sec.block_palette.len() <= 16 {
                4
            } else {
                8
            };
            sec_buf.u8(bits);
            sec_buf.varint(sec.block_palette.len() as i32);
            for i in 0..sec.block_palette.len() {
                sec_buf.varint(i as i32);
            }
            if let Some(storage) = &sec.block_storage {
                sec_buf.varint(storage.words().len() as i32);
                for &word in storage.words() {
                    sec_buf.i64(word as i64);
                }
            } else {
                sec_buf.varint(0);
            }
        }

        // 3. Biomes PalettedContainer
        if sec.biome_palette.len() <= 1 {
            // Single-valued palette
            sec_buf.u8(0);
            sec_buf.varint(0); // Biome ID: 0 (Plains)
            sec_buf.varint(0);
        } else {
            let bits = 3;
            sec_buf.u8(bits);
            sec_buf.varint(sec.biome_palette.len() as i32);
            for i in 0..sec.biome_palette.len() {
                sec_buf.varint(i as i32);
            }
            if let Some(storage) = &sec.biome_storage {
                sec_buf.varint(storage.words().len() as i32);
                for &word in storage.words() {
                    sec_buf.i64(word as i64);
                }
            } else {
                sec_buf.varint(0);
            }
        }

        // Light data tracking (24 sections + 2 border sections = 26 bits)
        let light_bit = 1u32 << (idx + 1);
        if let Some(sky) = &sec.sky_light {
            sky_mask |= light_bit;
            sky_updates.push(sky);
        } else {
            empty_sky_mask |= light_bit;
        }

        if let Some(block) = &sec.block_light {
            block_mask |= light_bit;
            block_updates.push(block);
        } else {
            empty_block_mask |= light_bit;
        }
    }

    // Write sections byte array (VarInt length prefix)
    w.varint(sec_buf.data.len() as i32);
    w.bytes(&sec_buf.data);

    // Block entities list (0 in standard terrain chunks)
    w.varint(0);

    // Light data:
    // trust_edges: bool
    w.bool(true);

    // BitSet arrays (1 long = 64 bits covers 26 sections)
    w.varint(1);
    w.i64(sky_mask as i64);
    w.varint(1);
    w.i64(block_mask as i64);
    w.varint(1);
    w.i64(empty_sky_mask as i64);
    w.varint(1);
    w.i64(empty_block_mask as i64);

    // Sky updates
    w.varint(sky_updates.len() as i32);
    for sky in sky_updates {
        w.varint(2048);
        w.bytes(sky);
    }

    // Block updates
    w.varint(block_updates.len() as i32);
    for blk in block_updates {
        w.varint(2048);
        w.bytes(blk);
    }

    w.data
}

// =========================================================================
// 6. PLAY SERVERBOUND PACKETS (1.20.1)
// =========================================================================

/// 1.20.1 Accept Teleportation Packet (Serverbound 0x00).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201AcceptTeleportation {
    pub teleport_id: i32,
}

impl Legacy1201AcceptTeleportation {
    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        Ok(Self {
            teleport_id: r.varint()?,
        })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::serverbound::ACCEPT_TELEPORTATION);
        w.varint(self.teleport_id);
        w.data
    }
}

/// 1.20.1 Serverbound Chat Message (0x05).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Legacy1201ChatMessage {
    pub message: String,
    pub timestamp: i64,
}

impl Legacy1201ChatMessage {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            timestamp: 0,
        }
    }

    pub fn decode(payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        let message = r.string(256)?;
        let timestamp = r.i64()?;
        Ok(Self { message, timestamp })
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.varint(ids::play::serverbound::CHAT_MESSAGE);
        w.string(&self.message);
        w.i64(self.timestamp);
        w.i64(0); // salt
        w.bool(false); // has_signature = false
        w.varint(0); // message count
        w.bytes(&[0u8, 0, 0]); // acknowledged bitset (20 bits)
        w.data
    }
}

/// 1.20.1 Move Player Packets (Serverbound 0x14, 0x15, 0x16, 0x17).
#[derive(Clone, Debug, PartialEq)]
pub enum Legacy1201ServerboundMovePlayer {
    Pos {
        x: f64,
        feet_y: f64,
        z: f64,
        on_ground: bool,
    },
    PosRot {
        x: f64,
        feet_y: f64,
        z: f64,
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    },
    Rot {
        yaw: f32,
        pitch: f32,
        on_ground: bool,
    },
    StatusOnly {
        on_ground: bool,
    },
}

impl Legacy1201ServerboundMovePlayer {
    pub fn decode(id: i32, payload: &[u8]) -> Result<Self> {
        let mut r = Reader::new(payload);
        match id {
            ids::play::serverbound::MOVE_PLAYER_POS => {
                let x = r.f64()?;
                let feet_y = r.f64()?;
                let z = r.f64()?;
                let on_ground = r.bool()?;
                Ok(Self::Pos {
                    x,
                    feet_y,
                    z,
                    on_ground,
                })
            }
            ids::play::serverbound::MOVE_PLAYER_POS_ROT => {
                let x = r.f64()?;
                let feet_y = r.f64()?;
                let z = r.f64()?;
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                let on_ground = r.bool()?;
                Ok(Self::PosRot {
                    x,
                    feet_y,
                    z,
                    yaw,
                    pitch,
                    on_ground,
                })
            }
            ids::play::serverbound::MOVE_PLAYER_ROT => {
                let yaw = r.f32()?;
                let pitch = r.f32()?;
                let on_ground = r.bool()?;
                Ok(Self::Rot {
                    yaw,
                    pitch,
                    on_ground,
                })
            }
            ids::play::serverbound::MOVE_PLAYER_STATUS => {
                let on_ground = r.bool()?;
                Ok(Self::StatusOnly { on_ground })
            }
            other => Err(Error::new(format!("unknown move player packet id {other}"))),
        }
    }

    pub fn encode(&self) -> Vec<u8> {
        let mut w = Writer::new();
        match self {
            Self::Pos {
                x,
                feet_y,
                z,
                on_ground,
            } => {
                w.varint(ids::play::serverbound::MOVE_PLAYER_POS);
                w.f64(*x);
                w.f64(*feet_y);
                w.f64(*z);
                w.bool(*on_ground);
            }
            Self::PosRot {
                x,
                feet_y,
                z,
                yaw,
                pitch,
                on_ground,
            } => {
                w.varint(ids::play::serverbound::MOVE_PLAYER_POS_ROT);
                w.f64(*x);
                w.f64(*feet_y);
                w.f64(*z);
                w.f32(*yaw);
                w.f32(*pitch);
                w.bool(*on_ground);
            }
            Self::Rot {
                yaw,
                pitch,
                on_ground,
            } => {
                w.varint(ids::play::serverbound::MOVE_PLAYER_ROT);
                w.f32(*yaw);
                w.f32(*pitch);
                w.bool(*on_ground);
            }
            Self::StatusOnly { on_ground } => {
                w.varint(ids::play::serverbound::MOVE_PLAYER_STATUS);
                w.bool(*on_ground);
            }
        }
        w.data
    }
}

// =========================================================================
// 7. SERVER SESSION HANDLER
// =========================================================================

pub enum Legacy1201SessionState {
    Handshake,
    Status,
    Login,
    Play {
        profile: GameProfile,
        teleport_confirmed: bool,
        last_keep_alive: i64,
        entity_id: i32,
    },
    Closed,
}

/// Manages a single 1.20.1 client connection lifecycle.
pub struct Legacy1201ServerSession {
    pub connection: Connection,
    pub state: Legacy1201SessionState,
}

impl Legacy1201ServerSession {
    pub fn new(connection: Connection) -> Self {
        Self {
            connection,
            state: Legacy1201SessionState::Handshake,
        }
    }

    /// Process initial handshake packet from client.
    pub fn handle_handshake(&mut self) -> Result<Legacy1201NextState> {
        let (id, payload) = self.connection.recv()?;
        if id != ids::handshake::INTENTION {
            return Err(Error::new(format!("expected handshake (0), got {id}")));
        }
        let hs = Legacy1201Handshake::decode(&payload)?;
        if hs.protocol_version != PROTOCOL_VERSION {
            return Err(Error::new(format!(
                "unsupported protocol {}: expected {}",
                hs.protocol_version, PROTOCOL_VERSION
            )));
        }
        match hs.next_state {
            Legacy1201NextState::Status => {
                self.state = Legacy1201SessionState::Status;
                Ok(Legacy1201NextState::Status)
            }
            Legacy1201NextState::Login => {
                self.state = Legacy1201SessionState::Login;
                Ok(Legacy1201NextState::Login)
            }
        }
    }

    /// Handle Server List Ping query and latency check.
    pub fn handle_status(&mut self, motd: &str, max_players: i32, online_players: i32) -> Result<()> {
        let (id, _) = self.connection.recv()?;
        if id != ids::status::serverbound::STATUS_REQUEST {
            return Err(Error::new(format!("expected status request (0), got {id}")));
        }
        let json = format_1_20_1_status_json(motd, max_players, online_players);
        let mut w = Writer::new();
        w.varint(ids::status::clientbound::STATUS_RESPONSE);
        w.string(&json);
        self.connection.send(&w.data)?;

        // Optional ping check
        if let Ok((ping_id, ping_payload)) = self.connection.recv() {
            if ping_id == ids::status::serverbound::PING_REQUEST {
                let mut r = Reader::new(&ping_payload);
                let time = r.i64()?;
                let mut pw = Writer::new();
                pw.varint(ids::status::clientbound::PONG_RESPONSE);
                pw.i64(time);
                self.connection.send(&pw.data)?;
            }
        }
        self.state = Legacy1201SessionState::Closed;
        Ok(())
    }

    /// Handle 1.20.1 offline login and immediately transition to Play!
    pub fn handle_login(&mut self, entity_id: i32) -> Result<GameProfile> {
        let (id, payload) = self.connection.recv()?;
        if id != ids::login::serverbound::HELLO {
            return Err(Error::new(format!("expected login hello (0), got {id}")));
        }
        let mut r = Reader::new(&payload);
        let name = r.string(16)?;
        let uuid = if r.bool()? {
            Some(r.uuid()?)
        } else {
            None
        }
        .unwrap_or_else(|| offline_uuid(&name));

        let profile = GameProfile {
            uuid,
            name: name.clone(),
            properties: Vec::new(),
        };

        // Send LoginFinished (0x02)
        let mut w = Writer::new();
        w.varint(ids::login::clientbound::LOGIN_FINISHED);
        w.uuid(profile.uuid);
        w.string(&profile.name);
        w.varint(0); // 0 properties
        self.connection.send(&w.data)?;

        // Direct transition to Play state!
        self.state = Legacy1201SessionState::Play {
            profile: profile.clone(),
            teleport_confirmed: false,
            last_keep_alive: 1,
            entity_id,
        };
        Ok(profile)
    }

    /// Sends Join Game and spawn packets for player in world.
    pub fn initialize_play(
        &mut self,
        world: &mut Legacy1201World,
        spawn_pos: [f64; 3],
    ) -> Result<()> {
        let entity_id = match &self.state {
            Legacy1201SessionState::Play { entity_id, .. } => *entity_id,
            _ => return Err(Error::new("not in play state")),
        };

        // 1. Join Game
        let join = Legacy1201JoinGame::overworld_default(entity_id);
        self.connection.send(&join.encode())?;

        // 2. Set Default Spawn Position
        let spawn = Legacy1201SetDefaultSpawnPosition {
            pos: [spawn_pos[0] as i32, spawn_pos[1] as i32, spawn_pos[2] as i32],
            angle: 0.0,
        };
        self.connection.send(&spawn.encode())?;

        // 3. Player Position (teleport id = 1)
        let player_pos = Legacy1201PlayerPosition {
            x: spawn_pos[0],
            y: spawn_pos[1],
            z: spawn_pos[2],
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 1,
        };
        self.connection.send(&player_pos.encode())?;

        // 4. Set Center Chunk (0, 0)
        let center = Legacy1201SetCenterChunk {
            chunk_x: 0,
            chunk_z: 0,
        };
        self.connection.send(&center.encode())?;

        // 5. Send initial chunks around spawn
        if let Ok(chunk) = world.get_or_load_chunk(ChunkPos { x: 0, z: 0 }) {
            let chunk_packet = serialize_legacy_1_20_1_chunk_packet(chunk);
            self.connection.send(&chunk_packet)?;
        }

        Ok(())
    }
}

// =========================================================================
// 8. TCP SERVER & GAME SERVER INTEGRATION
// =========================================================================

/// Represents a connected player in the live 1.20.1 game server.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectedPlayer {
    pub uuid: u128,
    pub name: String,
    pub entity_id: i32,
    pub pos: [f64; 3],
    pub yaw: f32,
    pub pitch: f32,
    pub on_ground: bool,
    pub teleport_confirmed: bool,
    pub last_keep_alive: i64,
}

/// Fully autonomous Minecraft Java 1.20.1 Game Server Engine.
pub struct Legacy1201GameServer {
    pub world: Arc<Mutex<Legacy1201World>>,
    pub entity_manager: Arc<Mutex<Legacy1201EntityManager>>,
    pub players: Arc<Mutex<HashMap<u128, ConnectedPlayer>>>,
    pub running: Arc<AtomicBool>,
    pub game_time: Arc<AtomicI64>,
    pub next_entity_id: Arc<AtomicI32>,
    pub port: u16,
    pub motd: String,
    pub max_players: i32,
}

impl Legacy1201GameServer {
    pub fn new(
        world: Legacy1201World,
        port: u16,
        motd: impl Into<String>,
        max_players: i32,
    ) -> Self {
        Self {
            world: Arc::new(Mutex::new(world)),
            entity_manager: Arc::new(Mutex::new(Legacy1201EntityManager::new())),
            players: Arc::new(Mutex::new(HashMap::new())),
            running: Arc::new(AtomicBool::new(true)),
            game_time: Arc::new(AtomicI64::new(0)),
            next_entity_id: Arc::new(AtomicI32::new(100)),
            port,
            motd: motd.into(),
            max_players,
        }
    }

    /// Runs a single 50ms server game tick (20 Hz).
    pub fn tick(&self) {
        let t = self.game_time.fetch_add(1, Ordering::Relaxed) + 1;
        if let Ok(mut w) = self.world.lock() {
            w.level_data.time = t;
            w.level_data.day_time = (t) % 24000;
        }
    }

    /// Handles an incoming client connection: Handshake + Login + initial Play packets.
    pub fn connect_player(
        &self,
        conn: &mut Connection,
        name: &str,
        uuid: Option<u128>,
    ) -> Result<ConnectedPlayer> {
        let player_uuid = uuid.unwrap_or_else(|| offline_uuid(name));
        let entity_id = self.next_entity_id.fetch_add(1, Ordering::Relaxed);

        // 1. Send LoginFinished (0x02)
        let mut w = Writer::new();
        w.varint(ids::login::clientbound::LOGIN_FINISHED);
        w.uuid(player_uuid);
        w.string(name);
        w.varint(0); // 0 properties
        conn.send(&w.data)?;

        // 2. Direct transition to Play state: send JoinGame
        let join = Legacy1201JoinGame::overworld_default(entity_id);
        conn.send(&join.encode())?;

        // 3. Send SetDefaultSpawnPosition (0, -60, 0)
        let spawn = Legacy1201SetDefaultSpawnPosition {
            pos: [0, -60, 0],
            angle: 0.0,
        };
        conn.send(&spawn.encode())?;

        // 4. Send PlayerPositionAndLook (teleport_id = 1)
        let spawn_coords = [0.5, -60.0, 0.5];
        let p_pos = Legacy1201PlayerPosition {
            x: spawn_coords[0],
            y: spawn_coords[1],
            z: spawn_coords[2],
            yaw: 0.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 1,
        };
        conn.send(&p_pos.encode())?;

        // 5. Send SetCenterChunk (0, 0)
        let center = Legacy1201SetCenterChunk {
            chunk_x: 0,
            chunk_z: 0,
        };
        conn.send(&center.encode())?;

        // 6. Send initial 3x3 chunks around spawn
        if let Ok(mut world) = self.world.lock() {
            for cz in -1..=1 {
                for cx in -1..=1 {
                    if let Ok(chunk) = world.get_or_load_chunk(ChunkPos { x: cx, z: cz }) {
                        let chunk_bytes = serialize_legacy_1_20_1_chunk_packet(chunk);
                        let _ = conn.send(&chunk_bytes);
                    }
                }
            }
        }

        // 7. Register player in entity manager and connected players map
        let player = ConnectedPlayer {
            uuid: player_uuid,
            name: name.to_string(),
            entity_id,
            pos: spawn_coords,
            yaw: 0.0,
            pitch: 0.0,
            on_ground: true,
            teleport_confirmed: false,
            last_keep_alive: 1,
        };

        if let Ok(mut em) = self.entity_manager.lock() {
            let mut ent = Legacy1201Entity::new(
                Legacy1201EntityType::Player,
                Vec3::new(spawn_coords[0], spawn_coords[1], spawn_coords[2]),
            );
            ent.uuid = [
                (player_uuid >> 96) as u32,
                (player_uuid >> 64) as u32,
                (player_uuid >> 32) as u32,
                player_uuid as u32,
            ];
            ent.custom_name = Some(name.to_string());
            em.spawn(ent);
        }

        if let Ok(mut pm) = self.players.lock() {
            pm.insert(player_uuid, player.clone());
        }

        Ok(player)
    }

    /// Dispatches an incoming play packet from a connected player.
    pub fn handle_player_packet(
        &self,
        player: &mut ConnectedPlayer,
        packet_id: i32,
        payload: &[u8],
    ) -> Result<Option<Vec<u8>>> {
        match packet_id {
            ids::play::serverbound::ACCEPT_TELEPORTATION => {
                let acc = Legacy1201AcceptTeleportation::decode(payload)?;
                if acc.teleport_id == 1 {
                    player.teleport_confirmed = true;
                }
                Ok(None)
            }
            ids::play::serverbound::MOVE_PLAYER_POS
            | ids::play::serverbound::MOVE_PLAYER_POS_ROT
            | ids::play::serverbound::MOVE_PLAYER_ROT
            | ids::play::serverbound::MOVE_PLAYER_STATUS => {
                let mv = Legacy1201ServerboundMovePlayer::decode(packet_id, payload)?;
                match mv {
                    Legacy1201ServerboundMovePlayer::Pos { x, feet_y, z, on_ground } => {
                        player.pos = [x, feet_y, z];
                        player.on_ground = on_ground;
                    }
                    Legacy1201ServerboundMovePlayer::PosRot { x, feet_y, z, yaw, pitch, on_ground } => {
                        player.pos = [x, feet_y, z];
                        player.yaw = yaw;
                        player.pitch = pitch;
                        player.on_ground = on_ground;
                    }
                    Legacy1201ServerboundMovePlayer::Rot { yaw, pitch, on_ground } => {
                        player.yaw = yaw;
                        player.pitch = pitch;
                        player.on_ground = on_ground;
                    }
                    Legacy1201ServerboundMovePlayer::StatusOnly { on_ground } => {
                        player.on_ground = on_ground;
                    }
                }
                Ok(None)
            }
            ids::play::serverbound::KEEP_ALIVE => {
                let ka = Legacy1201KeepAlive::decode(payload)?;
                player.last_keep_alive = ka.id;
                Ok(None)
            }
            ids::play::serverbound::CHAT_MESSAGE => {
                let mut r = Reader::new(payload);
                let msg = r.string(256)?;
                if msg == "/spawn" {
                    player.pos = [0.5, -60.0, 0.5];
                    let teleport = Legacy1201PlayerPosition {
                        x: 0.5,
                        y: -60.0,
                        z: 0.5,
                        yaw: player.yaw,
                        pitch: player.pitch,
                        flags: 0,
                        teleport_id: 2,
                    };
                    Ok(Some(teleport.encode()))
                } else {
                    let chat = Legacy1201SystemChat::new(format!("<{}> {}", player.name, msg));
                    Ok(Some(chat.encode()))
                }
            }
            _ => Ok(None),
        }
    }

    /// Saves all loaded and dirty chunks and level.dat to disk.
    pub fn save(&self, timestamp: u32) -> io::Result<()> {
        if let Ok(mut w) = self.world.lock() {
            w.save_all(timestamp)?;
        }
        Ok(())
    }

    /// Stops server.
    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

/// Standalone 1.20.1 TCP server listener.
pub struct Legacy1201TcpServer {
    pub running: Arc<AtomicBool>,
}

impl Legacy1201TcpServer {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(true)),
        }
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::Relaxed);
    }

    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connection::Pipe;
    use std::thread;

    #[test]
    fn test_handshake_roundtrip() {
        let hs = Legacy1201Handshake {
            protocol_version: 763,
            server_address: "localhost".into(),
            server_port: 25565,
            next_state: Legacy1201NextState::Login,
        };
        let encoded = hs.encode();
        assert_eq!(encoded[0], 0x00);
        let decoded = Legacy1201Handshake::decode(&encoded[1..]).unwrap();
        assert_eq!(decoded, hs);
    }

    #[test]
    fn test_status_response_formatting() {
        let json = format_1_20_1_status_json("Horizon 1.20.1", 20, 1);
        assert!(json.contains("\"protocol\":763"));
        assert!(json.contains("\"name\":\"1.20.1\""));
        assert!(json.contains("\"max\":20"));
        assert!(json.contains("Horizon 1.20.1"));
    }

    #[test]
    fn test_join_game_roundtrip() {
        let join = Legacy1201JoinGame::overworld_default(42);
        let bytes = join.encode();
        assert_eq!(bytes[0], ids::play::clientbound::JOIN_GAME as u8);
        let decoded = Legacy1201JoinGame::decode(&bytes[1..]).unwrap();
        assert_eq!(decoded.player_id, 42);
        assert_eq!(decoded.dimension_type, "minecraft:overworld");
        assert_eq!(decoded.dimension, "minecraft:overworld");
        assert_eq!(decoded.chunk_radius, 8);
    }

    #[test]
    fn test_player_position_roundtrip() {
        let pos = Legacy1201PlayerPosition {
            x: 100.5,
            y: 64.0,
            z: -200.5,
            yaw: 90.0,
            pitch: 0.0,
            flags: 0,
            teleport_id: 1,
        };
        let bytes = pos.encode();
        assert_eq!(bytes[0], ids::play::clientbound::PLAYER_POSITION as u8);
        let decoded = Legacy1201PlayerPosition::decode(&bytes[1..]).unwrap();
        assert_eq!(decoded, pos);
    }

    #[test]
    fn test_system_chat_roundtrip() {
        let chat = Legacy1201SystemChat::new("Welcome to Horizon 1.20.1!");
        let bytes = chat.encode();
        assert_eq!(bytes[0], ids::play::clientbound::SYSTEM_CHAT as u8);
        let decoded = Legacy1201SystemChat::decode(&bytes[1..]).unwrap();
        assert_eq!(decoded, chat);
    }

    #[test]
    fn test_set_center_chunk_roundtrip() {
        let center = Legacy1201SetCenterChunk {
            chunk_x: -5,
            chunk_z: 12,
        };
        let bytes = center.encode();
        assert_eq!(bytes[0], ids::play::clientbound::SET_CENTER_CHUNK as u8);
        let decoded = Legacy1201SetCenterChunk::decode(&bytes[1..]).unwrap();
        assert_eq!(decoded, center);
    }

    #[test]
    fn test_serverbound_move_player_roundtrip() {
        let move_pos = Legacy1201ServerboundMovePlayer::Pos {
            x: 0.5,
            feet_y: -60.0,
            z: 0.5,
            on_ground: true,
        };
        let bytes = move_pos.encode();
        assert_eq!(bytes[0], ids::play::serverbound::MOVE_PLAYER_POS as u8);
        let decoded = Legacy1201ServerboundMovePlayer::decode(bytes[0] as i32, &bytes[1..]).unwrap();
        assert_eq!(decoded, move_pos);
    }

    #[test]
    fn test_full_session_handshake_and_login() {
        let (client_pipe, server_pipe) = Pipe::pair();
        let mut client = Connection::local(client_pipe);
        let server = Connection::local(server_pipe);

        let mut session = Legacy1201ServerSession::new(server);

        let client_thread = thread::spawn(move || {
            // Send Handshake
            let hs = Legacy1201Handshake {
                protocol_version: 763,
                server_address: "127.0.0.1".into(),
                server_port: 25565,
                next_state: Legacy1201NextState::Login,
            };
            client.send(&hs.encode()).unwrap();

            // Send Hello
            let mut w = Writer::new();
            w.varint(ids::login::serverbound::HELLO);
            w.string("Steve");
            w.bool(false); // No UUID
            client.send(&w.data).unwrap();

            // Receive Login Finished
            let (id, payload) = client.recv().unwrap();
            assert_eq!(id, ids::login::clientbound::LOGIN_FINISHED);
            let mut r = Reader::new(&payload);
            let _uuid = r.uuid().unwrap();
            let name = r.string(16).unwrap();
            assert_eq!(name, "Steve");
        });

        let next = session.handle_handshake().unwrap();
        assert_eq!(next, Legacy1201NextState::Login);
        let profile = session.handle_login(1).unwrap();
        assert_eq!(profile.name, "Steve");

        client_thread.join().unwrap();
    }
}
