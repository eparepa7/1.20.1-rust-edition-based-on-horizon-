//! Dump cached 1.20.1 collision AABBs for comparison with the server.
use horizon_core::geometry::Axis;
use horizon_world::{
    collision::CachedCollisionRegistry,
    nbt::{self, Limits},
    registry::{BlockRegistry, BlockStateId},
};
use std::{error::Error, fs};
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and collision.nbt".into());
    }
    let registry = BlockRegistry::from_facts_json(fs::File::open(&args[0])?)?;
    let tag = nbt::read_named(&fs::read(&args[1])?, Limits::tables())?
        .0
        .tag;
    let cached = CachedCollisionRegistry::from_legacy_1_20_1_nbt(&tag, &registry)?;
    for i in 0..registry.len() {
        let Some(shape) = cached.cached_shape(BlockStateId(i as u32))? else {
            continue;
        };
        print!("{} {}", i, shape.boxes().len());
        for b in shape.boxes() {
            for v in [
                b.min(Axis::X),
                b.min(Axis::Y),
                b.min(Axis::Z),
                b.max(Axis::X),
                b.max(Axis::Y),
                b.max(Axis::Z),
            ] {
                print!(" {:016x}", v.to_bits());
            }
        }
        println!();
    }
    Ok(())
}
