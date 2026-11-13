//! Compare 1.20.1 scaffolding and powder-snow collision contexts.
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
    liquid_collision::LiquidCollisionWorld,
    nbt::{self, Limits},
    powder_snow_collision::{PowderSnowEntity, PowderSnowWorld},
    registry::{BlockRegistry, BlockStateId},
};
use std::{convert::Infallible, error::Error, fs, sync::Arc};
struct World {
    snapshot: PowderSnowEntity,
    boots: bool,
    walkable: bool,
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
impl LiquidCollisionWorld for World {
    type Error = Infallible;
    fn entity_liquid_shape(&mut self, _: EntityIdentity) -> Result<VoxelShape, Self::Error> {
        unreachable!()
    }
    fn fluid(&mut self, _: BlockPos) -> Result<FluidState, Self::Error> {
        Ok(FluidState::EMPTY)
    }
    fn entity_can_stand(&mut self, _: EntityIdentity, _: FluidState) -> Result<bool, Self::Error> {
        Ok(false)
    }
}
impl PowderSnowWorld for World {
    type Error = Infallible;
    fn powder_snow_entity(&mut self, _: EntityIdentity) -> Result<PowderSnowEntity, Self::Error> {
        Ok(self.snapshot)
    }
    fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> Result<bool, Self::Error> {
        Ok(self.walkable)
    }
    fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> Result<bool, Self::Error> {
        Ok(self.boots)
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
    let collision = BlockCollisionRegistry::new_legacy_1_20_1(Arc::new(cached))?;
    let contexts = [
        CollisionContext::position(1.0),
        CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(1),
                y: 1.0,
                descending: false,
                main_hand: Some(CollisionItem::EMPTY),
            },
            false,
        ),
    ];
    let mut world = World {
        snapshot: PowderSnowEntity {
            fall_distance: 0.0,
            falling_block: false,
        },
        boots: false,
        walkable: false,
    };
    for (mode, context) in contexts.iter().enumerate() {
        for i in 0..registry.len() {
            let shape = collision.collision_shape_with_context(
                &mut world,
                BlockStateId(i as u32),
                BlockPos::new(0, 0, 0),
                context,
            )?;
            emit(&format!("{mode} {i}"), &shape);
        }
    }
    Ok(())
}
