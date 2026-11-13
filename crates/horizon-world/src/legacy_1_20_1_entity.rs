//! Minecraft Java 1.20.1 Entity Types, NBT Serialization, Physics, and Entity Manager.
//!
//! Provides DataVersion 3465 entity structures for Minecraft Java 1.20.1:
//! - Complete registry of all 124 entity types from Java 1.20.1 bytecode
//! - Exact dimensions (width, height), categories, step heights, and attributes
//! - Full NBT serialization and deserialization (Pos, Motion, Rotation, Health, Air, Fire, UUID)
//! - Simulation physics: gravity (0.08 / 0.04), friction (air 0.91, ground 0.6 / ice 0.98),
//!   water drag (0.8), drowning, fire, step-up (0.6 / 1.5) and AABB collision resolution
//! - Spatial entity manager indexed by ChunkPos with AABB broadphase queries

use crate::{
    compression::invalid,
    legacy_1_20_1_anvil::Legacy1201BlockState,
    nbt::{Compound, JavaString, Tag},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::{BlockPos, ChunkPos},
};
use std::{
    collections::HashMap,
    io,
};

pub const LEGACY_1_20_1_DATA_VERSION: i32 = 3465;
pub const LIVING_GRAVITY: f64 = 0.08;
pub const ITEM_GRAVITY: f64 = 0.04;
pub const AIR_DRAG: f64 = 0.91;
pub const WATER_DRAG: f64 = 0.80;
pub const LAVA_DRAG: f64 = 0.50;
pub const DEFAULT_GROUND_FRICTION: f64 = 0.60;
pub const ICE_GROUND_FRICTION: f64 = 0.98;
pub const BLUE_ICE_GROUND_FRICTION: f64 = 0.989;
pub const SLIME_GROUND_FRICTION: f64 = 0.80;
pub const MAX_AIR_TICKS: i16 = 300;

#[inline]
fn key(val: &str) -> JavaString {
    JavaString::from(val)
}

/// Category of mob in Java 1.20.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Legacy1201MobCategory {
    Monster,
    Creature,
    Ambient,
    Axolotls,
    UndergroundWaterCreature,
    WaterCreature,
    WaterAmbient,
    Misc,
}

/// The 124 entity types of Minecraft Java 1.20.1.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Legacy1201EntityType {
    Allay,
    AreaEffectCloud,
    ArmorStand,
    Arrow,
    Axolotl,
    Bat,
    Bee,
    Blaze,
    BlockDisplay,
    Boat,
    Camel,
    Cat,
    CaveSpider,
    ChestBoat,
    ChestMinecart,
    Chicken,
    Cod,
    CommandBlockMinecart,
    Cow,
    Creeper,
    Dolphin,
    Donkey,
    DragonFireball,
    Drowned,
    Egg,
    ElderGuardian,
    EndCrystal,
    EnderDragon,
    EnderPearl,
    Enderman,
    Endermite,
    Evoker,
    EvokerFangs,
    ExperienceBottle,
    ExperienceOrb,
    EyeOfEnder,
    FallingBlock,
    Fireball,
    FireworkRocket,
    Fox,
    Frog,
    FurnaceMinecart,
    Ghast,
    Giant,
    GlowItemFrame,
    GlowSquid,
    Goat,
    Guardian,
    Hoglin,
    HopperMinecart,
    Horse,
    Husk,
    Illusioner,
    Interaction,
    IronGolem,
    Item,
    ItemDisplay,
    ItemFrame,
    LeashKnot,
    LightningBolt,
    Llama,
    LlamaSpit,
    MagmaCube,
    Marker,
    Minecart,
    Mooshroom,
    Mule,
    Ocelot,
    Painting,
    Panda,
    Parrot,
    Phantom,
    Pig,
    Piglin,
    PiglinBrute,
    Pillager,
    Player,
    PolarBear,
    Potion,
    Pufferfish,
    Rabbit,
    Ravager,
    Salmon,
    Sheep,
    Shulker,
    ShulkerBullet,
    Silverfish,
    Skeleton,
    SkeletonHorse,
    Slime,
    SmallFireball,
    Sniffer,
    SnowGolem,
    Snowball,
    SpawnerMinecart,
    SpectralArrow,
    Spider,
    Squid,
    Stray,
    Strider,
    Tadpole,
    TextDisplay,
    Tnt,
    TntMinecart,
    TraderLlama,
    Trident,
    TropicalFish,
    Turtle,
    Vex,
    Villager,
    Vindicator,
    WanderingTrader,
    Warden,
    Witch,
    Wither,
    WitherSkeleton,
    WitherSkull,
    Wolf,
    Zoglin,
    Zombie,
    ZombieHorse,
    ZombieVillager,
    ZombifiedPiglin,
}

