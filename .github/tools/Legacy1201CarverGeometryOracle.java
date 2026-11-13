// Original geometry harness. Vanilla computes paths, ellipsoids and mask checks.
// Only carveBlock is intercepted: block replacement is intentionally outside this test.
import com.google.gson.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;
import java.util.function.Function;
import net.minecraft.SharedConstants;
import net.minecraft.core.*;
import net.minecraft.server.Bootstrap;
import net.minecraft.util.Mth;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.*;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.chunk.*;
import net.minecraft.world.level.levelgen.*;
import net.minecraft.world.level.levelgen.carver.*;
import net.minecraft.world.ticks.ProtoChunkTicks;
import org.apache.commons.lang3.mutable.MutableBoolean;
import sun.misc.Unsafe;

public final class Legacy1201CarverGeometryOracle {
    private static DataOutputStream out;
    private static int count, minimum;
    private static boolean record(BlockPos position) {
        try {
            out.writeInt(((position.getY() - minimum) << 8) | ((position.getZ() & 15) << 4) | (position.getX() & 15));
            count++;
            return true;
        } catch (IOException error) { throw new UncheckedIOException(error); }
    }
    private static final class Cave extends CaveWorldCarver {
        Cave() { super(CaveCarverConfiguration.CODEC); }
        @Override protected boolean carveBlock(CarvingContext cx, CaveCarverConfiguration config, ChunkAccess chunk,
                Function<BlockPos, Holder<Biome>> biomes, CarvingMask mask, BlockPos.MutableBlockPos pos,
                BlockPos.MutableBlockPos below, Aquifer aquifer, MutableBoolean surface) { return record(pos); }
    }
    private static final class Nether extends NetherWorldCarver {
        Nether() { super(CaveCarverConfiguration.CODEC); }
        @Override protected boolean carveBlock(CarvingContext cx, CaveCarverConfiguration config, ChunkAccess chunk,
                Function<BlockPos, Holder<Biome>> biomes, CarvingMask mask, BlockPos.MutableBlockPos pos,
                BlockPos.MutableBlockPos below, Aquifer aquifer, MutableBoolean surface) { return record(pos); }
    }
    private static final class Canyon extends CanyonWorldCarver {
        Canyon() { super(CanyonCarverConfiguration.CODEC); }
        @Override protected boolean carveBlock(CarvingContext cx, CanyonCarverConfiguration config, ChunkAccess chunk,
                Function<BlockPos, Holder<Biome>> biomes, CarvingMask mask, BlockPos.MutableBlockPos pos,
                BlockPos.MutableBlockPos below, Aquifer aquifer, MutableBoolean surface) { return record(pos); }
    }
    private static CarvingContext context(int min, int height) throws Exception {
        var field = Unsafe.class.getDeclaredField("theUnsafe"); field.setAccessible(true);
        var result = (CarvingContext)((Unsafe)field.get(null)).allocateInstance(CarvingContext.class);
        for (var entry : Map.of("minY", min, "height", height).entrySet()) {
            var member = WorldGenerationContext.class.getDeclaredField(entry.getKey()); member.setAccessible(true);
            member.setInt(result, entry.getValue());
        }
        return result;
    }
    private static ProtoChunk chunk(ChunkPos pos, int min, int height, boolean upgrading) {
        var sections = new LevelChunkSection[height / 16];
        for (int i = 0; i < sections.length; i++) sections[i] = new LevelChunkSection(
            new PalettedContainer<>(Block.BLOCK_STATE_REGISTRY, Blocks.STONE.defaultBlockState(), PalettedContainer.Strategy.SECTION_STATES), null);
        var chunk = new ProtoChunk(pos, UpgradeData.EMPTY, sections, new ProtoChunkTicks<>(), new ProtoChunkTicks<>(),
            LevelHeightAccessor.create(min,height), null, null);
        if (upgrading) {
            var tag = new net.minecraft.nbt.CompoundTag(); tag.putString("target_status", "minecraft:full");
            chunk.setBelowZeroRetrogen(BelowZeroRetrogen.read(tag));
        }
        return chunk;
    }
    private static void trig(Path angles, Path output) throws Exception {
        var field = Mth.class.getDeclaredField("SIN"); field.setAccessible(true);
        var table = (float[])field.get(null);
        try (var stream = new DataOutputStream(new BufferedOutputStream(Files.newOutputStream(output)))) {
            stream.writeInt(0x4c435431);
            for (float value : table) stream.writeInt(Float.floatToRawIntBits(value));
            for (var raw : JsonParser.parseString(Files.readString(angles)).getAsJsonArray()) {
                int bits = (int)raw.getAsLong(); float angle = Float.intBitsToFloat(bits);
                stream.writeInt(bits); stream.writeInt(Float.floatToRawIntBits(Mth.sin(angle))); stream.writeInt(Float.floatToRawIntBits(Mth.cos(angle)));
            }
        }
    }
    @SuppressWarnings({"unchecked", "rawtypes"})
    public static void main(String[] args) throws Exception {
        SharedConstants.tryDetectVersion();
        if (!SharedConstants.getCurrentVersion().getName().equals("1.20.1")
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465) throw new IllegalStateException("expected 1.20.1");
        if (args.length == 3 && args[0].equals("trig")) { trig(Path.of(args[1]), Path.of(args[2])); return; }
        if (args.length != 4 || !args[0].equals("geometry")) throw new IllegalArgumentException("expected geometry bundles fixtures output, or trig angles output");
        Bootstrap.bootStrap();
        var ops = Legacy1201CarverConfigOracle.blockOps(Path.of(args[1]));
        var definitions = Legacy1201CarverConfigOracle.bundle(Path.of(args[1]), "carvers");
        var carvers = new TreeMap<String, ConfiguredWorldCarver<?>>();
        for (var entry : definitions.entrySet()) carvers.put(entry.getKey(), ConfiguredWorldCarver.DIRECT_CODEC.parse(ops, entry.getValue()).getOrThrow(false, s -> {}));
        try (var stream = new DataOutputStream(new BufferedOutputStream(Files.newOutputStream(Path.of(args[3]))))) {
            out = stream; out.writeInt(0x4c434731);
            int caseId = 0;
            for (var raw : JsonParser.parseString(Files.readString(Path.of(args[2]))).getAsJsonArray()) {
                var f = raw.getAsJsonObject(); String name = f.get("carver").getAsString();
                var config = carvers.get(name).config();
                String type = definitions.get(name).getAsJsonObject().get("type").getAsString();
                WorldCarver carver = switch(type) { case "minecraft:cave" -> new Cave(); case "minecraft:nether_cave" -> new Nether(); case "minecraft:canyon" -> new Canyon(); default -> throw new IllegalArgumentException(type); };
                int min = f.get("min").getAsInt(), height = f.get("height").getAsInt();
                var target = f.getAsJsonArray("target"); var origin = f.getAsJsonArray("origin");
                boolean upgrading = f.get("upgrading").getAsBoolean();
                var chunk = chunk(new ChunkPos(target.get(0).getAsInt(), target.get(1).getAsInt()), min, height, upgrading);
                if (chunk.isUpgrading() != upgrading) throw new AssertionError("upgrade fixture");
                var random = RandomSource.create(f.get("seed").getAsLong());
                boolean selected = !f.get("select_start").getAsBoolean() || carver.isStartChunk(config,random);
                count = 0; minimum = min;
                out.writeInt(caseId++); out.writeBoolean(selected);
                boolean result = selected && carver.carve(context(min,height), config, chunk, p -> { throw new AssertionError("geometry read biome"); }, random,
                    null, new ChunkPos(origin.get(0).getAsInt(), origin.get(1).getAsInt()), new CarvingMask(height,min));
                out.writeInt(-1); out.writeInt(count); out.writeBoolean(result); out.writeLong(random.nextLong());
            }
        }
    }
}
