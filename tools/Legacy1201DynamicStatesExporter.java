// Local development tool; no Minecraft classes or data are distributed here.
import com.google.gson.*;
import java.lang.reflect.Field;
import java.nio.charset.StandardCharsets;
import java.nio.file.*;
import net.minecraft.SharedConstants;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;

public final class Legacy1201DynamicStatesExporter {
    private static <T extends Comparable<T>> String valueName(Property<T> property, BlockState state) {
        return property.getName(state.getValue(property));
    }

    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        Bootstrap.bootStrap();
        Field cache = BlockBehaviour.BlockStateBase.class.getDeclaredField("cache");
        cache.setAccessible(true);
        JsonArray rows = new JsonArray();
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            Block block = state.getBlock();
            boolean uncached = cache.get(state) == null;
            if (uncached != block.hasDynamicShape()) {
                throw new IllegalStateException("Unexpected cache state: " + Block.getId(state));
            }
            if (!uncached) continue;
            JsonObject row = new JsonObject();
            row.addProperty("id", Block.getId(state));
            row.addProperty("block", BuiltInRegistries.BLOCK.getKey(block).toString());
            row.addProperty("class", block.getClass().getName());
            row.addProperty("dynamic_shape", block.hasDynamicShape());
            String dependency = switch (block.getClass().getSimpleName()) {
                case "MovingPistonBlock", "ShulkerBoxBlock" -> "block_entity";
                case "BambooStalkBlock", "PointedDripstoneBlock" -> "position_offset";
                case "ScaffoldingBlock", "PowderSnowBlock" -> "collision_context";
                default -> throw new IllegalStateException("Unclassified block: " + block.getClass());
            };
            row.addProperty("shape_dependency", dependency);
            JsonObject properties = new JsonObject();
            for (Property<?> property : state.getValues().keySet()) {
                properties.addProperty(property.getName(), valueName(property, state));
            }
            row.add("properties", properties);
            rows.add(row);
        }
        Files.writeString(Path.of(args[0]), new Gson().toJson(rows), StandardCharsets.UTF_8);
    }
}