impl Legacy1201EntityType {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Allay => "minecraft:allay",
            Self::AreaEffectCloud => "minecraft:area_effect_cloud",
            Self::ArmorStand => "minecraft:armor_stand",
            Self::Arrow => "minecraft:arrow",
            Self::Axolotl => "minecraft:axolotl",
            Self::Bat => "minecraft:bat",
            Self::Bee => "minecraft:bee",
            Self::Blaze => "minecraft:blaze",
            Self::BlockDisplay => "minecraft:block_display",
            Self::Boat => "minecraft:boat",
            Self::Camel => "minecraft:camel",
            Self::Cat => "minecraft:cat",
            Self::CaveSpider => "minecraft:cave_spider",
            Self::ChestBoat => "minecraft:chest_boat",
            Self::ChestMinecart => "minecraft:chest_minecart",
            Self::Chicken => "minecraft:chicken",
            Self::Cod => "minecraft:cod",
            Self::CommandBlockMinecart => "minecraft:command_block_minecart",
            Self::Cow => "minecraft:cow",
            Self::Creeper => "minecraft:creeper",
            Self::Dolphin => "minecraft:dolphin",
            Self::Donkey => "minecraft:donkey",
            Self::DragonFireball => "minecraft:dragon_fireball",
            Self::Drowned => "minecraft:drowned",
            Self::Egg => "minecraft:egg",
            Self::ElderGuardian => "minecraft:elder_guardian",
            Self::EndCrystal => "minecraft:end_crystal",
            Self::EnderDragon => "minecraft:ender_dragon",
            Self::EnderPearl => "minecraft:ender_pearl",
            Self::Enderman => "minecraft:enderman",
            Self::Endermite => "minecraft:endermite",
            Self::Evoker => "minecraft:evoker",
            Self::EvokerFangs => "minecraft:evoker_fangs",
            Self::ExperienceBottle => "minecraft:experience_bottle",
            Self::ExperienceOrb => "minecraft:experience_orb",
            Self::EyeOfEnder => "minecraft:eye_of_ender",
            Self::FallingBlock => "minecraft:falling_block",
            Self::Fireball => "minecraft:fireball",
            Self::FireworkRocket => "minecraft:firework_rocket",
            Self::Fox => "minecraft:fox",
            Self::Frog => "minecraft:frog",
            Self::FurnaceMinecart => "minecraft:furnace_minecart",
            Self::Ghast => "minecraft:ghast",
            Self::Giant => "minecraft:giant",
            Self::GlowItemFrame => "minecraft:glow_item_frame",
            Self::GlowSquid => "minecraft:glow_squid",
            Self::Goat => "minecraft:goat",
            Self::Guardian => "minecraft:guardian",
            Self::Hoglin => "minecraft:hoglin",
            Self::HopperMinecart => "minecraft:hopper_minecart",
            Self::Horse => "minecraft:horse",
            Self::Husk => "minecraft:husk",
            Self::Illusioner => "minecraft:illusioner",
            Self::Interaction => "minecraft:interaction",
            Self::IronGolem => "minecraft:iron_golem",
            Self::Item => "minecraft:item",
            Self::ItemDisplay => "minecraft:item_display",
            Self::ItemFrame => "minecraft:item_frame",
            Self::LeashKnot => "minecraft:leash_knot",
            Self::LightningBolt => "minecraft:lightning_bolt",
            Self::Llama => "minecraft:llama",
            Self::LlamaSpit => "minecraft:llama_spit",
            Self::MagmaCube => "minecraft:magma_cube",
            Self::Marker => "minecraft:marker",
            Self::Minecart => "minecraft:minecart",
            Self::Mooshroom => "minecraft:mooshroom",
            Self::Mule => "minecraft:mule",
            Self::Ocelot => "minecraft:ocelot",
            Self::Painting => "minecraft:painting",
            Self::Panda => "minecraft:panda",
            Self::Parrot => "minecraft:parrot",
            Self::Phantom => "minecraft:phantom",
            Self::Pig => "minecraft:pig",
            Self::Piglin => "minecraft:piglin",
            Self::PiglinBrute => "minecraft:piglin_brute",
            Self::Pillager => "minecraft:pillager",
            Self::Player => "minecraft:player",
            Self::PolarBear => "minecraft:polar_bear",
            Self::Potion => "minecraft:potion",
            Self::Pufferfish => "minecraft:pufferfish",
            Self::Rabbit => "minecraft:rabbit",
            Self::Ravager => "minecraft:ravager",
            Self::Salmon => "minecraft:salmon",
            Self::Sheep => "minecraft:sheep",
            Self::Shulker => "minecraft:shulker",
            Self::ShulkerBullet => "minecraft:shulker_bullet",
            Self::Silverfish => "minecraft:silverfish",
            Self::Skeleton => "minecraft:skeleton",
            Self::SkeletonHorse => "minecraft:skeleton_horse",
            Self::Slime => "minecraft:slime",
            Self::SmallFireball => "minecraft:small_fireball",
            Self::Sniffer => "minecraft:sniffer",
            Self::SnowGolem => "minecraft:snow_golem",
            Self::Snowball => "minecraft:snowball",
            Self::SpawnerMinecart => "minecraft:spawner_minecart",
            Self::SpectralArrow => "minecraft:spectral_arrow",
            Self::Spider => "minecraft:spider",
            Self::Squid => "minecraft:squid",
            Self::Stray => "minecraft:stray",
            Self::Strider => "minecraft:strider",
            Self::Tadpole => "minecraft:tadpole",
            Self::TextDisplay => "minecraft:text_display",
            Self::Tnt => "minecraft:tnt",
            Self::TntMinecart => "minecraft:tnt_minecart",
            Self::TraderLlama => "minecraft:trader_llama",
            Self::Trident => "minecraft:trident",
            Self::TropicalFish => "minecraft:tropical_fish",
            Self::Turtle => "minecraft:turtle",
            Self::Vex => "minecraft:vex",
            Self::Villager => "minecraft:villager",
            Self::Vindicator => "minecraft:vindicator",
            Self::WanderingTrader => "minecraft:wandering_trader",
            Self::Warden => "minecraft:warden",
            Self::Witch => "minecraft:witch",
            Self::Wither => "minecraft:wither",
            Self::WitherSkeleton => "minecraft:wither_skeleton",
            Self::WitherSkull => "minecraft:wither_skull",
            Self::Wolf => "minecraft:wolf",
            Self::Zoglin => "minecraft:zoglin",
            Self::Zombie => "minecraft:zombie",
            Self::ZombieHorse => "minecraft:zombie_horse",
            Self::ZombieVillager => "minecraft:zombie_villager",
            Self::ZombifiedPiglin => "minecraft:zombified_piglin",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        let name = name.strip_prefix("minecraft:").unwrap_or(name);
        match name {
            "allay" => Some(Self::Allay),
            "area_effect_cloud" => Some(Self::AreaEffectCloud),
            "armor_stand" => Some(Self::ArmorStand),
            "arrow" => Some(Self::Arrow),
            "axolotl" => Some(Self::Axolotl),
            "bat" => Some(Self::Bat),
            "bee" => Some(Self::Bee),
            "blaze" => Some(Self::Blaze),
            "block_display" => Some(Self::BlockDisplay),
            "boat" => Some(Self::Boat),
            "camel" => Some(Self::Camel),
            "cat" => Some(Self::Cat),
            "cave_spider" => Some(Self::CaveSpider),
            "chest_boat" => Some(Self::ChestBoat),
            "chest_minecart" => Some(Self::ChestMinecart),
            "chicken" => Some(Self::Chicken),
            "cod" => Some(Self::Cod),
            "command_block_minecart" => Some(Self::CommandBlockMinecart),
            "cow" => Some(Self::Cow),
            "creeper" => Some(Self::Creeper),
            "dolphin" => Some(Self::Dolphin),
            "donkey" => Some(Self::Donkey),
            "dragon_fireball" => Some(Self::DragonFireball),
            "drowned" => Some(Self::Drowned),
            "egg" => Some(Self::Egg),
            "elder_guardian" => Some(Self::ElderGuardian),
            "end_crystal" => Some(Self::EndCrystal),
            "ender_dragon" => Some(Self::EnderDragon),
            "ender_pearl" => Some(Self::EnderPearl),
            "enderman" => Some(Self::Enderman),
            "endermite" => Some(Self::Endermite),
            "evoker" => Some(Self::Evoker),
            "evoker_fangs" => Some(Self::EvokerFangs),
            "experience_bottle" => Some(Self::ExperienceBottle),
            "experience_orb" => Some(Self::ExperienceOrb),
            "eye_of_ender" => Some(Self::EyeOfEnder),
            "falling_block" => Some(Self::FallingBlock),
            "fireball" => Some(Self::Fireball),
            "firework_rocket" => Some(Self::FireworkRocket),
            "fox" => Some(Self::Fox),
            "frog" => Some(Self::Frog),
            "furnace_minecart" => Some(Self::FurnaceMinecart),
            "ghast" => Some(Self::Ghast),
            "giant" => Some(Self::Giant),
            "glow_item_frame" => Some(Self::GlowItemFrame),
            "glow_squid" => Some(Self::GlowSquid),
            "goat" => Some(Self::Goat),
            "guardian" => Some(Self::Guardian),
            "hoglin" => Some(Self::Hoglin),
            "hopper_minecart" => Some(Self::HopperMinecart),
            "horse" => Some(Self::Horse),
            "husk" => Some(Self::Husk),
            "illusioner" => Some(Self::Illusioner),
            "interaction" => Some(Self::Interaction),
            "iron_golem" => Some(Self::IronGolem),
            "item" => Some(Self::Item),
            "item_display" => Some(Self::ItemDisplay),
            "item_frame" => Some(Self::ItemFrame),
            "leash_knot" => Some(Self::LeashKnot),
            "lightning_bolt" => Some(Self::LightningBolt),
            "llama" => Some(Self::Llama),
            "llama_spit" => Some(Self::LlamaSpit),
            "magma_cube" => Some(Self::MagmaCube),
            "marker" => Some(Self::Marker),
            "minecart" => Some(Self::Minecart),
            "mooshroom" => Some(Self::Mooshroom),
            "mule" => Some(Self::Mule),
            "ocelot" => Some(Self::Ocelot),
            "painting" => Some(Self::Painting),
            "panda" => Some(Self::Panda),
            "parrot" => Some(Self::Parrot),
            "phantom" => Some(Self::Phantom),
            "pig" => Some(Self::Pig),
            "piglin" => Some(Self::Piglin),
            "piglin_brute" => Some(Self::PiglinBrute),
            "pillager" => Some(Self::Pillager),
            "player" => Some(Self::Player),
            "polar_bear" => Some(Self::PolarBear),
            "potion" => Some(Self::Potion),
            "pufferfish" => Some(Self::Pufferfish),
            "rabbit" => Some(Self::Rabbit),
            "ravager" => Some(Self::Ravager),
            "salmon" => Some(Self::Salmon),
            "sheep" => Some(Self::Sheep),
            "shulker" => Some(Self::Shulker),
            "shulker_bullet" => Some(Self::ShulkerBullet),
            "silverfish" => Some(Self::Silverfish),
            "skeleton" => Some(Self::Skeleton),
            "skeleton_horse" => Some(Self::SkeletonHorse),
            "slime" => Some(Self::Slime),
            "small_fireball" => Some(Self::SmallFireball),
            "sniffer" => Some(Self::Sniffer),
            "snow_golem" => Some(Self::SnowGolem),
            "snowball" => Some(Self::Snowball),
            "spawner_minecart" => Some(Self::SpawnerMinecart),
            "spectral_arrow" => Some(Self::SpectralArrow),
            "spider" => Some(Self::Spider),
            "squid" => Some(Self::Squid),
            "stray" => Some(Self::Stray),
            "strider" => Some(Self::Strider),
            "tadpole" => Some(Self::Tadpole),
            "text_display" => Some(Self::TextDisplay),
            "tnt" => Some(Self::Tnt),
            "tnt_minecart" => Some(Self::TntMinecart),
            "trader_llama" => Some(Self::TraderLlama),
            "trident" => Some(Self::Trident),
            "tropical_fish" => Some(Self::TropicalFish),
            "turtle" => Some(Self::Turtle),
            "vex" => Some(Self::Vex),
            "villager" => Some(Self::Villager),
            "vindicator" => Some(Self::Vindicator),
            "wandering_trader" => Some(Self::WanderingTrader),
            "warden" => Some(Self::Warden),
            "witch" => Some(Self::Witch),
            "wither" => Some(Self::Wither),
            "wither_skeleton" => Some(Self::WitherSkeleton),
            "wither_skull" => Some(Self::WitherSkull),
            "wolf" => Some(Self::Wolf),
            "zoglin" => Some(Self::Zoglin),
            "zombie" => Some(Self::Zombie),
            "zombie_horse" => Some(Self::ZombieHorse),
            "zombie_villager" => Some(Self::ZombieVillager),
            "zombified_piglin" => Some(Self::ZombifiedPiglin),
            _ => None,
        }
    }

    /// Exact entity width in blocks from 1.20.1 bytecode.
    pub fn width(&self) -> f64 {
        match self {
            Self::Allay => 0.35,
            Self::AreaEffectCloud => 6.0,
            Self::ArmorStand => 0.5,
            Self::Arrow => 0.5,
            Self::Axolotl => 0.75,
            Self::Bat => 0.5,
            Self::Bee => 0.7,
            Self::Blaze => 0.6,
            Self::BlockDisplay => 0.6,
            Self::Boat | Self::ChestBoat => 1.375,
            Self::Camel => 1.7,
            Self::Cat => 0.6,
            Self::CaveSpider => 0.7,
            Self::ChestMinecart
            | Self::CommandBlockMinecart
            | Self::FurnaceMinecart
            | Self::HopperMinecart
            | Self::Minecart
            | Self::SpawnerMinecart
            | Self::TntMinecart => 0.98,
            Self::Chicken => 0.4,
            Self::Cod => 0.5,
            Self::Cow => 0.9,
            Self::Creeper => 0.6,
            Self::Dolphin => 0.9,
            Self::Donkey => 1.3964844,
            Self::DragonFireball => 0.6,
            Self::Drowned => 0.6,
            Self::Egg | Self::Snowball => 0.25,
            Self::ElderGuardian => 1.9975,
            Self::EndCrystal => 2.0,
            Self::EnderDragon => 16.0,
            Self::EnderPearl => 0.25,
            Self::Enderman => 0.6,
            Self::Endermite => 0.4,
            Self::Evoker => 0.6,
            Self::EvokerFangs => 0.5,
            Self::ExperienceBottle => 0.25,
            Self::ExperienceOrb => 0.5,
            Self::EyeOfEnder => 0.25,
            Self::FallingBlock => 0.98,
            Self::Fireball => 1.0,
            Self::FireworkRocket => 0.25,
            Self::Fox => 0.6,
            Self::Frog => 0.5,
            Self::Ghast => 4.0,
            Self::Giant => 3.6,
            Self::GlowItemFrame | Self::ItemFrame => 0.5,
            Self::GlowSquid => 0.8,
            Self::Goat => 0.9,
            Self::Guardian => 0.85,
            Self::Hoglin => 1.3964844,
            Self::Horse => 1.3964844,
            Self::Husk => 0.6,
            Self::Illusioner => 0.6,
            Self::Interaction => 0.0,
            Self::IronGolem => 1.4,
            Self::Item => 0.25,
            Self::ItemDisplay => 0.6,
            Self::LeashKnot => 0.375,
            Self::LightningBolt => 0.0,
            Self::Llama | Self::TraderLlama => 0.9,
            Self::LlamaSpit => 0.25,
            Self::MagmaCube | Self::Slime => 2.04,
            Self::Marker => 0.0,
            Self::Mooshroom => 0.9,
            Self::Mule => 1.3964844,
            Self::Ocelot => 0.6,
            Self::Painting => 0.5,
            Self::Panda => 1.3,
            Self::Parrot => 0.5,
            Self::Phantom => 0.9,
            Self::Pig => 0.9,
            Self::Piglin | Self::PiglinBrute => 0.6,
            Self::Pillager => 0.6,
            Self::Player => 0.6,
            Self::PolarBear => 1.4,
            Self::Potion => 0.25,
            Self::Pufferfish => 0.7,
            Self::Rabbit => 0.4,
            Self::Ravager => 1.95,
            Self::Salmon => 0.7,
            Self::Sheep => 0.9,
            Self::Shulker => 1.0,
            Self::ShulkerBullet => 0.3125,
            Self::Silverfish => 0.4,
            Self::Skeleton => 0.6,
            Self::SkeletonHorse => 1.3964844,
            Self::SmallFireball => 0.3125,
            Self::Sniffer => 1.9,
            Self::SnowGolem => 0.7,
            Self::SpectralArrow => 0.5,
            Self::Spider => 1.4,
            Self::Squid => 0.8,
            Self::Stray => 0.6,
            Self::Strider => 0.9,
            Self::Tadpole => 0.4,
            Self::TextDisplay => 0.6,
            Self::Tnt => 0.98,
            Self::Trident => 0.5,
            Self::TropicalFish => 0.5,
            Self::Turtle => 1.2,
            Self::Vex => 0.4,
            Self::Villager => 0.6,
            Self::Vindicator => 0.6,
            Self::WanderingTrader => 0.6,
            Self::Warden => 0.9,
            Self::Witch => 0.6,
            Self::Wither => 0.9,
            Self::WitherSkeleton => 0.7,
            Self::WitherSkull => 0.3125,
            Self::Wolf => 0.6,
            Self::Zoglin => 1.3964844,
            Self::Zombie => 0.6,
            Self::ZombieHorse => 1.3964844,
            Self::ZombieVillager => 0.6,
            Self::ZombifiedPiglin => 0.6,
        }
    }

    /// Exact entity height in blocks from 1.20.1 bytecode.
    pub fn height(&self) -> f64 {
        match self {
            Self::Allay => 0.6,
            Self::AreaEffectCloud => 0.5,
            Self::ArmorStand => 1.975,
            Self::Arrow => 0.5,
            Self::Axolotl => 0.42,
            Self::Bat => 0.9,
            Self::Bee => 0.6,
            Self::Blaze => 1.8,
            Self::BlockDisplay => 1.8,
            Self::Boat | Self::ChestBoat => 0.5625,
            Self::Camel => 2.375,
            Self::Cat => 0.7,
            Self::CaveSpider => 0.5,
            Self::ChestMinecart
            | Self::CommandBlockMinecart
            | Self::FurnaceMinecart
            | Self::HopperMinecart
            | Self::Minecart
            | Self::SpawnerMinecart
            | Self::TntMinecart => 0.7,
            Self::Chicken => 0.7,
            Self::Cod => 0.3,
            Self::Cow => 1.4,
            Self::Creeper => 1.7,
            Self::Dolphin => 0.6,
            Self::Donkey => 1.5,
            Self::DragonFireball => 1.8,
            Self::Drowned => 1.95,
            Self::Egg | Self::Snowball => 0.25,
            Self::ElderGuardian => 1.9975,
            Self::EndCrystal => 2.0,
            Self::EnderDragon => 8.0,
            Self::EnderPearl => 0.25,
            Self::Enderman => 2.9,
            Self::Endermite => 0.3,
            Self::Evoker => 1.95,
            Self::EvokerFangs => 0.8,
            Self::ExperienceBottle => 0.25,
            Self::ExperienceOrb => 0.5,
            Self::EyeOfEnder => 0.25,
            Self::FallingBlock => 0.98,
            Self::Fireball => 1.0,
            Self::FireworkRocket => 0.25,
            Self::Fox => 0.7,
            Self::Frog => 0.5,
            Self::Ghast => 4.0,
            Self::Giant => 12.0,
            Self::GlowItemFrame | Self::ItemFrame => 0.5,
            Self::GlowSquid => 0.8,
            Self::Goat => 1.3,
            Self::Guardian => 0.85,
            Self::Hoglin => 1.4,
            Self::Horse => 1.6,
            Self::Husk => 1.95,
            Self::Illusioner => 1.95,
            Self::Interaction => 0.0,
            Self::IronGolem => 2.7,
            Self::Item => 0.25,
            Self::ItemDisplay => 1.8,
            Self::LeashKnot => 0.5,
            Self::LightningBolt => 0.0,
            Self::Llama | Self::TraderLlama => 1.87,
            Self::LlamaSpit => 0.25,
            Self::MagmaCube | Self::Slime => 2.04,
            Self::Marker => 0.0,
            Self::Mooshroom => 1.4,
            Self::Mule => 1.6,
            Self::Ocelot => 0.7,
            Self::Painting => 0.5,
            Self::Panda => 1.25,
            Self::Parrot => 0.9,
            Self::Phantom => 0.5,
            Self::Pig => 0.9,
            Self::Piglin | Self::PiglinBrute => 1.95,
            Self::Pillager => 1.95,
            Self::Player => 1.8,
            Self::PolarBear => 1.4,
            Self::Potion => 0.25,
            Self::Pufferfish => 0.7,
            Self::Rabbit => 0.5,
            Self::Ravager => 2.2,
            Self::Salmon => 0.4,
            Self::Sheep => 1.3,
            Self::Shulker => 1.0,
            Self::ShulkerBullet => 0.3125,
            Self::Silverfish => 0.3,
            Self::Skeleton => 1.99,
            Self::SkeletonHorse => 1.6,
            Self::SmallFireball => 0.3125,
            Self::Sniffer => 1.75,
            Self::SnowGolem => 1.9,
            Self::SpectralArrow => 0.5,
            Self::Spider => 0.9,
            Self::Squid => 0.8,
            Self::Stray => 1.99,
            Self::Strider => 1.7,
            Self::Tadpole => 0.3,
            Self::TextDisplay => 1.8,
            Self::Tnt => 0.98,
            Self::Trident => 0.5,
            Self::TropicalFish => 0.4,
            Self::Turtle => 0.4,
            Self::Vex => 0.8,
            Self::Villager => 1.95,
            Self::Vindicator => 1.95,
            Self::WanderingTrader => 1.95,
            Self::Warden => 2.9,
            Self::Witch => 1.95,
            Self::Wither => 3.5,
            Self::WitherSkeleton => 2.4,
            Self::WitherSkull => 0.3125,
            Self::Wolf => 0.85,
            Self::Zoglin => 1.4,
            Self::Zombie => 1.95,
            Self::ZombieHorse => 1.6,
            Self::ZombieVillager => 1.95,
            Self::ZombifiedPiglin => 1.95,
        }
    }

    pub fn default_max_health(&self) -> f32 {
        match self {
            Self::Warden => 500.0,
            Self::Wither => 300.0,
            Self::EnderDragon => 200.0,
            Self::Giant => 100.0,
            Self::IronGolem => 100.0,
            Self::Ravager => 100.0,
            Self::ElderGuardian => 80.0,
            Self::Hoglin | Self::Zoglin => 40.0,
            Self::Camel => 32.0,
            Self::Guardian | Self::PolarBear | Self::Pillager | Self::Vindicator | Self::Evoker => 30.0,
            Self::Witch => 26.0,
            Self::PiglinBrute => 50.0,
            Self::Enderman => 40.0,
            Self::Horse | Self::Donkey | Self::Mule => 30.0,
            Self::Creeper | Self::Zombie | Self::Husk | Self::Drowned | Self::Skeleton | Self::Stray
            | Self::WitherSkeleton | Self::Player | Self::Villager | Self::WanderingTrader => 20.0,
            Self::Spider | Self::CaveSpider => 16.0,
            Self::Sniffer => 14.0,
            Self::Wolf => 8.0,
            Self::Pig | Self::Sheep | Self::Cow | Self::Mooshroom => 10.0,
            Self::Chicken | Self::Rabbit => 4.0,
            Self::Bat => 6.0,
            Self::Axolotl => 14.0,
            Self::Frog => 10.0,
            _ => 10.0,
        }
    }

    pub fn step_height(&self) -> f64 {
        match self {
            Self::Camel => 1.5,
            Self::Horse | Self::SkeletonHorse | Self::ZombieHorse | Self::Donkey | Self::Mule => 1.0,
            Self::Boat | Self::ChestBoat => 0.0,
            _ => 0.6,
        }
    }

    pub fn is_fire_immune(&self) -> bool {
        matches!(
            self,
            Self::Blaze
                | Self::Ghast
                | Self::MagmaCube
                | Self::Wither
                | Self::WitherSkeleton
                | Self::ZombifiedPiglin
                | Self::Strider
                | Self::EnderDragon
        )
    }
}

