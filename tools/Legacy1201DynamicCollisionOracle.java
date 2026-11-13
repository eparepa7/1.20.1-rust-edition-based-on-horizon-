// Local 1.20.1 oracle for uncached collision without block entities.
import java.io.*;
import java.lang.reflect.Field;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;

public final class Legacy1201DynamicCollisionOracle {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected one output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName()) || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected 1.20.1 server");
        Bootstrap.bootStrap();
        Field cache=BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");cache.setAccessible(true);
        int[][] positions={{0,0,0},{17,-64,-33},{-1,319,-1},{30000000,63,-30000000},
                           {Integer.MAX_VALUE,Integer.MIN_VALUE,Integer.MIN_VALUE},{-42,1,43}};
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for(BlockState state:Block.BLOCK_STATE_REGISTRY) {
                if(cache.get(state)!=null) continue;
                for(int[] pos:positions) {
                    var boxes=state.getCollisionShape(EmptyBlockGetter.INSTANCE,new BlockPos(pos[0],pos[1],pos[2])).toAabbs();
                    out.printf("%d %d",Block.getId(state),boxes.size());
                    for(AABB b:boxes) for(double v:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ}) out.printf(" %016x",Double.doubleToRawLongBits(v));
                    out.println();
                }
            }
        }
    }
}
