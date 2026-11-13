// Local development exporter; generated Minecraft data is not distributed.
import java.lang.reflect.*;
import java.nio.file.*;
import java.io.*;
import java.util.*;
import it.unimi.dsi.fastutil.doubles.DoubleList;
import net.minecraft.SharedConstants;
import net.minecraft.core.Direction;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import net.minecraft.world.phys.shapes.*;

public final class Legacy1201CollisionExporter {
    private static <T extends Comparable<T>> String value(Property<T> p, BlockState state) {
        return p.getName(state.getValue(p));
    }
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected one output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected the 1.20.1 server (DataVersion 3465)");
        Bootstrap.bootStrap();
        Field cache = BlockBehaviour.BlockStateBase.class.getDeclaredField("cache"); cache.setAccessible(true);
        Field collision = cache.getType().getDeclaredField("collisionShape"); collision.setAccessible(true);
        Field shapeField = VoxelShape.class.getDeclaredField("shape"); shapeField.setAccessible(true);
        Method coords = VoxelShape.class.getDeclaredMethod("getCoords", Direction.Axis.class); coords.setAccessible(true);
        ListTag states = new ListTag(), shapes = new ListTag();
        IdentityHashMap<VoxelShape,Integer> ids = new IdentityHashMap<>();
        int[] mapping = new int[Block.BLOCK_STATE_REGISTRY.size()];
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            int id = Block.getId(state);
            if (id != states.size()) throw new IllegalStateException("non-dense registry");
            CompoundTag descriptor = new CompoundTag();
            descriptor.putString("id", BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString());
            if (!state.getValues().isEmpty()) {
                CompoundTag properties = new CompoundTag();
                for (Property<?> p : state.getValues().keySet()) properties.putString(p.getName(), value(p, state));
                descriptor.put("properties", properties);
            }
            states.add(descriptor);
            Object c = cache.get(state);
            if (c == null) { mapping[id] = -1; continue; }
            VoxelShape shape = (VoxelShape) collision.get(c);
            if (shape != state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)) throw new IllegalStateException("cache mismatch");
            Integer index = ids.get(shape);
            if (index == null) {
                index = shapes.size(); ids.put(shape, index);
                CompoundTag grid = new CompoundTag();
                grid.putByte("canonical", (byte)(shape == Shapes.empty() ? 1 : shape == Shapes.block() ? 2 : 0));
                grid.putByte("cube", (byte)(shape.getClass().getSimpleName().equals("CubeVoxelShape") ? 1 : 0));
                int axes = 0;
                for (Direction.Axis axis : Direction.Axis.values()) {
                    DoubleList list = (DoubleList)coords.invoke(shape, axis);
                    if (list.getClass().getSimpleName().equals("CubePointRange")) axes |= 1 << axis.ordinal();
                    long[] bits = new long[list.size()];
                    for (int i=0; i<bits.length; i++) bits[i] = Double.doubleToRawLongBits(list.getDouble(i));
                    grid.putLongArray(axis.getName(), bits);
                }
                grid.putByte("cube_axes", (byte)axes);
                DiscreteVoxelShape d = (DiscreteVoxelShape)shapeField.get(shape);
                grid.putIntArray("discrete_min", Arrays.stream(Direction.Axis.values()).mapToInt(d::firstFull).toArray());
                grid.putIntArray("discrete_max", Arrays.stream(Direction.Axis.values()).mapToInt(d::lastFull).toArray());
                byte[] bits = new byte[d.getXSize()*d.getYSize()*d.getZSize()];
                for (int x=0;x<d.getXSize();x++) for(int y=0;y<d.getYSize();y++) for(int z=0;z<d.getZSize();z++)
                    bits[(x*d.getYSize()+y)*d.getZSize()+z] = (byte)(d.isFull(x,y,z)?1:0);
                grid.putByteArray("occupied", bits); shapes.add(grid);
            }
            mapping[id] = index;
        }
        CompoundTag out = new CompoundTag(); out.putInt("DataVersion",3465);
        out.put("states",states); out.put("shapes",shapes); out.putIntArray("cached_shapes",mapping);
        try (DataOutputStream stream = new DataOutputStream(Files.newOutputStream(Path.of(args[0])))) { NbtIo.write(out, stream); }
    }
}
