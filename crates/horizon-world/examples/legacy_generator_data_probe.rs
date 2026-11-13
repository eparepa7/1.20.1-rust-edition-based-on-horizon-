//! Verify checked 1.20.1 facts through the full GeneratorData resource owner.
use horizon_core::position::ChunkPos;
use horizon_world::{
    heightmap::HeightmapType,
    nbt::Tag,
    registry::{BlockRegistry, BlockStateId, BlockTable, BlockTableFile},
    worldgen::{
        generator::{GeneratorData, OverworldGenerator},
        terrain::TerrainChunk,
    },
};
use std::{error::Error, fs::File, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 3 {
        return Err("expected facts.json, private worldgen directory, heightmap table".into());
    }
    let registry: &'static BlockRegistry = Box::leak(Box::new(BlockRegistry::from_facts_json(
        File::open(&args[0])?,
    )?));
    let table = BlockTableFile::open(Path::new(&args[2]), registry)?;
    let checked = table.table("generator heightmap")?;
    let dir = Path::new(&args[1]);
    let error = match GeneratorData::load(dir, registry) {
        Ok(_) => return Err("26.3 loader accepted the legacy bundle".into()),
        Err(error) => error.to_string(),
    };
    if !error.contains("requires load_legacy1201") {
        return Err(error.into());
    }
    let duplicate_registry = Box::leak(Box::new(BlockRegistry::from_facts_json(File::open(
        &args[0],
    )?)?));
    if GeneratorData::load_legacy1201(dir, duplicate_registry, checked).is_ok() {
        return Err("a heightmap bound to another registry was accepted".into());
    }
    let Tag::ByteArray(flags) = checked.get("flags").ok_or("flags")? else {
        return Err("flags type".into());
    };
    let mut invalid_flags = flags.clone();
    invalid_flags[0] |= 0x80;
    for (version, flags, message) in [
        (3464, flags.clone(), "DataVersion"),
        (3465, flags[..flags.len() - 1].to_vec(), "flag count"),
        (3465, invalid_flags, "invalid 1.20.1 heightmap flags"),
    ] {
        let invalid = Tag::compound([
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
        ]);
        let table = BlockTable::check(&invalid, registry, "negative case")?;
        let error = match GeneratorData::load_legacy1201(dir, registry, table) {
            Ok(_) => return Err("invalid heightmap was accepted".into()),
            Err(error) => error.to_string(),
        };
        if !error.contains(message) {
            return Err(error.into());
        }
    }
    let data = GeneratorData::load_legacy1201(dir, registry, checked)?;
    if !data.is_legacy1201() || data.biome_names().len() != 64 {
        return Err("unexpected generator profile".into());
    }
    if !data.warnings().is_empty() {
        return Err(format!("generator warnings: {:?}", data.warnings()).into());
    }
    let error = match OverworldGenerator::new(data.clone(), 0) {
        Ok(_) => return Err("unported generation was allowed".into()),
        Err(error) => error.to_string(),
    };
    if !error.contains("pipelines are not implemented") {
        return Err(error.into());
    }
    for (name, expected_air) in [
        ("minecraft:plains", 3),
        ("minecraft:nether_wastes", 1),
        ("minecraft:the_end", 0),
    ] {
        let id = data.biome_id(name).ok_or("missing biome")?;
        let steps = data
            .legacy1201_biome_carvers(id)
            .ok_or("missing carving profile")?;
        if steps.air.len() != expected_air || !steps.liquid.is_empty() {
            return Err("carving step mismatch".into());
        }
    }
    let mut chunk = TerrainChunk::new(
        ChunkPos::new(0, 0),
        0,
        16,
        registry,
        data.block_states().clone(),
    )?;
    let stone = registry.default_state("minecraft:stone").ok_or("stone")?;
    chunk.set_block_raw(3, 4, 3, stone)?;
    chunk.prime_heightmaps(&HeightmapType::ALL)?;
    for (index, bits) in flags.iter().enumerate() {
        let id = BlockStateId(index as u32);
        chunk.set_block_raw(3, 6, 3, id)?;
        for kind in HeightmapType::ALL {
            chunk.update_heightmap(kind, 3, 6, 3, id)?;
        }
        print!(
            "{index} {} {} {} {} {}",
            u8::from(bits & 1 != 0),
            u8::from(bits & 2 != 0),
            u8::from(bits & 4 != 0),
            u8::from(bits & 8 != 0),
            u8::from(bits & 16 != 0)
        );
        for kind in HeightmapType::ALL {
            let height = chunk.heightmap(kind).ok_or("map")?.first_available(3, 3)?;
            let result = match height {
                5 => 0,
                7 => 1,
                _ => return Err(format!("height {height} for state {index}").into()),
            };
            print!(" {result}");
        }
        println!();
    }
    Ok(())
}