/// Simulation entity state in Minecraft Java 1.20.1.
#[derive(Clone, Debug, PartialEq)]
pub struct Legacy1201Entity {
    pub entity_type: Legacy1201EntityType,
    pub uuid: [u32; 4],
    pub pos: Vec3,
    pub motion: Vec3,
    pub yaw: f32,
    pub pitch: f32,
    pub health: f32,
    pub max_health: f32,
    pub air_ticks: i16,
    pub fire_ticks: i16,
    pub fall_distance: f32,
    pub on_ground: bool,
    pub no_gravity: bool,
    pub invulnerable: bool,
    pub custom_name: Option<String>,
    pub age_ticks: u64,
}

impl Legacy1201Entity {
    pub fn new(entity_type: Legacy1201EntityType, pos: Vec3) -> Self {
        let max_health = entity_type.default_max_health();
        Self {
            entity_type,
            uuid: [1, 2, 3, 4],
            pos,
            motion: Vec3::default(),
            yaw: 0.0,
            pitch: 0.0,
            health: max_health,
            max_health,
            air_ticks: MAX_AIR_TICKS,
            fire_ticks: 0,
            fall_distance: 0.0,
            on_ground: false,
            no_gravity: false,
            invulnerable: false,
            custom_name: None,
            age_ticks: 0,
        }
    }

