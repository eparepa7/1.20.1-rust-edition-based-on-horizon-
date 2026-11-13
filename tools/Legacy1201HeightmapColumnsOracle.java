// 1.20.1 in-memory ProtoChunk oracle for heightmap priming and column updates.
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.EnumSet;
import net.minecraft.SharedConstants;
import net.minecraft.core.BlockPos;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.ticks.ProtoChunkTicks;

public final class Legacy1201HeightmapColumnsOracle {
    private static void emit(PrintWriter out, ProtoChunk chunk, int step, int x, int z) {
        out.printf("%d %d %d", step, x, z);
        for (Heightmap.Types kind : Heightmap.Types.values())
            out.printf(" %d", chunk.getOrCreateHeightmapUnprimed(kind).getFirstAvailable(x, z));
        out.println();
    }
    private static void emitGeneration(PrintWriter out, ProtoChunk chunk, int step) {
        out.printf("%d 5 5", step);
        for (Heightmap.Types kind : new Heightmap.Types[]{Heightmap.Types.WORLD_SURFACE_WG, Heightmap.Types.OCEAN_FLOOR_WG}) {
            if (!chunk.hasPrimedHeightmap(kind)) throw new IllegalStateException("missing generated heightmap " + kind);
            out.printf(" %d", chunk.getOrCreateHeightmapUnprimed(kind).getFirstAvailable(5, 5));
        }
        out.println();
    }
    private static ProtoChunk createChunk(int cx) {
        LevelChunkSection[] sections = new LevelChunkSection[24];
        for (int i = 0; i < sections.length; i++) {
            var states = new PalettedContainer<>(Block.BLOCK_STATE_REGISTRY, Blocks.AIR.defaultBlockState(),
                    PalettedContainer.Strategy.SECTION_STATES);
            sections[i] = new LevelChunkSection(states, null);
        }
        return new ProtoChunk(new ChunkPos(cx, 0), UpgradeData.EMPTY, sections,
                new ProtoChunkTicks<>(), new ProtoChunkTicks<>(), LevelHeightAccessor.create(-64, 384), null, null);
    }
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        var chunk = createChunk(0);
        BlockState[] states = {Blocks.AIR.defaultBlockState(), Blocks.STONE.defaultBlockState(),
                Blocks.OAK_LEAVES.defaultBlockState().setValue(LeavesBlock.WATERLOGGED, true),
                Blocks.WATER.defaultBlockState(), Blocks.OAK_LEAVES.defaultBlockState(),
                Blocks.CAVE_AIR.defaultBlockState(), Blocks.STONE_SLAB.defaultBlockState()};
        chunk.setBlockState(new BlockPos(0, -64, 0), states[1], false);
        chunk.setBlockState(new BlockPos(15, 319, 15), states[3], false);
        chunk.setBlockState(new BlockPos(8, 16, 8), states[2], false);
        Heightmap.primeHeightmaps(chunk, EnumSet.allOf(Heightmap.Types.class));
        int[][] operations = {
            {0,-64,0,0}, {15,319,15,1}, {15,319,15,2}, {15,318,15,1},
            {15,319,15,0}, {8,16,8,3}, {8,16,8,2}, {8,15,8,1},
            {8,16,8,5}, {8,15,8,0}, {8,-1,8,2}, {8,-2,8,1},
            {8,-1,8,0}, {0,0,15,6}, {0,1,15,3}, {0,1,15,0},
            {15,-64,0,2}, {15,-63,0,3}, {15,-63,0,0}, {15,-64,0,0}
        };
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            emit(out, chunk, -4, 0, 0); emit(out, chunk, -3, 15, 15);
            emit(out, chunk, -2, 8, 8); emit(out, chunk, -1, 0, 15);
            for (int i = 0; i < operations.length; i++) {
                int[] op = operations[i];
                BlockState state = states[op[3]];
                chunk.setBlockState(new BlockPos(op[0], op[1], op[2]), state, false);
                for (Heightmap.Types kind : Heightmap.Types.values())
                    chunk.getOrCreateHeightmapUnprimed(kind).update(op[0], op[1], op[2], state);
                emit(out, chunk, i, op[0], op[2]);
            }
            var second = createChunk(1);
            Heightmap.primeHeightmaps(second, EnumSet.allOf(Heightmap.Types.class));
            emit(out, second, 20, 3, 3);
            int[][] shared = {{21,1,10,1}, {22,1,11,2}, {23,0,2,3}, {24,1,0,-1}, {25,0,2,0}};
            for (int[] op : shared) {
                ProtoChunk selected = op[1] == 0 ? chunk : second;
                if (op[3] >= 0) {
                    BlockState state = states[op[3]];
                    int worldX = op[1] == 0 ? 3 : 19;
                    selected.setBlockState(new BlockPos(worldX, op[2], 3), state, false);
                    for (Heightmap.Types kind : Heightmap.Types.values())
                        selected.getOrCreateHeightmapUnprimed(kind).update(3, op[2], 3, state);
                }
                emit(out, selected, op[0], 3, 3);
            }
            ProtoChunk generated = createChunk(2);
            int[][] generation = {{0, 1}, {1, 2}, {1, 3}, {1, 0}, {0, 0}, {-64, 1}};
            for (int i = 0; i < generation.length; i++) {
                int[] op = generation[i];
                generated.setBlockState(new BlockPos(37, op[0], 5), states[op[1]], false);
                emitGeneration(out, generated, 30 + i);
            }
        }
    }
}
