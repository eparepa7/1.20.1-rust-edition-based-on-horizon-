// Local exporter of version-pinned heightmap facts. Generated game data stays local.
import java.io.DataOutputStream;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.NbtIo;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

public final class Legacy1201HeightmapExporter {
    private static <T extends Comparable<T>> String value(Property<T> property, BlockState state) {
        return property.getName(state.getValue(property));
    }
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        ListTag states = new ListTag();
        byte[] flags = new byte[Block.BLOCK_STATE_REGISTRY.size()];
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            int id = Block.getId(state);
            if (id != states.size()) throw new IllegalStateException("non-dense registry");
            CompoundTag descriptor = new CompoundTag();
            descriptor.putString("id", BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString());
            if (!state.getValues().isEmpty()) {
                CompoundTag properties = new CompoundTag();
                for (Property<?> property : state.getValues().keySet())
                    properties.putString(property.getName(), value(property, state));
                descriptor.put("properties", properties);
            }
            states.add(descriptor);
            int bits = (state.isAir() ? 1 : 0) | (state.is(Blocks.AIR) ? 2 : 0)
                    | (!state.getFluidState().isEmpty() ? 4 : 0) | (state.blocksMotion() ? 8 : 0)
                    | (state.getBlock() instanceof LeavesBlock ? 16 : 0);
            flags[id] = (byte) bits;
        }
        CompoundTag out = new CompoundTag();
        out.putInt("DataVersion", 3465);
        out.put("states", states);
        out.putByteArray("flags", flags);
        try (DataOutputStream stream = new DataOutputStream(Files.newOutputStream(Path.of(args[0])))) {
            NbtIo.write(out, stream);
        }
    }
}