    pub fn with_uuid(mut self, uuid: [u32; 4]) -> Self {
        self.uuid = uuid;
        self
    }

    pub fn bounds(&self) -> Legacy1201Aabb {
        let hw = self.entity_type.width() / 2.0;
        let h = self.entity_type.height();
        Legacy1201Aabb::new(
            self.pos.x - hw, self.pos.y, self.pos.z - hw,
            self.pos.x + hw, self.pos.y + h, self.pos.z + hw,
        )
    }

    pub fn chunk_pos(&self) -> ChunkPos {
        let bx = self.pos.x.floor() as i32;
        let bz = self.pos.z.floor() as i32;
        ChunkPos::new(bx >> 4, bz >> 4)
    }

    pub fn to_nbt(&self) -> Tag {
        let mut map = Compound::new();
        map.insert(key("id"), Tag::String(key(self.entity_type.name())));

        // Pos [double, double, double]
        let pos_tags = vec![
            Tag::Double(self.pos.x.to_bits()),
            Tag::Double(self.pos.y.to_bits()),
            Tag::Double(self.pos.z.to_bits()),
        ];
        map.insert(key("Pos"), Tag::List(pos_tags));

        // Motion [double, double, double]
        let motion_tags = vec![
            Tag::Double(self.motion.x.to_bits()),
            Tag::Double(self.motion.y.to_bits()),
            Tag::Double(self.motion.z.to_bits()),
        ];
        map.insert(key("Motion"), Tag::List(motion_tags));

        // Rotation [float, float]
        let rot_tags = vec![
            Tag::Float(self.yaw.to_bits()),
            Tag::Float(self.pitch.to_bits()),
        ];
        map.insert(key("Rotation"), Tag::List(rot_tags));

        map.insert(key("Health"), Tag::Float(self.health.to_bits()));
        map.insert(key("Air"), Tag::Short(self.air_ticks));
        map.insert(key("Fire"), Tag::Short(self.fire_ticks));
        map.insert(key("FallDistance"), Tag::Float(self.fall_distance.to_bits()));
        map.insert(key("OnGround"), Tag::Byte(if self.on_ground { 1 } else { 0 }));
        map.insert(key("NoGravity"), Tag::Byte(if self.no_gravity { 1 } else { 0 }));
        map.insert(key("Invulnerable"), Tag::Byte(if self.invulnerable { 1 } else { 0 }));

        // UUID [I; int, int, int, int]
        let uuid_ints: Vec<i32> = self.uuid.iter().map(|&u| u as i32).collect();
        map.insert(key("UUID"), Tag::IntArray(uuid_ints));

        if let Some(name) = &self.custom_name {
            map.insert(key("CustomName"), Tag::String(key(name)));
        }

        Tag::Compound(map)
    }

