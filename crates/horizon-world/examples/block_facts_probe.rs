//! Load locally prepared block facts through the actual Rust world registry.
//! This verifies block identities and property arithmetic, not a playable game.
use horizon_world::{
    level::states::StateTable,
    registry::{BlockRegistry, BlockStateId},
};
use std::{error::Error, fs::File, io, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = args
        .next()
        .map(PathBuf::from)
        .ok_or("expected a local facts.json path")?;
    if args.next().is_some() {
        return Err("expected only one facts.json path".into());
    }
    let (registry, flags) = BlockRegistry::from_facts_json_with_flags(File::open(path)?)?;
    let table = StateTable::new(&registry)?;
    for index in 0..registry.len() {
        let id = BlockStateId(u32::try_from(index)?);
        let state = registry.state(id).ok_or("missing block state")?;
        if registry.id(state) != Some(id) || table.name(id) != state.name {
            return Err(io::Error::other(format!("block state {index} is inconsistent")).into());
        }
    }
    println!(
        "states={} blocks={} air_states={} fluid_states={}",
        registry.len(),
        table.block_count(),
        flags.iter().filter(|(air, _)| *air).count(),
        flags.iter().filter(|(_, fluid)| *fluid).count()
    );
    Ok(())
}
