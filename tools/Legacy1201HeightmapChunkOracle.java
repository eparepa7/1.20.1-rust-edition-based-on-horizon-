// Local 1.20.1 ProtoChunk/Heightmap oracle, using one isolated in-memory column.
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
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.PalettedContainer;
import net.minecraft.world.level.chunk.ProtoChunk;
import net.minecraft.world.level.chunk.UpgradeData;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.ticks.ProtoChunkTicks;

public final class Legacy1201HeightmapChunkOracle {
    private static int bit(boolean value) { return value ? 1 : 0; }
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        var states = new PalettedContainer<>(Block.BLOCK_STATE_REGISTRY, Blocks.AIR.defaultBlockState(),
                PalettedContainer.Strategy.SECTION_STATES);
        // Biomes are not used in this heightmap-only probe; the section is already populated.
        var section = new LevelChunkSection(states, null);
        var chunk = new ProtoChunk(new ChunkPos(0, 0), UpgradeData.EMPTY,
                new LevelChunkSection[]{section}, new ProtoChunkTicks<>(), new ProtoChunkTicks<>(),
                LevelHeightAccessor.create(0, 16), null, null);
        BlockPos stone = new BlockPos(3, 4, 3), target = new BlockPos(3, 6, 3);
        chunk.setBlockState(stone, Blocks.STONE.defaultBlockState(), false);
        Heightmap.primeHeightmaps(chunk, EnumSet.allOf(Heightmap.Types.class));
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (var state : Block.BLOCK_STATE_REGISTRY) {
                chunk.setBlockState(target, state, false);
                out.printf("%d %d %d %d %d %d", Block.getId(state), bit(state.isAir()),
                        bit(state.is(Blocks.AIR)), bit(!state.getFluidState().isEmpty()),
                        bit(state.blocksMotion()), bit(state.getBlock() instanceof LeavesBlock));
                for (Heightmap.Types kind : Heightmap.Types.values()) {
                    Heightmap map = chunk.getOrCreateHeightmapUnprimed(kind);
                    map.update(3, 6, 3, state);
                    int height = map.getFirstAvailable(3, 3);
                    if (height != 5 && height != 7) throw new IllegalStateException("unexpected height " + height);
                    out.printf(" %d", height == 7 ? 1 : 0);
                }
                out.println();
            }
        }
    }
}
