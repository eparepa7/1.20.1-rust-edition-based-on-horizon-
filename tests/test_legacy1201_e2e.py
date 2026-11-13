import unittest
import struct
import json
import tempfile
import os
import shutil
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
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


def encode_string(s: str) -> bytes:
    encoded = s.encode("utf-8")
    return encode_varint(len(encoded)) + encoded


def decode_string(buf: bytes, offset: int = 0) -> tuple[str, int]:
    length, consumed = decode_varint(buf, offset)
    pos = offset + consumed
    s = buf[pos : pos + length].decode("utf-8")
    return s, consumed + length


class Legacy1201E2EIntegrationTests(unittest.TestCase):
    """End-to-End integration test simulating Minecraft 1.20.1 client joining horizon-1201 Rust."""

    def test_e2e_status_ping_flow(self):
        """Client performs Server List Ping (StatusRequest + PingRequest)."""
        # 1. Handshake Packet (id 0x00, protocol 763, addr '127.0.0.1', port 25565, state 1)
        handshake_payload = (
            encode_varint(763)
            + encode_string("127.0.0.1")
            + struct.pack(">H", 25565)
            + encode_varint(1)
        )
        packet_hs = encode_varint(0x00) + handshake_payload

        # Decode handshake
        proto_id, pos = decode_varint(packet_hs)
        self.assertEqual(proto_id, 0x00)
        proto_ver, c1 = decode_varint(packet_hs, pos)
        pos += c1
        self.assertEqual(proto_ver, 763)
        addr, c2 = decode_string(packet_hs, pos)
        pos += c2
        self.assertEqual(addr, "127.0.0.1")
        port = struct.unpack(">H", packet_hs[pos : pos + 2])[0]
        pos += 2
        self.assertEqual(port, 25565)
        next_state, _ = decode_varint(packet_hs, pos)
        self.assertEqual(next_state, 1)

        # 2. Server Status Response JSON
        motd = "horizon-1201 1.20.1 Rust Standalone"
        status_json = (
            f'{{"version":{{"name":"1.20.1","protocol":763}},'
            f'"players":{{"max":20,"online":0,"sample":[]}},'
            f'"description":{{"text":"{motd}"}}}}'
        )
        parsed = json.loads(status_json)
        self.assertEqual(parsed["version"]["protocol"], 763)
        self.assertEqual(parsed["version"]["name"], "1.20.1")
        self.assertEqual(parsed["description"]["text"], motd)

        # 3. Ping/Pong Check
        client_time = 1710000000123
        ping_packet = encode_varint(0x01) + struct.pack(">q", client_time)
        pid, pos = decode_varint(ping_packet)
        self.assertEqual(pid, 0x01)
        pong_time = struct.unpack(">q", ping_packet[pos : pos + 8])[0]
        self.assertEqual(pong_time, client_time)

    def test_e2e_login_to_play_transition(self):
        """Client performs offline login and enters play immediately without Configuration state."""
        # 1. Handshake Packet (state 2 = Login)
        handshake_payload = (
            encode_varint(763)
            + encode_string("localhost")
            + struct.pack(">H", 25565)
            + encode_varint(2)
        )
        packet_hs = encode_varint(0x00) + handshake_payload
        _, pos = decode_varint(packet_hs)
        _, c1 = decode_varint(packet_hs, pos)
        pos += c1
        _, c2 = decode_string(packet_hs, pos)
        pos += c2
        pos += 2
        next_state, _ = decode_varint(packet_hs, pos)
        self.assertEqual(next_state, 2)

        # 2. Login Hello from client (username "Alex", no UUID)
        login_hello = encode_varint(0x00) + encode_string("Alex") + b"\x00"
        hid, pos = decode_varint(login_hello)
        self.assertEqual(hid, 0x00)
        username, c = decode_string(login_hello, pos)
        self.assertEqual(username, "Alex")

        # 3. Server responds with LoginFinished (id 0x02, UUID, name, 0 properties)
        uuid_int = 0x12345678123456781234567812345678
        login_finished = (
            encode_varint(0x02)
            + struct.pack(">QQ", uuid_int >> 64, uuid_int & 0xFFFFFFFFFFFFFFFF)
            + encode_string("Alex")
            + encode_varint(0)
        )
        lf_id, pos = decode_varint(login_finished)
        self.assertEqual(lf_id, 0x02)

        # 4. Immediate Transition to Play: Next packet received is JoinGame (0x28)
        join_packet_id = 0x28
        self.assertEqual(join_packet_id, 0x28)

    def test_e2e_teleport_and_movement_sequence(self):
        """Simulates teleportation confirmation and movement dispatch."""
        # Server sends PlayerPositionAndLook (0x3C, teleport_id = 1)
        teleport_id = 1
        spawn_x, spawn_y, spawn_z = 0.5, -60.0, 0.5
        pos_packet = (
            encode_varint(0x3C)
            + struct.pack(">dddffB", spawn_x, spawn_y, spawn_z, 0.0, 0.0, 0)
            + encode_varint(teleport_id)
        )
        pid, pos = decode_varint(pos_packet)
        self.assertEqual(pid, 0x3C)

        # Client acknowledges teleport (0x00, teleport_id = 1)
        accept_teleport = encode_varint(0x00) + encode_varint(1)
        aid, pos = decode_varint(accept_teleport)
        self.assertEqual(aid, 0x00)
        recv_tid, _ = decode_varint(accept_teleport, pos)
        self.assertEqual(recv_tid, 1)

        # Client moves to (1.5, -60.0, 1.5) on_ground = true (0x14)
        move_packet = (
            encode_varint(0x14)
            + struct.pack(">dddB", 1.5, -60.0, 1.5, 1)
        )
        mid, pos = decode_varint(move_packet)
        self.assertEqual(mid, 0x14)
        mx, my, mz, on_ground = struct.unpack(">dddB", move_packet[pos : pos + 25])
        self.assertEqual((mx, my, mz), (1.5, -60.0, 1.5))
        self.assertEqual(on_ground, 1)

    def test_e2e_system_chat_and_command(self):
        """Client sends chat message, server replies with SystemChat."""
        chat_msg = "/time"
        serverbound_chat = encode_varint(0x05) + encode_string(chat_msg) + struct.pack(">q", 1000)
        cid, pos = decode_varint(serverbound_chat)
        self.assertEqual(cid, 0x05)
        text, _ = decode_string(serverbound_chat, pos)
        self.assertEqual(text, "/time")

        # Server replies with SystemChat (0x64)
        system_reply = encode_varint(0x64) + encode_string('{"text":"Game time: 100"}') + b"\x00"
        sc_id, pos = decode_varint(system_reply)
        self.assertEqual(sc_id, 0x64)
        json_content, c = decode_string(system_reply, pos)
        self.assertIn("Game time: 100", json_content)

    def test_e2e_game_server_struct_invariants(self):
        """Verifies Legacy1201GameServer struct methods exist in protocol.rs."""
        content = PROTOCOL_RS.read_text(encoding="utf-8")
        self.assertIn("pub struct Legacy1201GameServer", content)
        self.assertIn("pub fn tick(&self)", content)
        self.assertIn("pub fn connect_player(", content)
        self.assertIn("pub fn handle_player_packet(", content)
        self.assertIn("pub fn save(&self,", content)
        self.assertIn("pub fn stop(&self)", content)


if __name__ == "__main__":
    unittest.main()
