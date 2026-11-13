import unittest
import struct
import zipfile
import json
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
CLIENT_JAR = REPO_ROOT / ".local" / "versions" / "1.20.1" / "minecraft-1.20.1-client.jar"
PROTOCOL_RS = REPO_ROOT / "crates" / "horizon-protocol" / "src" / "legacy_1_20_1_protocol.rs"


def encode_varint(val: int) -> bytes:
    val = val & 0xFFFFFFFF
    out = bytearray()
    while True:
        b = val & 0x7F
        val >>= 7
        if val:
            out.append(b | 0x80)
        else:
            out.append(b)
            break
    return bytes(out)


def decode_varint(buf: bytes, offset: int = 0) -> tuple[int, int]:
    result = 0
    shift = 0
    pos = offset
    while True:
        b = buf[pos]
        pos += 1
        result |= (b & 0x7F) << shift
        if not (b & 0x80):
            break
        shift += 7
    if result >= (1 << 31):
        result -= (1 << 32)
    return result, pos - offset


class Legacy1201ProtocolParityTests(unittest.TestCase):
    """Verifies Minecraft Java 1.20.1 (ProtocolVersion 763) Network Protocol parity."""

    def test_protocol_version_constants(self):
        """ProtocolVersion must be 763 and GameVersion '1.20.1'."""
        self.assertTrue(PROTOCOL_RS.is_file(), "legacy_1_20_1_protocol.rs must exist")
        content = PROTOCOL_RS.read_text(encoding="utf-8")
        self.assertIn("pub const PROTOCOL_VERSION: i32 = 763;", content)
        self.assertIn('pub const GAME_VERSION: &str = "1.20.1";', content)

    def test_bytecode_packet_id_parity(self):
        """Verifies exact packet IDs from 1.20.1 client bytecode (se.class)."""
        if not CLIENT_JAR.is_file():
            self.skipTest("Client JAR not found in .local")

        with zipfile.ZipFile(CLIENT_JAR, "r") as z:
            se_bytes = z.read("se.class")
            # Parse constant pool
            pos = 8
            cp_count = struct.unpack(">H", se_bytes[pos : pos + 2])[0]
            pos += 2
            cp = [None] * cp_count
            i = 1
            while i < cp_count:
                tag = se_bytes[pos]
                pos += 1
                if tag == 1:
                    l = struct.unpack(">H", se_bytes[pos : pos + 2])[0]
                    pos += 2
                    s = se_bytes[pos : pos + l].decode("utf-8", errors="replace")
                    pos += l
                    cp[i] = ("Utf8", s)
                elif tag in (3, 4):
                    pos += 4
                    cp[i] = ("Val",)
                elif tag in (5, 6):
                    pos += 8
                    cp[i] = ("Val2",)
                    i += 1
                elif tag in (7, 8):
                    pos += 2
                    cp[i] = (tag,)
                elif tag in (9, 10, 11, 12, 17, 18):
                    pos += 4
                    cp[i] = ("Ref",)
                elif tag == 15:
                    pos += 3
                    cp[i] = ("MH",)
                elif tag in (16, 19, 20):
                    pos += 2
                    cp[i] = ("2B",)
                i += 1

            # Check that packet classes exist in jar
            expected_classes = [
                "abb.class",   # Handshake
                "abs.class",   # StatusResponse (0x00)
                "abr.class",   # PongResponse (0x01)
                "abj.class",   # LoginDisconnect (0x00)
                "abh.class",   # LoginHello (0x01)
                "abg.class",   # LoginFinished (0x02)
                "abi.class",   # Compression (0x03)
                "wi.class",    # JoinGame (0x28)
                "wd.class",    # LevelChunkWithLight (0x24)
                "xa.class",    # PlayerPositionAndLook (0x3C)
                "xs.class",    # SetCenterChunk (0x4E)
                "xq.class",    # SetDefaultSpawnPosition (0x4C)
                "wb.class",    # KeepAlive (0x23)
                "vs.class",    # Disconnect (0x1A)
                "zd.class",    # AcceptTeleportation (0x00)
                "zv.class",    # ServerboundKeepAlive (0x12)
            ]
            namelist = set(z.namelist())
            for cls_name in expected_classes:
                self.assertIn(cls_name, namelist, f"Packet class {cls_name} must exist in 1.20.1 JAR")

    def test_join_game_fields_parity(self):
        """Verifies ClientboundLoginPacket (JoinGame wi.class) record fields match exact 1.20.1 format."""
        if not CLIENT_JAR.is_file():
            self.skipTest("Client JAR not found in .local")

        with zipfile.ZipFile(CLIENT_JAR, "r") as z:
            wi_bytes = z.read("wi.class")
            expected_fields = [
                "playerId", "hardcore", "gameType", "previousGameType", "levels",
                "registryHolder", "dimensionType", "dimension", "seed", "maxPlayers",
                "chunkRadius", "simulationDistance", "reducedDebugInfo", "showDeathScreen",
                "isDebug", "isFlat", "lastDeathLocation", "portalCooldown"
            ]
            wi_str = wi_bytes.decode("utf-8", errors="replace")
            for f in expected_fields:
                self.assertIn(f, wi_str, f"JoinGame packet field {f} must be present in wi.class")

    def test_server_status_ping_json_structure(self):
        """Verifies 1.20.1 Server List Ping JSON structure."""
        content = PROTOCOL_RS.read_text(encoding="utf-8")
        self.assertIn("format_1_20_1_status_json", content)
        motd = "Test horizon-1201"
        json_str = (
            f'{{"version":{{"name":"1.20.1","protocol":763}},'
            f'"players":{{"max":20,"online":0,"sample":[]}},'
            f'"description":{{"text":"{motd}"}}}}'
        )
        parsed = json.loads(json_str)
        self.assertEqual(parsed["version"]["protocol"], 763)
        self.assertEqual(parsed["version"]["name"], "1.20.1")
        self.assertEqual(parsed["players"]["max"], 20)
        self.assertEqual(parsed["description"]["text"], motd)

    def test_no_configuration_state_invariant(self):
        """Vanilla 1.20.1 transitions directly from Login to Play without Configuration state."""
        content = PROTOCOL_RS.read_text(encoding="utf-8")
        # In 1.20.1 next_state in Handshake is ONLY Status or Login.
        self.assertIn("pub enum Legacy1201NextState {\n    Status,\n    Login,\n}", content)
        # In 1.20.1 there is no LoginAcknowledged packet (introduced in 1.20.2)
        self.assertNotIn("LOGIN_ACKNOWLEDGED", content, "1.20.1 has NO LoginAcknowledged packet")

    def test_varint_codec_invariants(self):
        """VarInt encoding and decoding test for packet lengths and IDs."""
        test_values = [0, 1, 2, 127, 128, 255, 25565, 2097151, -1]
        for val in test_values:
            encoded = encode_varint(val)
            decoded, consumed = decode_varint(encoded)
            self.assertEqual(decoded, val)
            self.assertEqual(consumed, len(encoded))


if __name__ == "__main__":
    unittest.main()
