//! Check locally prepared 1.20.1 collision data against its registry.
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
    if CachedCollisionRegistry::from_nbt(&tag, &registry).is_ok() {
        return Err("26.3 loader accepted 1.20.1".into());
    }
    let collision = CachedCollisionRegistry::from_legacy_1_20_1_nbt(&tag, &registry)?;
    let mut missing = 0;
    for i in 0..registry.len() {
        if collision.cached_shape(BlockStateId(i as u32))?.is_none() {
            missing += 1;
        }
    }
    println!(
        "states={} cached={} uncached={} shapes={}",
        registry.len(),
        collision.cached_state_count(),
        missing,
        collision.shape_count()
    );
    Ok(())
}
