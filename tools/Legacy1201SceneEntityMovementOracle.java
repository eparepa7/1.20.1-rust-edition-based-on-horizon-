// Local 1.20.1 snapshot oracle for moving pistons and shulker collision movement.
import java.io.PrintWriter;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.List;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity;
import net.minecraft.world.level.block.piston.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.*;
import net.minecraft.world.phys.shapes.*;

public final class Legacy1201SceneEntityMovementOracle {
    private static final BlockPos POS = new BlockPos(3, 0, 3);
    private static final AABB[] BOUNDS = {
        new AABB(3.2, 1.5, 3.2, 3.8, 3.3, 3.8),
        new AABB(1.8, 0.2, 3.2, 2.4, 0.8, 3.8)
    };
    private static final Vec3[] MOVEMENT = {new Vec3(0, -2, 0), new Vec3(1.4, 0, 0)};
    private static BlockGetter world(Object entity) {
        return (BlockGetter) Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),
                new Class<?>[]{BlockGetter.class}, (proxy, method, args) -> {
                    if (method.getName().equals("getBlockEntity")) return entity;
                    throw new AssertionError(method);
                });
    }
    private static void emit(PrintWriter out, String prefix, BlockState state, Object entity,
                             Constructor<EntityCollisionContext> ctor, Method clip) throws Exception {
        for (int scenario = 0; scenario < BOUNDS.length; scenario++) {
            var context = ctor.newInstance(false, BOUNDS[scenario].minY, ItemStack.EMPTY,
                    (Predicate<Object>) ignored -> false, null);
            VoxelShape shape = state.getCollisionShape(world(entity), POS, context).move(3, 0, 3);
            var shapes = shape.isEmpty() ? List.<VoxelShape>of() : List.of(shape);
            Vec3 result = (Vec3) clip.invoke(null, MOVEMENT[scenario], BOUNDS[scenario], shapes);
            out.printf("%s %d %016x %016x %016x%n", prefix, scenario,
                    Double.doubleToRawLongBits(result.x), Double.doubleToRawLongBits(result.y),
                    Double.doubleToRawLongBits(result.z));
        }
    }
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        var ctor = EntityCollisionContext.class.getDeclaredConstructor(
                boolean.class, double.class, ItemStack.class, Predicate.class, Entity.class);
        ctor.setAccessible(true);
        Method clip = Entity.class.getDeclaredMethod("collideWithShapes", Vec3.class, AABB.class, List.class);
        clip.setAccessible(true);
        Field old = ShulkerBoxBlockEntity.class.getDeclaredField("progressOld"); old.setAccessible(true);
        Field progress = ShulkerBoxBlockEntity.class.getDeclaredField("progress"); progress.setAccessible(true);
        Field pistonProgress = PistonMovingBlockEntity.class.getDeclaredField("progress"); pistonProgress.setAccessible(true);
        float[] values = {0, 0.25f, 0.5f, 0.75f, 1};
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            Direction[] directions = Direction.values();
            for (int dir = 0; dir < directions.length; dir++) {
                BlockState state = Blocks.SHULKER_BOX.defaultBlockState().setValue(ShulkerBoxBlock.FACING, directions[dir]);
                var entity = new ShulkerBoxBlockEntity(POS, state);
                for (int p = 0; p < values.length; p++) {
                    old.setFloat(entity, values[p]); progress.setFloat(entity, values[p]);
                    emit(out, "S " + dir + " " + p, state, entity, ctor, clip);
                }
            }
            for (int dir = 0; dir < directions.length; dir++) {
                BlockState state = Blocks.MOVING_PISTON.defaultBlockState().setValue(MovingPistonBlock.FACING, directions[dir]);
                BlockState[] moved = {Blocks.STONE.defaultBlockState(), Blocks.OAK_SLAB.defaultBlockState()};
                for (int kind = 0; kind < moved.length; kind++) for (int ext = 0; ext < 2; ext++) {
                    for (int p = 0; p < values.length; p++) {
                        var entity = new PistonMovingBlockEntity(POS, state, moved[kind], directions[dir], ext != 0, false);
                        pistonProgress.setFloat(entity, values[p]);
                        emit(out, "P " + dir + " " + kind + " " + ext + " " + p, state, entity, ctor, clip);
                    }
                }
            }
        }
    }
}
