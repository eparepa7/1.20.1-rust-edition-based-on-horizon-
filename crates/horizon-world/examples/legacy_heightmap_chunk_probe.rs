//! Verify 1.20.1 table-backed heightmaps through actual terrain chunk writes.
use horizon_core::position::ChunkPos;
use horizon_world::{
    heightmap::{HeightmapType, Legacy1201HeightmapRegistry},
    registry::{BlockRegistry, BlockStateId, BlockTableFile},
    worldgen::terrain::TerrainChunk,
};
use std::{error::Error, fs::File, path::Path};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and heightmap table".into());
    }
    let registry = BlockRegistry::from_facts_json(File::open(&args[0])?)?;
    let table = BlockTableFile::open(Path::new(&args[1]), &registry)?;
    let properties = Legacy1201HeightmapRegistry::from_table(table.table("legacy heightmap")?)?;
    let mut chunk = TerrainChunk::new(ChunkPos::new(0, 0), 0, 16, &registry, properties)?;
    let stone = registry.default_state("minecraft:stone").ok_or("stone")?;
    chunk.set_block_raw(3, 4, 3, stone)?;
    chunk.prime_heightmaps(&HeightmapType::ALL)?;
    for index in 0..registry.len() {
        let id = BlockStateId(index as u32);
        chunk.set_block_raw(3, 6, 3, id)?;
        for kind in HeightmapType::ALL {
            chunk.update_heightmap(kind, 3, 6, 3, id)?;
        }
        let state = chunk.block_properties(id);
        print!(
            "{index} {} {} {} {} {}",
            u8::from(state.air),
            u8::from(state.air_block),
            u8::from(state.fluid),
            u8::from(state.blocks_motion),
            u8::from(state.leaves_block)
        );
        for kind in HeightmapType::ALL {
            let height = chunk
                .heightmap(kind)
                .ok_or("missing map")?
                .first_available(3, 3)?;
            let result = match height {
                5 => 0,
                7 => 1,
                _ => return Err(format!("unexpected height {height} for state {index}").into()),
            };
            print!(" {result}");
        }
        println!();
    }
    Ok(())
}
