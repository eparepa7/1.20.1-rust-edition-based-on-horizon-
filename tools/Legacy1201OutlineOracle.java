// Local 1.20.1 oracle for dynamic selection outlines.
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.item.*;
import net.minecraft.world.level.*;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity;
import net.minecraft.world.level.block.state.*;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.*;

public final class Legacy1201OutlineOracle {
    private static void emit(PrintWriter out,String prefix,VoxelShape shape){
        var boxes=shape.toAabbs();out.printf("%s %d",prefix,boxes.size());
        for(AABB b:boxes)for(double v:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ})out.printf(" %016x",Double.doubleToRawLongBits(v));
        out.println();
    }
    public static void main(String[] args)throws Exception{
        if(args.length!=1)throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if(!"1.20.1".equals(SharedConstants.getCurrentVersion().getName()) || SharedConstants.getCurrentVersion().getDataVersion().getVersion()!=3465) throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        Field cache=BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");cache.setAccessible(true);
        int[][] positions={{0,0,0},{17,-64,-33},{-1,319,-1},{30000000,63,-30000000},
                           {Integer.MAX_VALUE,Integer.MIN_VALUE,Integer.MIN_VALUE},{-42,1,43}};
        BlockPos pos=new BlockPos(17,-64,-33);
        Field old=ShulkerBoxBlockEntity.class.getDeclaredField("progressOld");old.setAccessible(true);
        Field progress=ShulkerBoxBlockEntity.class.getDeclaredField("progress");progress.setAccessible(true);
        Constructor<EntityCollisionContext> ctor=EntityCollisionContext.class.getDeclaredConstructor(boolean.class,double.class,ItemStack.class,Predicate.class,Entity.class);ctor.setAccessible(true);
        float[] values={0.0f,0.25f,0.5f,0.75f,1.0f};
        ItemStack[] held={ItemStack.EMPTY,new ItemStack(Items.SCAFFOLDING),new ItemStack(Items.SCAFFOLDING,0),new ItemStack(Items.STONE)};
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))){
            for(BlockState state:Block.BLOCK_STATE_REGISTRY){
                if(cache.get(state)!=null)continue;
                for(int[] p:positions)emit(out,"D "+Block.getId(state),state.getShape(EmptyBlockGetter.INSTANCE,new BlockPos(p[0],p[1],p[2])));
            }
            for(BlockState state:Block.BLOCK_STATE_REGISTRY){
                if(!(state.getBlock() instanceof ShulkerBoxBlock))continue;
                var entity=new ShulkerBoxBlockEntity(pos,state);
                BlockGetter world=(BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},
                    (proxy,method,arguments)->{if(method.getName().equals("getBlockEntity"))return entity;throw new AssertionError(method);});
                for(float previous:values)for(float current:values){old.setFloat(entity,previous);progress.setFloat(entity,current);emit(out,"S "+Block.getId(state),state.getShape(world,pos));}
            }
            for(BlockState state:Block.BLOCK_STATE_REGISTRY){
                if(!(state.getBlock() instanceof ScaffoldingBlock))continue;
                for(ItemStack item:held){
                    CollisionContext context=ctor.newInstance(false,0.0,item,(Predicate<Object>)x->false,null);
                    emit(out,"H "+Block.getId(state),state.getShape(EmptyBlockGetter.INSTANCE,BlockPos.ZERO,context));
                }
            }
        }
    }
}
