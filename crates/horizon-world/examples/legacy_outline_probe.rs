//! Compare dynamic 1.20.1 outlines, block entities, and held-item contexts.
use horizon_core::geometry::{Axis, VoxelShape};
use horizon_world::{
    block_collision::{DynamicCollisionWorld, ShulkerCollisionState},
    collision::CachedCollisionRegistry,
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    dynamic_support::DynamicSupportWorld,
    entity_obstruction::EntityIdentity,
    legacy_1_20_1_shapes::dynamic_outline,
    nbt::{self, Limits},
    registry::{BlockRegistry, BlockStateId},
};
use std::{convert::Infallible, error::Error, fs};
struct World(Option<ShulkerCollisionState>);
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
        Ok(self.0)
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
        if cached.cached_shape(id)?.is_some() {
            continue;
        }
        let state = registry.state(id).ok_or("state")?;
        for pos in positions {
            let shape = dynamic_outline(
                state,
                &mut World(None),
                pos,
                &CollisionContext::empty(false),
            )?
            .ok_or("unhandled outline")?;
            emit(&format!("D {i}"), &shape);
        }
    }
    let values = [0.0f32, 0.25, 0.5, 0.75, 1.0];
    for i in 0..registry.len() {
        let state = registry.state(BlockStateId(i as u32)).ok_or("state")?;
        if !state.name.ends_with("shulker_box") {
            continue;
        }
        for previous_progress in values {
            for progress in values {
                let mut world = World(Some(ShulkerCollisionState {
                    previous_progress,
                    progress,
                }));
                let shape = dynamic_outline(
                    state,
                    &mut world,
                    [17, -64, -33],
                    &CollisionContext::empty(false),
                )?
                .ok_or("shulker outline")?;
                emit(&format!("S {i}"), &shape);
            }
        }
    }
    let items = [
        CollisionItem::EMPTY,
        CollisionItem {
            item: "minecraft:scaffolding",
            count: 1,
        },
        CollisionItem {
            item: "minecraft:scaffolding",
            count: 0,
        },
        CollisionItem {
            item: "minecraft:stone",
            count: 1,
        },
    ];
    for i in 0..registry.len() {
        let state = registry.state(BlockStateId(i as u32)).ok_or("state")?;
        if state.name != "minecraft:scaffolding" {
            continue;
        }
        for item in items {
            let context = CollisionContext::for_entity(
                CollisionEntity {
                    identity: EntityIdentity(1),
                    y: 0.0,
                    descending: false,
                    main_hand: Some(item),
                },
                false,
            );
            let shape = dynamic_outline(state, &mut World(None), [0, 0, 0], &context)?
                .ok_or("scaffold outline")?;
            emit(&format!("H {i}"), &shape);
        }
    }
    Ok(())
}
