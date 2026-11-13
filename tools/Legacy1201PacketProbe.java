// Development-only probe for Minecraft Java 1.20.1's packet class registrations.
// Run against the player's local remapped server and its original libraries.
// The output is local research data, not a network adapter or playable port.
import java.lang.reflect.Method;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;

public class Legacy1201PacketProbe {
    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            throw new IllegalArgumentException("expected a temporary output path");
        }
        Class<?> protocol = Class.forName("net.minecraft.network.ConnectionProtocol");
        Class<?> flow = Class.forName("net.minecraft.network.protocol.PacketFlow");
        Method packetsById = protocol.getMethod("getPacketsByIds", flow);
        List<String> lines = new ArrayList<>();
        for (Object phase : protocol.getEnumConstants()) {
            for (Object direction : flow.getEnumConstants()) {
                Map<?, ?> packets = (Map<?, ?>) packetsById.invoke(phase, direction);
                for (Map.Entry<?, ?> entry : packets.entrySet()) {
                    Class<?> packetClass = (Class<?>) entry.getValue();
                    lines.add(phase + "\t" + direction + "\t" + entry.getKey() + "\t" + packetClass.getName());
                }
            }
        }
        lines.sort(String::compareTo);
        Files.write(Path.of(args[0]), lines, StandardCharsets.UTF_8);
    }
}
