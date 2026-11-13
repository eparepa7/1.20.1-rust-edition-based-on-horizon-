// Local oracle for every 1.20.1 cached collision shape.
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;

public final class Legacy1201CachedCollisionOracle {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected one output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected the 1.20.1 server (DataVersion 3465)");
        Bootstrap.bootStrap();
        Field cache=BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");cache.setAccessible(true);
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for(BlockState state:Block.BLOCK_STATE_REGISTRY) {
                if(cache.get(state)==null) continue;
                var boxes=state.getCollisionShape(EmptyBlockGetter.INSTANCE,BlockPos.ZERO).toAabbs();
                out.printf("%d %d",Block.getId(state),boxes.size());
                for(AABB b:boxes) for(double v:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ}) out.printf(" %016x",Double.doubleToRawLongBits(v));
                out.println();
            }
        }
    }
}
