//! Compare all uncached 1.20.1 states through the collision registry.
use horizon_core::geometry::{Axis, VoxelShape};
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    collision::CachedCollisionRegistry,
    dynamic_support::DynamicSupportWorld,
    nbt::{self, Limits},
    registry::{BlockRegistry, BlockStateId},
};
use std::{convert::Infallible, error::Error, fs, sync::Arc};
struct EmptyWorld;
impl DynamicSupportWorld for EmptyWorld {
    type Error = Infallible;
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, Self::Error> {
        Ok(None)
    }
    fn moving_piston_shape(&mut self, _: [i32; 3]) -> Result<Option<VoxelShape>, Self::Error> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for EmptyWorld {
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, Self::Error> {
        Ok(None)
    }
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
    let collisions = BlockCollisionRegistry::new_legacy_1_20_1(Arc::new(cached))?;
    let positions = [
        [0, 0, 0],
        [17, -64, -33],
        [-1, 319, -1],
        [30000000, 63, -30000000],
        [i32::MAX, i32::MIN, i32::MIN],
        [-42, 1, 43],
    ];
    for i in 0..registry.len() {
        let id = BlockStateId(i as u32);
        if collisions.cached_shapes().cached_shape(id)?.is_some() {
            continue;
        }
        for pos in positions {
            let shape = collisions.collision_shape(&mut EmptyWorld, id, pos)?;
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
    }
    Ok(())
}
