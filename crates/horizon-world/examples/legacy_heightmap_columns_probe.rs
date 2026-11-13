//! Compare 1.20.1 terrain heightmap priming and writes across section boundaries.
use horizon_core::position::ChunkPos;
use horizon_world::{
    heightmap::{HeightmapType, Legacy1201HeightmapRegistry},
    registry::{BlockRegistry, BlockStateId, BlockTableFile},
    worldgen::terrain::TerrainChunk,
};
use std::{error::Error, fs::File, path::Path};

fn emit(
    chunk: &TerrainChunk<'_, Legacy1201HeightmapRegistry<'_>>,
    step: i32,
    x: i32,
    z: i32,
) -> Result<(), Box<dyn Error>> {
    print!("{step} {x} {z}");
    for kind in HeightmapType::ALL {
        print!(
            " {}",
            chunk
                .heightmap(kind)
                .ok_or("missing heightmap")?
                .first_available(x, z)?
        );
    }
    println!();
    Ok(())
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and heightmap table".into());
    }
    let registry = BlockRegistry::from_facts_json(File::open(&args[0])?)?;
    let table = BlockTableFile::open(Path::new(&args[1]), &registry)?;
    let properties = Legacy1201HeightmapRegistry::from_table(table.table("legacy heightmap")?)?;
    let mut chunk =
        TerrainChunk::new(ChunkPos::new(0, 0), -64, 384, &registry, properties.clone())?;
    let mut second = TerrainChunk::new(ChunkPos::new(1, 0), -64, 384, &registry, properties)?;
    let find = |name| registry.default_state(name).ok_or("missing state");
    let air = find("minecraft:air")?;
    let stone = find("minecraft:stone")?;
    let leaves = find("minecraft:oak_leaves")?;
    let mut wet_state = registry.state(leaves).ok_or("leaves")?.clone();
    wet_state
        .properties
        .insert("waterlogged".into(), "true".into());
    let wet = registry.id(&wet_state).ok_or("wet leaves")?;
    let states = [
        air,
        stone,
        wet,
        find("minecraft:water")?,
        leaves,
        find("minecraft:cave_air")?,
        find("minecraft:stone_slab")?,
    ];
    chunk.set_block_raw(0, -64, 0, stone)?;
    chunk.set_block_raw(15, 319, 15, states[3])?;
    chunk.set_block_raw(8, 16, 8, wet)?;
    chunk.prime_heightmaps(&HeightmapType::ALL)?;
    let operations = [
        (0, -64, 0, 0),
        (15, 319, 15, 1),
        (15, 319, 15, 2),
        (15, 318, 15, 1),
        (15, 319, 15, 0),
        (8, 16, 8, 3),
        (8, 16, 8, 2),
        (8, 15, 8, 1),
        (8, 16, 8, 5),
        (8, 15, 8, 0),
        (8, -1, 8, 2),
        (8, -2, 8, 1),
        (8, -1, 8, 0),
        (0, 0, 15, 6),
        (0, 1, 15, 3),
        (0, 1, 15, 0),
        (15, -64, 0, 2),
        (15, -63, 0, 3),
        (15, -63, 0, 0),
        (15, -64, 0, 0),
    ];
    for (step, x, z) in [(-4, 0, 0), (-3, 15, 15), (-2, 8, 8), (-1, 0, 15)] {
        emit(&chunk, step, x, z)?;
    }
    for (i, (x, y, z, kind)) in operations.into_iter().enumerate() {
        let state: BlockStateId = states[kind];
        chunk.set_block_raw(x, y, z, state)?;
        for map in HeightmapType::ALL {
            chunk.update_heightmap(map, x, y, z, state)?;
        }
        emit(&chunk, i as i32, x, z)?;
    }
    second.prime_heightmaps(&HeightmapType::ALL)?;
    emit(&second, 20, 3, 3)?;
    for (step, which, y, kind) in [
        (21, 1, 10, Some(1)),
        (22, 1, 11, Some(2)),
        (23, 0, 2, Some(3)),
        (24, 1, 0, None),
        (25, 0, 2, Some(0)),
    ] {
        let selected = if which == 0 { &mut chunk } else { &mut second };
        if let Some(kind) = kind {
            let state = states[kind];
            selected.set_block_raw(3, y, 3, state)?;
            for map in HeightmapType::ALL {
                selected.update_heightmap(map, 3, y, 3, state)?;
            }
        }
        emit(selected, step, 3, 3)?;
    }
    let table = BlockTableFile::open(Path::new(&args[1]), &registry)?;
    let properties = Legacy1201HeightmapRegistry::from_table(table.table("legacy heightmap")?)?;
    let mut generated = TerrainChunk::new(ChunkPos::new(2, 0), -64, 384, &registry, properties)?;
    for (i, (y, kind)) in [(0, 1), (1, 2), (1, 3), (1, 0), (0, 0), (-64, 1)]
        .into_iter()
        .enumerate()
    {
        generated.set_block_worldgen(5, y, 5, states[kind])?;
        print!("{} 5 5", 30 + i);
        for kind in [HeightmapType::WorldSurfaceWg, HeightmapType::OceanFloorWg] {
            let map = generated
                .heightmap(kind)
                .ok_or("missing generated heightmap")?;
            print!(" {}", map.first_available(5, 5)?);
        }
        println!();
    }
    Ok(())
}
