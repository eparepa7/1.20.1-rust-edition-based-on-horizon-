//! Resident-world movement with captured 1.20.1 shulker and piston states.
use horizon_client_render::{
    block_entity_anim::{Shulker, ShulkerStatus},
    chunk::Chunk,
    collisions::{self, SceneCollisions},
    world::World,
};
use horizon_core::geometry::{Aabb, Direction, Vec3};
use horizon_world::{
    piston_shape::MovingPistonState,
    registry::{BlockRegistry, BlockStateId},
};
use std::{error::Error, fs::File, path::Path, sync::Arc};

fn state_facing(
    registry: &BlockRegistry,
    name: &str,
    facing: &str,
) -> Result<BlockStateId, Box<dyn Error>> {
    let id = registry.default_state(name).ok_or("missing default")?;
    let mut state = registry.state(id).ok_or("missing state")?.clone();
    state.properties.insert("facing".into(), facing.into());
    registry.id(&state).ok_or_else(|| "missing facing".into())
}

fn emit(scene: &mut SceneCollisions<'_, '_>, prefix: &str) -> Result<(), Box<dyn Error>> {
    let bounds = [
        Aabb::new(Vec3::new(3.2, 1.5, 3.2), Vec3::new(3.8, 3.3, 3.8)),
        Aabb::new(Vec3::new(1.8, 0.2, 3.2), Vec3::new(2.4, 0.8, 3.8)),
    ];
    let movement = [Vec3::new(0.0, -2.0, 0.0), Vec3::new(1.4, 0.0, 0.0)];
    for scenario in 0..bounds.len() {
        let result = scene.resolve(bounds[scenario], movement[scenario], 0.0, false)?;
        let [x, y, z] = result.components();
        println!(
            "{prefix} {scenario} {:016x} {:016x} {:016x}",
            x.to_bits(),
            y.to_bits(),
            z.to_bits()
        );
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and collision directory".into());
    }
    let registry = BlockRegistry::from_facts_json(File::open(&args[0])?)?;
    let binding = collisions::load_legacy_1_20_1(Path::new(&args[1]), &registry)?;
    let mut world = World::new(registry.default_state("minecraft:air").ok_or("air")?, 0);
    world.chunks.insert((0, 0), Arc::new(Chunk::empty(0, 0)));
    let names = ["down", "up", "north", "south", "west", "east"];
    let values = [0.0f32, 0.25, 0.5, 0.75, 1.0];
    for (dir, name) in names.iter().enumerate() {
        world.set(
            3,
            0,
            3,
            state_facing(&registry, "minecraft:shulker_box", name)?,
        )?;
        for (p, progress) in values.iter().copied().enumerate() {
            world.shulkers.insert(
                [3, 0, 3],
                Shulker {
                    status: ShulkerStatus::Opening,
                    progress,
                    progress_old: progress,
                    ..Shulker::default()
                },
            );
            let mut scene = SceneCollisions {
                world: &world,
                binding: &binding,
                subject: None,
            };
            emit(&mut scene, &format!("S {dir} {p}"))?;
        }
    }
    world.shulkers.clear();
    for (dir, direction) in Direction::ALL.into_iter().enumerate() {
        world.set(
            3,
            0,
            3,
            state_facing(&registry, "minecraft:moving_piston", names[dir])?,
        )?;
        for (kind, name) in ["minecraft:stone", "minecraft:oak_slab"].iter().enumerate() {
            let moved_state = registry.default_state(name).ok_or("missing moved state")?;
            for ext in 0..2 {
                for (p, progress) in values.iter().copied().enumerate() {
                    world.moving_pistons.clear();
                    world.moving_pistons.push((
                        [3, 0, 3],
                        MovingPistonState {
                            moved_state,
                            direction,
                            extending: ext != 0,
                            source: false,
                            progress,
                            previous_progress: 0.0,
                        },
                    ));
                    let mut scene = SceneCollisions {
                        world: &world,
                        binding: &binding,
                        subject: None,
                    };
                    emit(&mut scene, &format!("P {dir} {kind} {ext} {p}"))?;
                }
            }
        }
    }
    Ok(())
}
