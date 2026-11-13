//! Controlled resident-world collision smoke test using local 1.20.1 data.
use horizon_client_render::{
    chunk::Chunk,
    collisions::{self, SceneCollisions},
    world::World,
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::registry::BlockRegistry;
use std::{error::Error, fs::File, path::Path, sync::Arc};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and collision directory".into());
    }
    let registry = BlockRegistry::from_facts_json(File::open(&args[0])?)?;
    let binding = collisions::load_legacy_1_20_1(Path::new(&args[1]), &registry)?;
    assert!(collisions::load(Path::new(&args[1]), &registry).is_err());
    let air = registry.default_state("minecraft:air").ok_or("air")?;
    let stone = registry.default_state("minecraft:stone").ok_or("stone")?;
    let slab = registry
        .default_state("minecraft:stone_slab")
        .ok_or("stone slab")?;
    let mut world = World::new(air, 0);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    world.set(3, 0, 3, stone)?;
    world.set(5, 0, 3, slab)?;
    let mut scene = SceneCollisions {
        world: &world,
        binding: &binding,
        subject: None,
    };
    let bounds = Aabb::new(Vec3::new(3.2, 1.5, 3.2), Vec3::new(3.8, 3.3, 3.8));
    let result = scene.resolve(bounds, Vec3::new(0.0, -1.0, 0.0), 0.0, false)?;
    assert_eq!(result.y.to_bits(), (-0.5_f64).to_bits());
    let slab_bounds = Aabb::new(Vec3::new(5.2, 1.5, 3.2), Vec3::new(5.8, 3.3, 3.8));
    let slab_result = scene.resolve(slab_bounds, Vec3::new(0.0, -2.0, 0.0), 0.0, false)?;
    assert_eq!(slab_result.y.to_bits(), (-1.0_f64).to_bits());
    let air_bounds = Aabb::new(Vec3::new(7.2, 1.5, 3.2), Vec3::new(7.8, 3.3, 3.8));
    let air_result = scene.resolve(air_bounds, Vec3::new(0.0, -2.0, 0.0), 0.0, false)?;
    assert_eq!(air_result.y.to_bits(), (-2.0_f64).to_bits());
    println!("1.20.1 resident-world collision matched: stone, slab, air");
    Ok(())
}
