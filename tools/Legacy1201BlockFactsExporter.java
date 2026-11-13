// Local preparation tool. It reads the player's remapped 1.20.1 server at runtime;
// no Minecraft classes or generated resources are included in the project.
import com.google.gson.Gson;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.TreeSet;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

/** Builds the block identity data consumed by Horizon's Rust registry. */
public final class Legacy1201BlockFactsExporter {
    private static <T extends Comparable<T>> String valueName(Property<T> property, BlockState state) {
        return property.getName(state.getValue(property));
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("expected a temporary output path");
        }
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();

        JsonArray states = new JsonArray();
        JsonObject defaults = new JsonObject();
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            String name = BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
            JsonArray row = new JsonArray();
            row.add(name);
            row.add(state.isAir());
            row.add(!state.getFluidState().isEmpty());
            row.add(state.toString());

            JsonObject descriptor = new JsonObject();
            descriptor.addProperty("id", name);
            if (!state.getValues().isEmpty()) {
                JsonObject properties = new JsonObject();
                for (Property<?> property : state.getValues().keySet()) {
                    properties.addProperty(property.getName(), valueName(property, state));
                }
                descriptor.add("properties", properties);
            }
            row.add(descriptor);
            states.add(row);
        }
        for (var name : new TreeSet<>(BuiltInRegistries.BLOCK.keySet())) {
            Block block = BuiltInRegistries.BLOCK.get(name);
            defaults.addProperty(name.toString(), Block.getId(block.defaultBlockState()));
        }
        JsonObject output = new JsonObject();
        output.add("states", states);
        output.add("defaults", defaults);
        Files.writeString(Path.of(args[0]), new Gson().toJson(output), StandardCharsets.UTF_8);
    }
}
