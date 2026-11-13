// Original development oracle. Reads private bundles; uses the real 1.20.1 codecs.
import com.google.gson.*;
import com.mojang.serialization.JsonOps;
import java.io.PrintWriter;
import java.lang.reflect.Field;
import java.nio.file.*;
import java.util.*;
import java.util.stream.Stream;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.core.registries.*;
import net.minecraft.data.registries.VanillaRegistries;
import net.minecraft.resources.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.tags.*;
import net.minecraft.util.RandomSource;
import net.minecraft.util.valueproviders.FloatProvider;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.carver.*;
import sun.misc.Unsafe;

public final class Legacy1201CarverConfigOracle {
    static SortedMap<String, JsonElement> bundle(Path dir, String name) throws Exception {
        var object = JsonParser.parseString(Files.readString(dir.resolve(name + ".json"))).getAsJsonObject();
        var result = new TreeMap<String, JsonElement>();
        object.entrySet().forEach(e -> result.put(e.getKey(), JsonParser.parseString(e.getValue().getAsString())));
        return result;
    }
    private static WorldGenerationContext context(int min, int height) throws Exception {
        // Supply only the two immutable context fields, then call actual vanilla methods.
        var field = Unsafe.class.getDeclaredField("theUnsafe"); field.setAccessible(true);
        var result = (WorldGenerationContext)((Unsafe)field.get(null)).allocateInstance(WorldGenerationContext.class);
        for (var entry : Map.of("minY", min, "height", height).entrySet()) {
            var member = WorldGenerationContext.class.getDeclaredField(entry.getKey());
            member.setAccessible(true); member.setInt(result, entry.getValue());
        }
        return result;
    }
    static RegistryOps<JsonElement> blockOps(Path dir) throws Exception {
        var tagInput = new HashMap<ResourceLocation, List<TagLoader.EntryWithSource>>();
        for (var entry : bundle(dir, "block_tags").entrySet()) {
            var tag = TagFile.CODEC.parse(JsonOps.INSTANCE, entry.getValue()).getOrThrow(false, s -> {});
            tagInput.put(new ResourceLocation(entry.getKey()), tag.entries().stream()
                .map(e -> new TagLoader.EntryWithSource(e, "oracle")).toList());
        }
        var loader = new TagLoader<Holder<Block>>(id -> BuiltInRegistries.BLOCK.getHolder(ResourceKey.create(Registries.BLOCK, id)).map(h -> h), "tags/blocks");
        var tags = new HashMap<TagKey<Block>, List<Holder<Block>>>();
        loader.build(tagInput).forEach((id, values) -> tags.put(TagKey.create(Registries.BLOCK, id), List.copyOf(values)));
        BuiltInRegistries.BLOCK.bindTags(tags);
        var lookup = HolderLookup.Provider.create(Stream.of(BuiltInRegistries.BLOCK.asLookup()));
        return RegistryOps.create(JsonOps.INSTANCE, lookup);
    }
    @SuppressWarnings({"rawtypes", "unchecked"})
    public static void main(String[] args) throws Exception {
        if (args.length != 2) throw new IllegalArgumentException("expected private bundles and output");
        SharedConstants.tryDetectVersion();
        if (!SharedConstants.getCurrentVersion().getName().equals("1.20.1")
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        var dir = Path.of(args[0]);
        var ops = blockOps(dir);
        var canReplace = WorldCarver.class.getDeclaredMethod("canReplaceBlock", CarverConfiguration.class, net.minecraft.world.level.block.state.BlockState.class);
        canReplace.setAccessible(true);
        var floorField = CaveCarverConfiguration.class.getDeclaredField("floorLevel"); floorField.setAccessible(true);
        try (var out = new PrintWriter(Files.newBufferedWriter(Path.of(args[1])))) {
            for (var entry : bundle(dir, "carvers").entrySet()) {
                var name = entry.getKey();
                Optional<ConfiguredWorldCarver<?>> decoded;
                try {
                    decoded = ConfiguredWorldCarver.DIRECT_CODEC.parse(ops, entry.getValue()).result();
                } catch (IllegalStateException error) {
                    if (!error.getMessage().startsWith("Missing tag ")) throw error;
                    out.println("E " + name); continue;
                }
                if (decoded.isEmpty()) { out.println("E " + name); continue; }
                ConfiguredWorldCarver carver = decoded.get();
                var config = (CarverConfiguration)carver.config();
                var debug = config.debugSettings;
                out.printf("C %s %s %s %d %d %d %d %d%n", name, BuiltInRegistries.CARVER.getKey(carver.worldCarver()),
                    Integer.toUnsignedString(Float.floatToRawIntBits(config.probability)), debug.isDebugMode() ? 1 : 0,
                    Block.getId(debug.getAirState()), Block.getId(debug.getWaterState()), Block.getId(debug.getLavaState()), Block.getId(debug.getBarrierState()));
                for (var state : Block.BLOCK_STATE_REGISTRY)
                    out.printf("B %s %d %d%n", name, Block.getId(state), (boolean)canReplace.invoke(carver.worldCarver(), config, state) ? 1 : 0);
                int[][] contexts = {{-64,384}, {0,128}, {-2032,4064}};
                for (int ci = 0; ci < contexts.length; ci++) {
                    var context = context(contexts[ci][0], contexts[ci][1]);
                    out.printf("L %s %d %d%n", name, ci, config.lavaLevel.resolveY(context));
                    for (long seed : new long[]{0,1,-1,Long.MIN_VALUE,Long.MAX_VALUE,0x12345678}) {
                        var random = RandomSource.create(seed);
                        for (int index = 0; index < 32; index++) {
                            boolean start = carver.isStartChunk(random);
                            int y = config.y.sample(random, context);
                            var values = new ArrayList<Float>(); values.add(config.yScale.sample(random));
                            if (config instanceof CaveCarverConfiguration cave) {
                                values.add(cave.horizontalRadiusMultiplier.sample(random));
                                values.add(cave.verticalRadiusMultiplier.sample(random));
                                values.add(((FloatProvider)floorField.get(cave)).sample(random));
                            } else if (config instanceof CanyonCarverConfiguration canyon) {
                                values.add(canyon.verticalRotation.sample(random)); values.add(canyon.shape.distanceFactor.sample(random));
                                values.add(canyon.shape.thickness.sample(random)); values.add(canyon.shape.horizontalRadiusFactor.sample(random));
                                values.add(canyon.shape.verticalRadiusDefaultFactor); values.add(canyon.shape.verticalRadiusCenterFactor);
                                values.add((float)canyon.shape.widthSmoothness);
                            } else throw new IllegalStateException("unsupported config");
                            out.printf("R %s %d %d %d %d %d", name, ci, seed, index, start ? 1 : 0, y);
                            for (float value : values) out.print(" " + Integer.toUnsignedString(Float.floatToRawIntBits(value)));
                            out.println(" " + random.nextLong());
                        }
                    }
                }
            }
            // Original biome objects provide their AIR/LIQUID lists, not a JSON imitation.
            var vanilla = VanillaRegistries.createLookup();
            var biomes = vanilla.lookupOrThrow(Registries.BIOME);
            for (var name : bundle(dir, "biomes").keySet()) {
                var biome = biomes.getOrThrow(ResourceKey.create(Registries.BIOME, new ResourceLocation(name))).value();
                for (var step : GenerationStep.Carving.values()) {
                    var names = new ArrayList<String>();
                    for (var holder : biome.getGenerationSettings().getCarvers(step)) names.add(holder.unwrapKey().orElseThrow().location().toString());
                    out.printf("S %s %s %s%n", name, step.getName(), String.join(" ", names));
                }
            }
        }
    }
}
