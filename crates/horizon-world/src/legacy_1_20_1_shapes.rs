//! Dynamic selection outlines for 1.20.1. This is not a world adapter.
use crate::{
    block_collision::{DynamicCollisionWorld, ScaffoldingCollision},
    chunk::BlockState,
    collision_context::CollisionContext,
    compression::invalid,
    dynamic_support::{DynamicSupport, DynamicSupportError, scaffolding_stable_shape},
};
use horizon_core::geometry::{Aabb, ShapeError, ShapeOp, Vec3, VoxelShape};
use std::io;

#[derive(Debug)]
pub enum LegacyOutlineError<E> {
    Data(io::Error),
    Dynamic(DynamicSupportError<E>),
    Shape(ShapeError),
}
impl<E: std::fmt::Display> std::fmt::Display for LegacyOutlineError<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Data(e) => e.fmt(f),
            Self::Dynamic(e) => e.fmt(f),
            Self::Shape(e) => e.fmt(f),
        }
    }
}
impl<E: std::error::Error + 'static> std::error::Error for LegacyOutlineError<E> {}

/// None means the state is not one of the uncached 1.20.1 families.
pub fn dynamic_outline<W: DynamicCollisionWorld>(
    state: &BlockState,
    world: &mut W,
    position: [i32; 3],
    context: &CollisionContext<'_>,
) -> Result<Option<VoxelShape>, LegacyOutlineError<W::Error>> {
    if state.name == "minecraft:sulfur_spike" {
        return Ok(None);
    }
    let Some(dynamic) = DynamicSupport::from_state(state).map_err(LegacyOutlineError::Data)? else {
        return Ok(None);
    };
    let shape = match state.name.as_str() {
        // port: net.minecraft.world.level.block.BambooStalkBlock#getShape
        "minecraft:bamboo" => {
            let width = match state.properties.get("leaves").map(String::as_str) {
                Some("large") => 10.0,
                Some("small" | "none") => 6.0,
                _ => return Err(LegacyOutlineError::Data(invalid("invalid bamboo leaves"))),
            };
            let lo = (8.0 - width / 2.0) / 16.0;
            let hi = (8.0 + width / 2.0) / 16.0;
            VoxelShape::from_box(Aabb::new(Vec3::new(lo, 0.0, lo), Vec3::new(hi, 1.0, hi)))
                .map_err(LegacyOutlineError::Shape)?
                .moved(dynamic.offset(position))
        }
        // port: net.minecraft.world.level.block.ScaffoldingBlock#getShape
        "minecraft:scaffolding" => {
            let _ = ScaffoldingCollision::from_state(state).map_err(LegacyOutlineError::Data)?;
            if context.is_holding_item("minecraft:scaffolding") {
                VoxelShape::block()
            } else if state.properties.get("bottom").map(String::as_str) == Some("true") {
                let bottom =
                    VoxelShape::from_box(Aabb::new(Vec3::ZERO, Vec3::new(1.0, 0.125, 1.0)))
                        .map_err(LegacyOutlineError::Shape)?;
                scaffolding_stable_shape()
                    .join(&bottom, ShapeOp::OR)
                    .map_err(LegacyOutlineError::Shape)?
            } else {
                scaffolding_stable_shape()
            }
        }
        // port: net.minecraft.world.level.block.piston.MovingPistonBlock#getShape
        "minecraft:moving_piston" => VoxelShape::empty(),
        // port: net.minecraft.world.level.block.state.BlockBehaviour#getShape
        "minecraft:powder_snow" => VoxelShape::block(),
        // port: net.minecraft.world.level.block.PointedDripstoneBlock#getShape
        // port: net.minecraft.world.level.block.ShulkerBoxBlock#getShape
        _ => dynamic
            .collision_shape(world, position)
            .map_err(LegacyOutlineError::Dynamic)?,
    };
    Ok(Some(shape))
}
