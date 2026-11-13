// Local 1.20.1 oracle for block-entity collision snapshots.
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity;
import net.minecraft.world.level.block.piston.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.VoxelShape;

public final class Legacy1201EntityCollisionOracle {
    private static void emit(PrintWriter out,String prefix,VoxelShape shape) {
        var boxes=shape.toAabbs();out.printf("%s %d",prefix,boxes.size());
        for(AABB b:boxes) for(double v:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ}) out.printf(" %016x",Double.doubleToRawLongBits(v));
        out.println();
    }
    private static BlockGetter world(Object entity) {
        return (BlockGetter)Proxy.newProxyInstance(BlockGetter.class.getClassLoader(),new Class<?>[]{BlockGetter.class},
            (proxy,method,args)->{if(method.getName().equals("getBlockEntity"))return entity;throw new AssertionError(method);});
    }
    @SuppressWarnings("unchecked") public static void main(String[] args) throws Exception {
        if(args.length!=1)throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if(!"1.20.1".equals(SharedConstants.getCurrentVersion().getName()) || SharedConstants.getCurrentVersion().getDataVersion().getVersion()!=3465) throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        Field old=ShulkerBoxBlockEntity.class.getDeclaredField("progressOld");old.setAccessible(true);
        Field progress=ShulkerBoxBlockEntity.class.getDeclaredField("progress");progress.setAccessible(true);
        Field pistonProgress=PistonMovingBlockEntity.class.getDeclaredField("progress");pistonProgress.setAccessible(true);
        Field clipField=PistonMovingBlockEntity.class.getDeclaredField("NOCLIP");clipField.setAccessible(true);
        ThreadLocal<Direction> noclip=(ThreadLocal<Direction>)clipField.get(null);
        BlockPos pos=new BlockPos(17,-64,-33);
        float[] values={0.0f,0.25f,0.5f,0.75f,1.0f};
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for(BlockState state:Block.BLOCK_STATE_REGISTRY) {
                if(!(state.getBlock() instanceof ShulkerBoxBlock))continue;
                var entity=new ShulkerBoxBlockEntity(pos,state);var world=world(entity);
                for(float previous:values)for(float current:values){
                    old.setFloat(entity,previous);progress.setFloat(entity,current);
                    emit(out,"S "+Block.getId(state),state.getCollisionShape(world,pos));
                }
            }
            for(Direction direction:Direction.values()) {
                BlockState moving=Blocks.MOVING_PISTON.defaultBlockState().setValue(MovingPistonBlock.FACING,direction);
                BlockState[] moved={Blocks.STONE.defaultBlockState(),Blocks.OAK_SLAB.defaultBlockState(),
                    Blocks.PISTON.defaultBlockState().setValue(PistonBaseBlock.FACING,direction),
                    Blocks.STICKY_PISTON.defaultBlockState().setValue(PistonBaseBlock.FACING,direction)};
                for(int kind=0;kind<moved.length;kind++)for(boolean extending:new boolean[]{false,true})for(float p:values)for(int clip=0;clip<3;clip++){
                    var entity=new PistonMovingBlockEntity(pos,moving,moved[kind],direction,extending,kind>=2);
                    pistonProgress.setFloat(entity,p);noclip.set(clip==0?null:clip==1?direction:direction.getOpposite());
                    emit(out,"P",moving.getCollisionShape(world(entity),pos));
                }
            }
        } finally {noclip.remove();}
    }
}
