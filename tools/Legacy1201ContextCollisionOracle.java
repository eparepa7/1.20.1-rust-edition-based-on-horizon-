// Local 1.20.1 oracle for scaffolding and powder-snow collision contexts.
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.function.Predicate;
import java.util.*;
import com.google.gson.JsonParser;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.item.*;
import net.minecraft.world.entity.monster.Zombie;
import net.minecraft.world.entity.animal.Rabbit;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.tags.EntityTypeTags;
import net.minecraft.world.item.*;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.*;

public final class Legacy1201ContextCollisionOracle {
    private static void emit(PrintWriter out,String prefix,VoxelShape shape){
        var boxes=shape.toAabbs();out.printf("%s %d",prefix,boxes.size());
        for(AABB b:boxes)for(double v:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ})out.printf(" %016x",Double.doubleToRawLongBits(v));
        out.println();
    }
    public static void main(String[] args)throws Exception{
        if(args.length!=1)throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if(!"1.20.1".equals(SharedConstants.getCurrentVersion().getName()) || SharedConstants.getCurrentVersion().getDataVersion().getVersion()!=3465)throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        Constructor<EntityCollisionContext> ctor=EntityCollisionContext.class.getDeclaredConstructor(boolean.class,double.class,ItemStack.class,Predicate.class,Entity.class);ctor.setAccessible(true);
        BlockPos pos=new BlockPos(17,-64,-33);
        double[] heights={-65.1,-65.0,-64.99999,-64.9,-64.0,-63.00001,-63.0,-62.0};
        Class<?> uc=Class.forName("sun.misc.Unsafe");Field uf=uc.getDeclaredField("theUnsafe");uf.setAccessible(true);
        Object unsafe=uf.get(null);Method allocate=uc.getMethod("allocateInstance",Class.class);
        Field type=Entity.class.getDeclaredField("type");type.setAccessible(true);
        Field armor=Mob.class.getDeclaredField("armorItems");armor.setAccessible(true);
        var tagUrl=Legacy1201ContextCollisionOracle.class.getResourceAsStream("/data/minecraft/tags/entity_types/powder_snow_walkable_mobs.json");
        if(tagUrl==null)throw new IllegalStateException("missing server entity tag");
        var holders=new ArrayList<Holder<EntityType<?>>>();
        try(var reader=new InputStreamReader(tagUrl,java.nio.charset.StandardCharsets.UTF_8)){
            for(var id:JsonParser.parseReader(reader).getAsJsonObject().getAsJsonArray("values")){
                var location=new ResourceLocation(id.getAsString());
                var entityType=BuiltInRegistries.ENTITY_TYPE.get(location);
                if(!BuiltInRegistries.ENTITY_TYPE.containsKey(location))throw new IllegalStateException("unknown tagged entity");
                holders.add(entityType.builtInRegistryHolder());
            }
        }
        BuiltInRegistries.ENTITY_TYPE.bindTags(Map.of(EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS,holders));
        if(!EntityType.RABBIT.is(EntityTypeTags.POWDER_SNOW_WALKABLE_MOBS))throw new IllegalStateException("rabbit tag missing");
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))){
            for(BlockState state:Block.BLOCK_STATE_REGISTRY){
                if(!(state.getBlock() instanceof ScaffoldingBlock))continue;
                for(boolean descending:new boolean[]{false,true})for(double height:heights){
                    CollisionContext context=ctor.newInstance(descending,height,ItemStack.EMPTY,(Predicate<Object>)x->false,null);
                    emit(out,"S "+Block.getId(state),state.getCollisionShape(EmptyBlockGetter.INSTANCE,pos,context));
                }
            }
            for(int kind=0;kind<5;kind++)for(float distance:new float[]{0.0f,2.5f,2.6f})
                for(boolean descending:new boolean[]{false,true})for(double height:new double[]{-64.0,-63.00001,-63.0,-62.0}){
                    Class<?> cls=kind==0?ItemEntity.class:kind==1?FallingBlockEntity.class:kind==4?Rabbit.class:Zombie.class;
                    Entity entity=(Entity)allocate.invoke(unsafe,cls);
                    type.set(entity,kind==0?EntityType.ITEM:kind==1?EntityType.FALLING_BLOCK:kind==4?EntityType.RABBIT:EntityType.ZOMBIE);
                    if(kind>=2){NonNullList<ItemStack> equipment=NonNullList.withSize(4,ItemStack.EMPTY);if(kind==3)equipment.set(EquipmentSlot.FEET.getIndex(),new ItemStack(Items.LEATHER_BOOTS));armor.set(entity,equipment);}
                    entity.fallDistance=distance;
                    CollisionContext context=ctor.newInstance(descending,height,ItemStack.EMPTY,(Predicate<Object>)x->false,entity);
                    emit(out,"P",Blocks.POWDER_SNOW.defaultBlockState().getCollisionShape(EmptyBlockGetter.INSTANCE,pos,context));
                }
        }
    }
}
