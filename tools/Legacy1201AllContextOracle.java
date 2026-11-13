// Local 1.20.1 collision oracle for every state with two ordinary contexts.
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.function.Predicate;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.monster.Zombie;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.*;

public final class Legacy1201AllContextOracle {
    public static void main(String[] args)throws Exception{
        if(args.length!=1)throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if(!"1.20.1".equals(SharedConstants.getCurrentVersion().getName()) || SharedConstants.getCurrentVersion().getDataVersion().getVersion()!=3465)throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        Constructor<EntityCollisionContext> ctor=EntityCollisionContext.class.getDeclaredConstructor(boolean.class,double.class,ItemStack.class,Predicate.class,Entity.class);ctor.setAccessible(true);
        Class<?> uc=Class.forName("sun.misc.Unsafe");Field uf=uc.getDeclaredField("theUnsafe");uf.setAccessible(true);Object unsafe=uf.get(null);Method allocate=uc.getMethod("allocateInstance",Class.class);
        Entity entity=(Entity)allocate.invoke(unsafe,Zombie.class);
        Field type=Entity.class.getDeclaredField("type");type.setAccessible(true);type.set(entity,EntityType.ZOMBIE);
        Field armor=Mob.class.getDeclaredField("armorItems");armor.setAccessible(true);armor.set(entity,NonNullList.withSize(4,ItemStack.EMPTY));
        CollisionContext[] contexts={ctor.newInstance(false,1.0,ItemStack.EMPTY,(Predicate<Object>)x->false,null),ctor.newInstance(false,1.0,ItemStack.EMPTY,(Predicate<Object>)x->false,entity)};
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))){
            for(int mode=0;mode<contexts.length;mode++)for(BlockState state:Block.BLOCK_STATE_REGISTRY){
                var boxes=state.getCollisionShape(EmptyBlockGetter.INSTANCE,BlockPos.ZERO,contexts[mode]).toAabbs();
                out.printf("%d %d %d",mode,Block.getId(state),boxes.size());
                for(AABB b:boxes)for(double v:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ})out.printf(" %016x",Double.doubleToRawLongBits(v));
                out.println();
            }
        }
    }
}
