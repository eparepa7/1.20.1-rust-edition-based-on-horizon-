// Local 1.20.1 heightmap facts and predicate oracle. No game data is committed.
import java.io.PrintWriter;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.SharedConstants;
import net.minecraft.server.Bootstrap;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.levelgen.Heightmap;

public final class Legacy1201HeightmapOracle {
    private static int bit(boolean value) { return value ? 1 : 0; }
    public static void main(String[] args) throws Exception {
        if (args.length != 1) throw new IllegalArgumentException("expected output path");
        SharedConstants.tryDetectVersion();
        if (!"1.20.1".equals(SharedConstants.getCurrentVersion().getName())
                || SharedConstants.getCurrentVersion().getDataVersion().getVersion() != 3465)
            throw new IllegalStateException("expected 1.20.1");
        Bootstrap.bootStrap();
        try (PrintWriter out = new PrintWriter(Files.newBufferedWriter(Path.of(args[0])))) {
            for (var state : Block.BLOCK_STATE_REGISTRY) {
                out.printf("%d %d %d %d %d %d", Block.getId(state), bit(state.isAir()),
                        bit(state.is(Blocks.AIR)), bit(!state.getFluidState().isEmpty()),
                        bit(state.blocksMotion()), bit(state.getBlock() instanceof LeavesBlock));
                for (Heightmap.Types kind : Heightmap.Types.values())
                    out.printf(" %d", bit(kind.isOpaque().test(state)));
                out.println();
            }
        }
    }
}
