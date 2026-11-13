// Local oracle: vanilla's movement clipping against one block collision shape.
// No world loading, saves, block entities, or entity ticks are involved.
import java.io.PrintWriter;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.List;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.EntityCollisionContext;
import net.minecraft.world.phys.shapes.VoxelShape;

public final class Legacy1201SceneMovementOracle {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465) {
            throw new IllegalStateException("expected 1.20.1");
        }
        Bootstrap.bootStrap();
        Constructor<EntityCollisionContext> ctor = EntityCollisionContext.class.getDeclaredConstructor(
                boolean.class, double.class, ItemStack.class, Predicate.class, Entity.class);
        ctor.setAccessible(true);
        Method clip = Entity.class.getDeclaredMethod("collideWithShapes", Vec3.class, AABB.class, List.class);
        clip.setAccessible(true);
        BlockPos pos = new BlockPos(3, 0, 3);
        AABB[] bounds = {
            new AABB(3.2, 1.5, 3.2, 3.8, 3.3, 3.8),
            new AABB(1.8, 0.2, 3.2, 2.4, 0.8, 3.8)
        };
        Vec3[] movement = { new Vec3(0.0, -2.0, 0.0), new Vec3(1.4, 0.0, 0.0) };
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
                for (int scenario = 0; scenario < bounds.length; scenario++) {
                    var context = ctor.newInstance(false, bounds[scenario].minY, ItemStack.EMPTY,
                            (Predicate<Object>) ignored -> false, null);
                    VoxelShape shape = state.getCollisionShape(EmptyBlockGetter.INSTANCE, pos, context)
                            .move(pos.getX(), pos.getY(), pos.getZ());
                    List<VoxelShape> shapes = shape.isEmpty() ? List.of() : List.of(shape);
                    Vec3 result = (Vec3) clip.invoke(null, movement[scenario], bounds[scenario], shapes);
                    out.printf("%d %d %016x %016x %016x%n", Block.getId(state), scenario,
                            Double.doubleToRawLongBits(result.x), Double.doubleToRawLongBits(result.y),
                            Double.doubleToRawLongBits(result.z));
                }
            }
        }
    }
}
