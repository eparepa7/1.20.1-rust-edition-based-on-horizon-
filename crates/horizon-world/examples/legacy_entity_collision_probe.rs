//! Compare 1.20.1 block-entity collision snapshots with Rust.
use horizon_core::geometry::{Axis, Direction, VoxelShape};
use horizon_world::{
    block_collision::{DynamicCollisionWorld, ShulkerCollisionState},
    collision::CachedCollisionRegistry,
    dynamic_support::{DynamicSupport, DynamicSupportWorld},
    nbt::{self, Limits},
    piston_shape::{MovingPistonState, PistonBlockBindings, PistonCollisionWorld},
    registry::{BlockRegistry, BlockStateId},
};
use std::{error::Error, fs, io};
struct World<'a, 'r> {
    cached: &'a CachedCollisionRegistry<'r>,
    shulker: Option<ShulkerCollisionState>,
}
impl DynamicSupportWorld for World<'_, '_> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> io::Result<Option<bool>> {
        Ok(None)
    }
    fn moving_piston_shape(&mut self, _: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for World<'_, '_> {
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        Ok(self.shulker)
    }
}
impl PistonCollisionWorld for World<'_, '_> {
    type Error = io::Error;
    fn collision_shape(&mut self, id: BlockStateId, _: [i32; 3]) -> io::Result<VoxelShape> {
        self.cached
            .cached_shape(id)?
            .cloned()
            .ok_or_else(|| io::Error::other("uncached piston part"))
    }
}
fn emit(prefix: &str, shape: &VoxelShape) {
    print!("{} {}", prefix, shape.boxes().len());
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
    let bindings = PistonBlockBindings::new(&registry)?;
    let values = [0.0f32, 0.25, 0.5, 0.75, 1.0];
    let pos = [17, -64, -33];
    for i in 0..registry.len() {
        let state = registry
            .state(BlockStateId(i as u32))
            .ok_or("missing state")?;
        if !state.name.ends_with("shulker_box") {
            continue;
        }
        let support = DynamicSupport::from_state(state)?.ok_or("missing shulker shape")?;
        for previous_progress in values {
            for progress in values {
                let mut world = World {
                    cached: &cached,
                    shulker: Some(ShulkerCollisionState {
                        previous_progress,
                        progress,
                    }),
                };
                emit(
                    &format!("S {i}"),
                    &support.collision_shape(&mut world, pos)?,
                );
            }
        }
    }
    let names = ["down", "up", "north", "south", "west", "east"];
    for (n, direction) in Direction::ALL.into_iter().enumerate() {
        for kind in 0..4 {
            let name = [
                "minecraft:stone",
                "minecraft:oak_slab",
                "minecraft:piston",
                "minecraft:sticky_piston",
            ][kind];
            let mut state = registry
                .state(registry.default_state(name).ok_or("missing default")?)
                .ok_or("missing state")?
                .clone();
            if kind >= 2 {
                state.properties.insert("facing".into(), names[n].into());
            }
            let moved_state = registry.id(&state).ok_or("missing moved state")?;
            for extending in [false, true] {
                for progress in values {
                    for clip in 0..3 {
                        let snapshot = MovingPistonState {
                            moved_state,
                            direction,
                            extending,
                            source: kind >= 2,
                            progress,
                            previous_progress: 0.0,
                        };
                        let noclip = match clip {
                            1 => Some(direction),
                            2 => Some(direction.opposite()),
                            _ => None,
                        };
                        let mut world = World {
                            cached: &cached,
                            shulker: None,
                        };
                        emit(
                            "P",
                            &bindings.collision_shape(&mut world, pos, snapshot, noclip)?,
                        );
                    }
                }
            }
        }
    }
    Ok(())
}
