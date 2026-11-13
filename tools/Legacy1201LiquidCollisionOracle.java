// Local 1.20.1 oracle for water and lava collision contexts.
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.*;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.*;

public final class Legacy1201LiquidCollisionOracle {
    public static void main(String[] args) throws Exception {
        if(args.length!=1)throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if(!"1.20.1".equals(SharedConstants.getCurrentVersion().getName()) || SharedConstants.getCurrentVersion().getDataVersion().getVersion()!=3465) throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        Constructor<EntityCollisionContext> ctor=EntityCollisionContext.class.getDeclaredConstructor(boolean.class,double.class,ItemStack.class,Predicate.class,Entity.class);ctor.setAccessible(true);
        double[] heights={-64.0,-63.50001,-63.5,-63.0};
        FluidState[] fluids={Fluids.EMPTY.defaultFluidState(),Fluids.WATER.defaultFluidState(),Fluids.LAVA.defaultFluidState()};
        BlockPos pos=new BlockPos(17,-64,-33);
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for(BlockState state:Block.BLOCK_STATE_REGISTRY) {
                if(!(state.getBlock() instanceof LiquidBlock))continue;
                for(double height:heights)for(boolean stands:new boolean[]{false,true})for(FluidState above:fluids){
                    int[] calls={0,0};Predicate<FluidState> predicate=x->{calls[1]++;return stands;};
                    CollisionContext context=ctor.newInstance(false,height,ItemStack.EMPTY,predicate,null);
                    BlockGetter world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},
                        (proxy,method,values)->{if(method.getName().equals("getFluidState")){calls[0]++;return above;}throw new AssertionError(method);});
                    var boxes=state.getCollisionShape(world,pos,context).toAabbs();
                    out.printf("%d %d %d %d",Block.getId(state),calls[0],calls[1],boxes.size());
                    for(AABB b:boxes)for(double v:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ})out.printf(" %016x",Double.doubleToRawLongBits(v));
                    out.println();
                }
            }
        }
    }
}
