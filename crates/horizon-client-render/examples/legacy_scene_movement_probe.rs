//! Resident-world movements against every 1.20.1 state, for comparison with Java.
use horizon_client_render::{
    chunk::Chunk,
    collisions::{self, SceneCollisions},
    world::World,
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::registry::{BlockRegistry, BlockStateId};
use std::{error::Error, fs::File, path::Path, sync::Arc};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and collision directory".into());
    }
    let registry = BlockRegistry::from_facts_json(File::open(&args[0])?)?;
    let binding = collisions::load_legacy_1_20_1(Path::new(&args[1]), &registry)?;
    let air = registry.default_state("minecraft:air").ok_or("air")?;
    let mut world = World::new(air, 0);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let bounds = [
        Aabb::new(Vec3::new(3.2, 1.5, 3.2), Vec3::new(3.8, 3.3, 3.8)),
        Aabb::new(Vec3::new(1.8, 0.2, 3.2), Vec3::new(2.4, 0.8, 3.8)),
    ];
    let movement = [Vec3::new(0.0, -2.0, 0.0), Vec3::new(1.4, 0.0, 0.0)];
    for i in 0..registry.len() {
        world.set(3, 0, 3, BlockStateId(i as u32))?;
        let mut scene = SceneCollisions {
            world: &world,
            binding: &binding,
            subject: None,
        };
        for scenario in 0..bounds.len() {
            let result = scene.resolve(bounds[scenario], movement[scenario], 0.0, false)?;
            let [x, y, z] = result.components();
            println!(
                "{i} {scenario} {:016x} {:016x} {:016x}",
                x.to_bits(),
                y.to_bits(),
                z.to_bits()
            );
        }
    }
    Ok(())
}
