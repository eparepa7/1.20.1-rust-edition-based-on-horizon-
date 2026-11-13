//! Compare 1.20.1 fluid collision shapes and query order.
use horizon_core::{
    geometry::{Axis, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    collision::CachedCollisionRegistry,
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    dynamic_support::DynamicSupportWorld,
    entity_obstruction::EntityIdentity,
    fluid::FluidState,
    liquid_collision::{LiquidCollision, LiquidCollisionWorld},
    nbt::{self, Limits},
    powder_snow_collision::{PowderSnowEntity, PowderSnowWorld},
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    convert::Infallible,
    error::Error,
    fs::{self, File},
    sync::Arc,
};
struct World {
    above: FluidState,
    stands: bool,
    fluid_calls: u32,
    stand_calls: u32,
}
impl LiquidCollisionWorld for World {
    type Error = Infallible;
    fn entity_liquid_shape(&mut self, _: EntityIdentity) -> Result<VoxelShape, Self::Error> {
        panic!("1.20.1 must not query entity shape")
    }
    fn fluid(&mut self, _: BlockPos) -> Result<FluidState, Self::Error> {
        self.fluid_calls += 1;
        Ok(self.above)
    }
    fn entity_can_stand(&mut self, _: EntityIdentity, _: FluidState) -> Result<bool, Self::Error> {
        self.stand_calls += 1;
        Ok(self.stands)
    }
}
impl DynamicSupportWorld for World {
    type Error = Infallible;
    fn shulker_is_closed(&mut self, _: [i32; 3]) -> Result<Option<bool>, Self::Error> {
        Ok(None)
    }
    fn moving_piston_shape(&mut self, _: [i32; 3]) -> Result<Option<VoxelShape>, Self::Error> {
        Ok(None)
    }
}
impl DynamicCollisionWorld for World {
    fn shulker_collision_state(
        &mut self,
        _: [i32; 3],
    ) -> Result<Option<ShulkerCollisionState>, Self::Error> {
        Ok(None)
    }
}
impl PowderSnowWorld for World {
    type Error = Infallible;
    fn powder_snow_entity(&mut self, _: EntityIdentity) -> Result<PowderSnowEntity, Self::Error> {
        unreachable!()
    }
    fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> Result<bool, Self::Error> {
        unreachable!()
    }
    fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> Result<bool, Self::Error> {
        unreachable!()
    }
}
fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    if args.len() != 2 {
        return Err("expected facts.json and collision.nbt".into());
    }
    let registry = BlockRegistry::from_facts_json(File::open(&args[0])?)?;
    let tag = nbt::read_named(&fs::read(&args[1])?, Limits::tables())?
        .0
        .tag;
    let cached = CachedCollisionRegistry::from_legacy_1_20_1_nbt(&tag, &registry)?;
    let collisions = BlockCollisionRegistry::new_legacy_1_20_1(Arc::new(cached))?;
    let fluids = [
        FluidState::from_json(r#"{"id":"minecraft:empty"}"#)?,
        FluidState::from_json(r#"{"id":"minecraft:water"}"#)?,
        FluidState::from_json(r#"{"id":"minecraft:lava"}"#)?,
    ];
    for i in 0..registry.len() {
        let state = registry.state(BlockStateId(i as u32)).ok_or("state")?;
        if LiquidCollision::from_state(state)?.is_none() {
            continue;
        }
        for height in [-64.0, -63.50001, -63.5, -63.0] {
            for stands in [false, true] {
                for above in fluids {
                    let mut world = World {
                        above,
                        stands,
                        fluid_calls: 0,
                        stand_calls: 0,
                    };
                    let context = CollisionContext::for_entity(
                        CollisionEntity {
                            identity: EntityIdentity(1),
                            y: height,
                            descending: false,
                            main_hand: Some(CollisionItem::EMPTY),
                        },
                        false,
                    );
                    let shape = collisions.collision_shape_with_context(
                        &mut world,
                        BlockStateId(i as u32),
                        BlockPos::new(17, -64, -33),
                        &context,
                    )?;
                    print!(
                        "{} {} {} {}",
                        i,
                        world.fluid_calls,
                        world.stand_calls,
                        shape.boxes().len()
                    );
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
        }
    }
    Ok(())
}