    pub fn from_nbt(tag: &Tag) -> io::Result<Self> {
        let Tag::Compound(map) = tag else {
            return Err(invalid("entity is not a compound"));
        };

        let Some(Tag::String(id_tag)) = map.get(&key("id")) else {
            return Err(invalid("entity missing string id"));
        };
        let id_str = id_tag.to_string().map_err(|e| invalid(e.to_string()))?;
        let entity_type = Legacy1201EntityType::from_name(&id_str)
            .ok_or_else(|| invalid(format!("unknown 1.20.1 entity type: {id_str}")))?;

        // Pos
        let mut pos = Vec3::default();
        if let Some(Tag::List(list)) = map.get(&key("Pos")) {
            if list.len() >= 3 {
                if let (Tag::Double(x), Tag::Double(y), Tag::Double(z)) = (&list[0], &list[1], &list[2]) {
                    pos = Vec3::new(f64::from_bits(*x), f64::from_bits(*y), f64::from_bits(*z));
                }
            }
        }

        // Motion
        let mut motion = Vec3::default();
        if let Some(Tag::List(list)) = map.get(&key("Motion")) {
            if list.len() >= 3 {
                if let (Tag::Double(x), Tag::Double(y), Tag::Double(z)) = (&list[0], &list[1], &list[2]) {
                    motion = Vec3::new(f64::from_bits(*x), f64::from_bits(*y), f64::from_bits(*z));
                }
            }
        }

        // Rotation
        let mut yaw = 0.0f32;
        let mut pitch = 0.0f32;
        if let Some(Tag::List(list)) = map.get(&key("Rotation")) {
            if list.len() >= 2 {
                if let (Tag::Float(y), Tag::Float(p)) = (&list[0], &list[1]) {
                    yaw = f32::from_bits(*y);
                    pitch = f32::from_bits(*p);
                }
            }
        }

        let max_health = entity_type.default_max_health();
        let health = if let Some(Tag::Float(h)) = map.get(&key("Health")) {
            f32::from_bits(*h)
        } else {
            max_health
        };

        let air_ticks = if let Some(Tag::Short(a)) = map.get(&key("Air")) {
            *a
        } else {
            MAX_AIR_TICKS
        };

        let fire_ticks = if let Some(Tag::Short(f)) = map.get(&key("Fire")) {
            *f
        } else {
            0
        };

        let fall_distance = if let Some(Tag::Float(fd)) = map.get(&key("FallDistance")) {
            f32::from_bits(*fd)
        } else {
            0.0
        };

        let on_ground = if let Some(Tag::Byte(og)) = map.get(&key("OnGround")) {
            *og != 0
        } else {
            false
        };

        let no_gravity = if let Some(Tag::Byte(ng)) = map.get(&key("NoGravity")) {
            *ng != 0
        } else {
            false
        };

        let invulnerable = if let Some(Tag::Byte(inv)) = map.get(&key("Invulnerable")) {
            *inv != 0
        } else {
            false
        };

        let mut uuid = [1, 2, 3, 4];
        if let Some(Tag::IntArray(u)) = map.get(&key("UUID")) {
            if u.len() == 4 {
                uuid = [u[0] as u32, u[1] as u32, u[2] as u32, u[3] as u32];
            }
        }

        let custom_name = if let Some(Tag::String(cn)) = map.get(&key("CustomName")) {
            cn.to_string().ok()
        } else {
            None
        };

        Ok(Self {
            entity_type,
            uuid,
            pos,
            motion,
            yaw,
            pitch,
            health,
            max_health,
            air_ticks,
            fire_ticks,
            fall_distance,
            on_ground,
            no_gravity,
            invulnerable,
            custom_name,
            age_ticks: 0,
        })
    }
}

