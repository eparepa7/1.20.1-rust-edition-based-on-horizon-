//! Exercise the Rust block-facts decoder on local 1.20.1 data.
//! This checks data loading, not the rest of the game simulation.
use horizon_items::blocks::BlockFacts;
use horizon_world::registry::BlockRegistry;
use std::{error::Error, fs::File, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = args
        .next()
        .map(PathBuf::from)
        .ok_or("expected a data folder")?;
    if args.next().is_some() {
        return Err("expected only one data folder".into());
    }
    let registry = BlockRegistry::from_facts_json(File::open(root.join("facts.json"))?)?;
    let facts = BlockFacts::load(&root, &registry)?;
    if facts.states.len() != registry.len() {
        return Err("gameplay facts do not match registry".into());
    }
    let stone = facts.get(
        registry
            .default_state("minecraft:stone")
            .ok_or("missing stone")?,
    );
    let torch = facts.get(
        registry
            .default_state("minecraft:torch")
            .ok_or("missing torch")?,
    );
    if stone.hardness != 1.5
        || !stone.requires_tool
        || stone.emission != 0
        || stone.sturdy_full != 63
        || !stone.legacy_solid
        || torch.emission != 14
    {
        return Err("known 1.20.1 block properties do not match".into());
    }
    println!(
        "decoded_states={} stone_hardness={} stone_solid={} stone_sturdy_faces={} torch_light={}",
        facts.states.len(),
        stone.hardness,
        stone.legacy_solid,
        stone.sturdy_full,
        torch.emission
    );
    Ok(())
}
