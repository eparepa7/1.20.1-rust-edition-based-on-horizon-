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
        unreachable!()
    }
    fn entity_can_stand(&mut self, _: EntityIdentity, _: FluidState) -> Result<bool, Self::Error> {
        unreachable!()
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
    let pos = BlockPos::new(17, -64, -33);
    for i in 0..registry.len() {
        let state = registry.state(BlockStateId(i as u32)).ok_or("state")?;
        if state.name != "minecraft:scaffolding" {
            continue;
        }
        for descending in [false, true] {
            for height in [
                -65.1, -65.0, -64.99999, -64.9, -64.0, -63.00001, -63.0, -62.0,
            ] {
                let context = CollisionContext::position(height).with_descending(descending);
                let mut world = World {
                    snapshot: PowderSnowEntity {
                        fall_distance: 0.0,
                        falling_block: false,
                    },
                    boots: false,
                    walkable: false,
                };
                emit(
                    &format!("S {i}"),
                    &collision.collision_shape_with_context(
                        &mut world,
                        BlockStateId(i as u32),
                        pos,
                        &context,
                    )?,
                );
            }
        }
    }
    let powder = registry
        .default_state("minecraft:powder_snow")
        .ok_or("missing powder snow")?;
    for kind in 0..5 {
        for distance in [0.0f32, 2.5, 2.6] {
            for descending in [false, true] {
                for height in [-64.0, -63.00001, -63.0, -62.0] {
                    let context = CollisionContext::for_entity(
                        CollisionEntity {
                            identity: EntityIdentity(1),
                            y: height,
                            descending,
                            main_hand: if kind >= 2 {
                                Some(CollisionItem::EMPTY)
                            } else {
                                None
                            },
                        },
                        false,
                    );
                    let mut world = World {
                        snapshot: PowderSnowEntity {
                            fall_distance: f64::from(distance),
                            falling_block: kind == 1,
                        },
                        boots: kind == 3,
                        walkable: kind == 4,
                    };
                    emit(
                        "P",
                        &collision
                            .collision_shape_with_context(&mut world, powder, pos, &context)?,
                    );
                }
            }
        }
    }
    let mut world = World {
        snapshot: PowderSnowEntity {
            fall_distance: 0.0,
            falling_block: false,
        },
        boots: false,
        walkable: false,
    };
    if collision
        .collision_shape_with_context(&mut world, powder, pos, &CollisionContext::placement(None))
        .is_ok()
    {
        return Err("newer placement context was accepted".into());
    }
    if collision
        .collision_shape_with_context(&mut world, powder, pos, &CollisionContext::empty(true))
        .is_ok()
    {
        return Err("newer fluid context was accepted".into());
    }
    Ok(())
}