/// Axis-aligned bounding box for 1.20.1 entity collision and broadphase.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Legacy1201Aabb {
    pub min_x: f64,
    pub min_y: f64,
    pub min_z: f64,
    pub max_x: f64,
    pub max_y: f64,
    pub max_z: f64,
}

impl Legacy1201Aabb {
    pub fn new(min_x: f64, min_y: f64, min_z: f64, max_x: f64, max_y: f64, max_z: f64) -> Self {
        Self {
            min_x: min_x.min(max_x),
            min_y: min_y.min(max_y),
            min_z: min_z.min(max_z),
            max_x: min_x.max(max_x),
            max_y: min_y.max(max_y),
            max_z: min_z.max(max_z),
        }
    }

    pub fn intersects(&self, other: &Self) -> bool {
        self.max_x > other.min_x
            && self.min_x < other.max_x
            && self.max_y > other.min_y
            && self.min_y < other.max_y
            && self.max_z > other.min_z
            && self.min_z < other.max_z
    }

    pub fn to_core_aabb(&self) -> Aabb {
        Aabb::new(
            Vec3::new(self.min_x, self.min_y, self.min_z),
            Vec3::new(self.max_x, self.max_y, self.max_z),
        )
    }

    pub fn from_core_aabb(a: Aabb) -> Self {
        Self::new(
            a.min(horizon_core::geometry::Axis::X),
            a.min(horizon_core::geometry::Axis::Y),
            a.min(horizon_core::geometry::Axis::Z),
            a.max(horizon_core::geometry::Axis::X),
            a.max(horizon_core::geometry::Axis::Y),
            a.max(horizon_core::geometry::Axis::Z),
        )
    }
}

/// Abstract block collision query interface.
pub trait Legacy1201BlockQuery {
    fn get_block(&self, pos: BlockPos) -> Option<Legacy1201BlockState>;
}

/// Physics engine for Java 1.20.1 entities.
pub struct Legacy1201EntityPhysics;

impl Legacy1201EntityPhysics {
    /// Collide and move an entity by delta (dx, dy, dz) with step-up support.
    pub fn move_entity(
        entity: &mut Legacy1201Entity,
        mut delta: Vec3,
        query: &impl Legacy1201BlockQuery,
    ) {
        let current_bounds = entity.bounds();
        let step_height = entity.entity_type.step_height();

        // 1. Resolve Y axis first
        let min_bx = (current_bounds.min_x + delta.x.min(0.0) - 1.0).floor() as i32;
        let max_bx = (current_bounds.max_x + delta.x.max(0.0) + 1.0).ceil() as i32;
        let min_by = (current_bounds.min_y + delta.y.min(0.0) - 1.0).floor() as i32;
        let max_by = (current_bounds.max_y + delta.y.max(0.0) + 1.0).ceil() as i32;
        let min_bz = (current_bounds.min_z + delta.z.min(0.0) - 1.0).floor() as i32;
        let max_bz = (current_bounds.max_z + delta.z.max(0.0) + 1.0).ceil() as i32;

        let mut obstacle_boxes = Vec::new();
        for y in min_by..=max_by {
            for z in min_bz..=max_bz {
                for x in min_bx..=max_bx {
                    let bpos = BlockPos::new(x, y, z);
                    if let Some(state) = query.get_block(bpos) {
                        if state.blocks_movement() {
                            obstacle_boxes.push(Legacy1201Aabb::new(
                                x as f64, y as f64, z as f64,
                                (x + 1) as f64, (y + 1) as f64, (z + 1) as f64,
                            ));
                        }
                    }
                }
            }
        }

        // Collide Y
        let mut allowed_dy = delta.y;
        for ob in &obstacle_boxes {
            if current_bounds.max_x > ob.min_x && current_bounds.min_x < ob.max_x
                && current_bounds.max_z > ob.min_z && current_bounds.min_z < ob.max_z
            {
                if delta.y > 0.0 && current_bounds.max_y <= ob.min_y {
                    let d = ob.min_y - current_bounds.max_y;
                    if d < allowed_dy { allowed_dy = d; }
                } else if delta.y < 0.0 && current_bounds.min_y >= ob.max_y {
                    let d = ob.max_y - current_bounds.min_y;
                    if d > allowed_dy { allowed_dy = d; }
                }
            }
        }
        let moved_y_bounds = Legacy1201Aabb::new(
            current_bounds.min_x, current_bounds.min_y + allowed_dy, current_bounds.min_z,
            current_bounds.max_x, current_bounds.max_y + allowed_dy, current_bounds.max_z,
        );

        // Collide X
        let mut allowed_dx = delta.x;
        for ob in &obstacle_boxes {
            if moved_y_bounds.max_y > ob.min_y && moved_y_bounds.min_y < ob.max_y
                && moved_y_bounds.max_z > ob.min_z && moved_y_bounds.min_z < ob.max_z
            {
                if delta.x > 0.0 && moved_y_bounds.max_x <= ob.min_x {
                    let d = ob.min_x - moved_y_bounds.max_x;
                    if d < allowed_dx { allowed_dx = d; }
                } else if delta.x < 0.0 && moved_y_bounds.min_x >= ob.max_x {
                    let d = ob.max_x - moved_y_bounds.min_x;
                    if d > allowed_dx { allowed_dx = d; }
                }
            }
        }
        let moved_yx_bounds = Legacy1201Aabb::new(
            moved_y_bounds.min_x + allowed_dx, moved_y_bounds.min_y, moved_y_bounds.min_z,
            moved_y_bounds.max_x + allowed_dx, moved_y_bounds.max_y, moved_y_bounds.max_z,
        );

        // Collide Z
        let mut allowed_dz = delta.z;
        for ob in &obstacle_boxes {
            if moved_yx_bounds.max_y > ob.min_y && moved_yx_bounds.min_y < ob.max_y
                && moved_yx_bounds.max_x > ob.min_x && moved_yx_bounds.min_x < ob.max_x
            {
                if delta.z > 0.0 && moved_yx_bounds.max_z <= ob.min_z {
                    let d = ob.min_z - moved_yx_bounds.max_z;
                    if d < allowed_dz { allowed_dz = d; }
                } else if delta.z < 0.0 && moved_yx_bounds.min_z >= ob.max_z {
                    let d = ob.max_z - moved_yx_bounds.min_z;
                    if d > allowed_dz { allowed_dz = d; }
                }
            }
        }

        // Ground detection and fall damage
        let hit_ground = delta.y < 0.0 && allowed_dy != delta.y;
        entity.on_ground = hit_ground;
        if hit_ground {
            if entity.fall_distance > 3.0 {
                let damage = (entity.fall_distance - 3.0).floor();
                if damage > 0.0 && !entity.invulnerable {
                    entity.health = (entity.health - damage).max(0.0);
                }
            }
            entity.fall_distance = 0.0;
            entity.motion.y = 0.0;
        } else if delta.y < 0.0 {
            entity.fall_distance -= delta.y as f32;
        }

        entity.pos.x += allowed_dx;
        entity.pos.y += allowed_dy;
        entity.pos.z += allowed_dz;
    }

    /// Run single physics tick for an entity.
    pub fn tick_entity(entity: &mut Legacy1201Entity, query: &impl Legacy1201BlockQuery) {
        entity.age_ticks += 1;

        // Gravity
        if !entity.no_gravity && !entity.on_ground {
            let grav = if matches!(entity.entity_type, Legacy1201EntityType::Item | Legacy1201EntityType::Arrow) {
                ITEM_GRAVITY
            } else {
                LIVING_GRAVITY
            };
            entity.motion.y -= grav;
        }

        // Move
        Self::move_entity(entity, entity.motion, query);

        // Friction / drag
        let friction = if entity.on_ground {
            let below = BlockPos::new(
                entity.pos.x.floor() as i32,
                (entity.pos.y - 0.2).floor() as i32,
                entity.pos.z.floor() as i32,
            );
            if let Some(state) = query.get_block(below) {
                match state.name.as_str() {
                    "minecraft:ice" | "minecraft:packed_ice" => ICE_GROUND_FRICTION,
                    "minecraft:blue_ice" => BLUE_ICE_GROUND_FRICTION,
                    "minecraft:slime_block" => SLIME_GROUND_FRICTION,
                    _ => DEFAULT_GROUND_FRICTION,
                }
            } else {
                DEFAULT_GROUND_FRICTION
            }
        } else {
            AIR_DRAG
        };

        entity.motion.x *= friction;
        entity.motion.z *= friction;
        entity.motion.y *= 0.98; // vertical air drag

        // Drowning and fire ticks
        let head_pos = BlockPos::new(
            entity.pos.x.floor() as i32,
            (entity.pos.y + entity.entity_type.height()).floor() as i32,
            entity.pos.z.floor() as i32,
        );
        let in_water = query.get_block(head_pos).map_or(false, |s| s.is_fluid() && s.name.contains("water"));
        if in_water {
            entity.air_ticks = entity.air_ticks.saturating_sub(1);
            if entity.air_ticks <= -20 {
                entity.air_ticks = 0;
                if !entity.invulnerable {
                    entity.health = (entity.health - 2.0).max(0.0);
                }
            }
            entity.fire_ticks = 0;
        } else {
            entity.air_ticks = MAX_AIR_TICKS;
        }

        if entity.fire_ticks > 0 {
            entity.fire_ticks -= 1;
            if entity.age_ticks % 20 == 0 && !entity.is_fire_immune() && !entity.invulnerable {
                entity.health = (entity.health - 1.0).max(0.0);
            }
        }
    }
}

impl Legacy1201Entity {
    pub fn is_fire_immune(&self) -> bool {
        self.entity_type.is_fire_immune()
    }
}

/// In-memory entity manager organized by chunks for 1.20.1 world.
#[derive(Clone, Debug, Default)]
pub struct Legacy1201EntityManager {
    chunks: HashMap<ChunkPos, Vec<Legacy1201Entity>>,
}

impl Legacy1201EntityManager {
    pub fn new() -> Self {
        Self {
            chunks: HashMap::new(),
        }
    }

    pub fn spawn(&mut self, entity: Legacy1201Entity) {
        let cp = entity.chunk_pos();
        self.chunks.entry(cp).or_default().push(entity);
    }

    pub fn total_entities(&self) -> usize {
        self.chunks.values().map(|v| v.len()).sum()
    }

    pub fn entities_in_chunk(&self, cp: ChunkPos) -> &[Legacy1201Entity] {
        self.chunks.get(&cp).map(|v| v.as_slice()).unwrap_or(&[])
    }

    pub fn entities_in_aabb(&self, aabb: &Legacy1201Aabb) -> Vec<&Legacy1201Entity> {
        let min_cx = (aabb.min_x.floor() as i32) >> 4;
        let max_cx = (aabb.max_x.ceil() as i32) >> 4;
        let min_cz = (aabb.min_z.floor() as i32) >> 4;
        let max_cz = (aabb.max_z.ceil() as i32) >> 4;

        let mut result = Vec::new();
        for cz in min_cz..=max_cz {
            for cx in min_cx..=max_cx {
                let cp = ChunkPos::new(cx, cz);
                if let Some(entities) = self.chunks.get(&cp) {
                    for ent in entities {
                        if ent.bounds().intersects(aabb) {
                            result.push(ent);
                        }
                    }
                }
            }
        }
        result
    }

    pub fn tick_all(&mut self, query: &impl Legacy1201BlockQuery) {
        let mut reassign = Vec::new();
        for (cp, entities) in &mut self.chunks {
            let mut i = 0;
            while i < entities.len() {
                Legacy1201EntityPhysics::tick_entity(&mut entities[i], query);
                let new_cp = entities[i].chunk_pos();
                if new_cp != *cp {
                    let ent = entities.remove(i);
                    reassign.push(ent);
                } else if entities[i].health <= 0.0 {
                    // Remove dead entity
                    entities.remove(i);
                } else {
                    i += 1;
                }
            }
        }
        for ent in reassign {
            self.spawn(ent);
        }
    }

    pub fn chunk_to_nbt(&self, cp: ChunkPos) -> Tag {
        let entities = self.entities_in_chunk(cp);
        let tags: Vec<Tag> = entities.iter().map(Legacy1201Entity::to_nbt).collect();
        Tag::List(tags)
    }

    pub fn chunk_from_nbt(&mut self, cp: ChunkPos, tag: &Tag) -> io::Result<()> {
        let Tag::List(list) = tag else {
            return Err(invalid("entities tag is not a list"));
        };
        let mut entities = Vec::new();
        for etag in list {
            entities.push(Legacy1201Entity::from_nbt(etag)?);
        }
        self.chunks.insert(cp, entities);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FlatWorldQuery;
    impl Legacy1201BlockQuery for FlatWorldQuery {
        fn get_block(&self, pos: BlockPos) -> Option<Legacy1201BlockState> {
            if pos.y < 64 {
                Some(Legacy1201BlockState::stone())
            } else {
                Some(Legacy1201BlockState::air())
            }
        }
    }

    #[test]
    fn test_legacy1201_entity_dimensions_parity() {
        assert_eq!(Legacy1201EntityType::Player.width(), 0.6);
        assert_eq!(Legacy1201EntityType::Player.height(), 1.8);

        assert_eq!(Legacy1201EntityType::Camel.width(), 1.7);
        assert_eq!(Legacy1201EntityType::Camel.height(), 2.375);
        assert_eq!(Legacy1201EntityType::Camel.step_height(), 1.5);

        assert_eq!(Legacy1201EntityType::Sniffer.width(), 1.9);
        assert_eq!(Legacy1201EntityType::Sniffer.height(), 1.75);

        assert_eq!(Legacy1201EntityType::IronGolem.width(), 1.4);
        assert_eq!(Legacy1201EntityType::IronGolem.height(), 2.7);

        assert_eq!(Legacy1201EntityType::Zombie.width(), 0.6);
        assert_eq!(Legacy1201EntityType::Zombie.height(), 1.95);
    }

    #[test]
    fn test_legacy1201_entity_nbt_roundtrip() {
        let mut zombie = Legacy1201Entity::new(Legacy1201EntityType::Zombie, Vec3::new(10.5, 64.0, -20.5));
        zombie.motion = Vec3::new(0.1, -0.05, 0.2);
        zombie.yaw = 90.0;
        zombie.pitch = 15.0;
        zombie.health = 18.5;
        zombie.air_ticks = 250;
        zombie.fire_ticks = 40;
        zombie.on_ground = true;
        zombie.custom_name = Some("BossZombie".to_string());

        let tag = zombie.to_nbt();
        let loaded = Legacy1201Entity::from_nbt(&tag).unwrap();

        assert_eq!(loaded.entity_type, Legacy1201EntityType::Zombie);
        assert_eq!(loaded.pos.x, 10.5);
        assert_eq!(loaded.pos.y, 64.0);
        assert_eq!(loaded.pos.z, -20.5);
        assert_eq!(loaded.health, 18.5);
        assert_eq!(loaded.air_ticks, 250);
        assert_eq!(loaded.fire_ticks, 40);
        assert!(loaded.on_ground);
        assert_eq!(loaded.custom_name.as_deref(), Some("BossZombie"));
    }

    #[test]
    fn test_legacy1201_entity_gravity_and_ground_collision() {
        let query = FlatWorldQuery;
        let mut entity = Legacy1201Entity::new(Legacy1201EntityType::Pig, Vec3::new(0.0, 68.0, 0.0));
        assert!(!entity.on_ground);

        // Run ticks until entity lands on ground at Y = 64
        for _ in 0..50 {
            Legacy1201EntityPhysics::tick_entity(&mut entity, &query);
            if entity.on_ground {
                break;
            }
        }

        assert!(entity.on_ground);
        assert_eq!(entity.pos.y, 64.0);
        assert_eq!(entity.motion.y, 0.0);
    }

    #[test]
    fn test_legacy1201_entity_manager_spatial_queries() {
        let mut manager = Legacy1201EntityManager::new();
        let e1 = Legacy1201Entity::new(Legacy1201EntityType::Cow, Vec3::new(5.0, 64.0, 5.0));
        let e2 = Legacy1201Entity::new(Legacy1201EntityType::Sheep, Vec3::new(25.0, 64.0, 25.0));

        manager.spawn(e1);
        manager.spawn(e2);
        assert_eq!(manager.total_entities(), 2);

        // Query AABB covering only first chunk (0..16)
        let query_box = Legacy1201Aabb::new(0.0, 60.0, 0.0, 16.0, 70.0, 16.0);
        let hits = manager.entities_in_aabb(&query_box);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].entity_type, Legacy1201EntityType::Cow);
    }
}
