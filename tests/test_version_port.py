import hashlib
import contextlib
import io
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

from tools import mojang_class_map, version_port as port


def jar(path, files):
    with zipfile.ZipFile(path, "w") as archive:
        for name, value in files.items():
            archive.writestr(name, value if isinstance(value, (str, bytes)) else json.dumps(value))
    return path


class DetectTests(unittest.TestCase):
    MANIFEST = {"latest": {"release": "26.4", "snapshot": "26.5-snapshot-1"}}

    def test_new_release_and_snapshot(self):
        self.assertEqual(port.news(self.MANIFEST, "26.3", {}), [("release", "26.4")])
        self.assertEqual(port.news(self.MANIFEST, "26.3", {}, snapshots=True),
                         [("release", "26.4"), ("snapshot", "26.5-snapshot-1")])

    def test_pinned_or_seen_is_not_news(self):
        self.assertEqual(port.news(self.MANIFEST, "26.4", {}), [])
        self.assertEqual(port.news(self.MANIFEST, "26.3", {"release": "26.4", "snapshot": "26.5-snapshot-1"},
                                   snapshots=True), [])

    def test_a_release_is_not_also_reported_as_the_latest_snapshot(self):
        manifest = {"latest": {"release": "26.4", "snapshot": "26.4"}}
        self.assertEqual(port.news(manifest, "26.3", {}, snapshots=True), [("release", "26.4")])

    def test_unsafe_versions_are_refused(self):
        for version in ["..", "26.4/../x", ""]:
            with self.assertRaises(ValueError):
                port.safe_version(version)


class ReportDiffTests(unittest.TestCase):
    def test_json_diff_paths(self):
        self.assertEqual(port.json_diff({"a": 1, "b": [1, 2]}, {"a": 2, "b": [1, 3], "c": True}),
                         [("/a", 1, 2), ("/b[1]", 2, 3), ("/c", None, True)])
        self.assertEqual(port.json_diff([1], [1, 2]), [("/", [1], [1, 2])])
        self.assertEqual(len(port.json_diff({str(i): i for i in range(9)}, {}, limit=3)), 3)

    def test_packets_added_removed_renumbered(self):
        old = {"play": {"serverbound": {"minecraft:swing": {"protocol_id": 0}, "minecraft:use": {"protocol_id": 1}}}}
        new = {"play": {"serverbound": {"minecraft:punch": {"protocol_id": 0}, "minecraft:use": {"protocol_id": 2},
                                        "minecraft:x": {"protocol_id": 1}}}}
        self.assertEqual(port.diff_packets(old, new), {"play/serverbound": {
            "added": {"minecraft:punch": 0, "minecraft:x": 1}, "removed": {"minecraft:swing": 0},
            "renumbered": {"minecraft:use": [1, 2]}, "count": [2, 3]}})
        self.assertEqual(port.diff_packets(old, old), {})

    def test_registries(self):
        def registry(*names, default=None):
            return {"default": default, "entries": {n: {"protocol_id": i} for i, n in enumerate(names)}}
        old = {"minecraft:item": registry("air", "stone"), "minecraft:gone": registry("a")}
        new = {"minecraft:item": registry("air", "dirt", "stone"), "minecraft:new": registry("b")}
        self.assertEqual(port.diff_registries(old, new), {
            "added": {"minecraft:new": ["b"]}, "removed": ["minecraft:gone"],
            "changed": {"minecraft:item": {"added": ["dirt"], "ids_shifted": 1}}})

    def test_blocks_state_ids_and_format(self):
        def block(first, *values, default=0, **extra):
            states = [{"id": first + i, "properties": {"p": v}} for i, v in enumerate(values)]
            states[default]["default"] = True
            return {"properties": {"p": list(values)}, "states": states, **extra}
        old = {"a": block(0, "x", "y"), "b": block(2, "x")}
        new = {"a": block(0, "x", "y", "z", default=1, definition={}),
               "b": block(3, "x", definition={}), "c": block(4, "x", definition={})}
        result = port.diff_blocks(old, new)
        self.assertEqual(result["added"], ["c"])
        self.assertEqual(result["format"], ["definition"])
        self.assertEqual(set(result["changed"]), {"a"})
        self.assertEqual(result["changed"]["a"]["states"], [2, 3])
        self.assertEqual(result["changed"]["a"]["default"], [{"p": "x"}, {"p": "y"}])
        self.assertEqual((result["state_ids_shifted"], result["first_shifted"]), (1, "b"))

    def test_items_by_component(self):
        old = {"stone": {"swing": {}, "name": "Stone"}, "dirt": {"swing": {}}}
        new = {"stone": {"attack": {}, "name": "Rock"}, "dirt": {"attack": {}}, "mud": {}}
        result = port.diff_items(old, new)
        self.assertEqual(result["added"], ["mud"])
        self.assertEqual(result["components"], {"attack": {"added": 2, "removed": 0, "changed": 0},
                                                "swing": {"added": 0, "removed": 2, "changed": 0},
                                                "name": {"added": 0, "removed": 0, "changed": 1}})

    def test_command_paths(self):
        def literal(**children):
            return {"type": "literal", "children": children} if children else {"type": "literal", "executable": True}
        old = {"type": "root", "children": {"data": literal(get=literal(block=literal(targetPos=literal())))}}
        new = {"type": "root", "children": {"data": literal(get=literal(block=literal(pos=literal()))),
                                            "compute": literal()}}
        result = port.diff_commands(old, new)
        self.assertEqual(result["added"], ["compute", "data get block pos"])
        self.assertEqual(result["removed"], ["data get block targetPos"])


class JarDiffTests(unittest.TestCase):
    def test_contents_lang_and_facts(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            common = {"data/minecraft/tags/block/logs.json": {"values": ["oak"]}}
            old = jar(root / "old.jar", {**common,
                                         "data/minecraft/recipe/a.json": {"result": "a", "count": 1},
                                         "data/minecraft/recipe/gone.json": {},
                                         "assets/minecraft/lang/en_us.json": {"k": "K", "same": "S"}})
            new = jar(root / "new.jar", {**common,
                                         "data/minecraft/recipe/a.json": {"result": "a", "count": 2},
                                         "data/minecraft/worldgen/biome/new.json": {},
                                         "assets/minecraft/lang/en_us.json": {"k": "K2", "same": "S", "n": "N"},
                                         "version.json": {"id": "26.4", "world_version": 5100,
                                                          "protocol_version": 778, "java_version": 25,
                                                          "pack_version": {"resource_major": 98, "resource_minor": 0,
                                                                           "data_major": 122, "data_minor": 0}}})
            data = port.diff_contents(old, new, "data/minecraft/")
            self.assertEqual(data["counts"], {"added": 1, "removed": 1, "changed": 1})
            self.assertEqual(data["kinds"]["recipe"], {"added": [], "removed": ["data/minecraft/recipe/gone.json"],
                                                       "changed": ["data/minecraft/recipe/a.json"]})
            self.assertEqual(data["kinds"]["worldgen/biome"]["added"], ["data/minecraft/worldgen/biome/new.json"])
            self.assertEqual(data["json"], {"data/minecraft/recipe/a.json": [("/count", 1, 2)]})
            self.assertEqual(port.diff_lang(old, new), {"added": ["n"], "removed": [], "changed": ["k"]})
            facts = port.jar_facts(new)
            self.assertEqual((facts["world_version"], facts["protocol_version"], facts["data_pack"]),
                             (5100, 778, "122.0"))
            self.assertEqual(port.diff_facts({"id": "a", "protocol_version": 777}, {"id": "b", "protocol_version": 778}),
                             {"protocol_version": [777, 778]})


class InspectLocalTests(unittest.TestCase):
    def test_client_mappings_are_checked_and_only_class_name_coverage_is_reported(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            client = jar(root / "client.jar", {"version.json": {
                "id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                "java_version": 17, "pack_version": {"resource": 15, "data": 15},
            }, "a.class": b"test", "x.class": b"test"})
            mapping = root / "client.txt"
            mapping.write_text("# test\nexample.World -> a:\n    int seed -> b\n", encoding="utf-8")
            metadata = root / "manifest.json"
            metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {
                "client": {"sha1": hashlib.sha1(client.read_bytes()).hexdigest()},
                "client_mappings": {"sha1": hashlib.sha1(mapping.read_bytes()).hexdigest()},
            }}))
            result = port.inspect_local("1.20.1", client, metadata, mapping)
            self.assertEqual(result["class_names"], {"mapped": 1, "unmapped": 1,
                                                    "scope": "class names only; bytecode is not remapped"})
            self.assertTrue(result["client_mappings_match_supplied_metadata"])
            mapping.write_text("changed", encoding="utf-8")
            with self.assertRaisesRegex(ValueError, "mappings do not match"):
                port.inspect_local("1.20.1", client, metadata, mapping)

    def test_class_map_rejects_ambiguous_or_invalid_class_names(self):
        self.assertEqual(mojang_class_map.read_classes([
            "# comment\n", "example.World -> a:\n", "    int seed -> b\n",
            "example.World$Inner -> a$a:\n",
        ]), {"a": "example/World", "a$a": "example/World$Inner"})
        for lines in (["example.One -> a:\n", "example.Two -> a:\n"],
                      ["example.One -> a:\n", "example.One -> b:\n"],
                      ["not a class mapping\n"], ["# no classes\n"]):
            with self.assertRaises(ValueError):
                mojang_class_map.read_classes(lines)

    def test_inspect_command_does_not_create_the_version_store(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            client = jar(root / "client.jar", {"version.json": {
                "id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                "java_version": 17, "pack_version": {"resource": 15, "data": 15},
            }})
            metadata = root / "manifest.json"
            metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {
                "client": {"sha1": hashlib.sha1(client.read_bytes()).hexdigest()}}}))
            store = root / "uncreated-store"
            out = io.StringIO()
            argv = ["version_port.py", "inspect", "1.20.1", "--client", str(client),
                    "--metadata", str(metadata)]
            with patch.object(port, "STORE", store), patch("sys.argv", argv), contextlib.redirect_stdout(out):
                self.assertEqual(port.main(), 0)
            self.assertEqual(json.loads(out.getvalue())["version"]["protocol_version"], 763)
            self.assertFalse(store.exists())

    def test_1_20_1_is_checked_and_reported_without_enabling_the_port(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            client = jar(root / "client.jar", {"version.json": {
                "id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                "java_version": 17, "stable": True, "pack_version": {"resource": 15, "data": 15},
            }})
            original = client.read_bytes()
            metadata = root / "1.20.1.json"
            metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {
                "client": {"sha1": hashlib.sha1(original).hexdigest()}, "client_mappings": {},
            }}))
            before = metadata.read_bytes()
            result = port.inspect_local("1.20.1", client, metadata)
            self.assertEqual(result["version"], {"id": "1.20.1", "world_version": 3465,
                             "protocol_version": 763, "resource_pack": "15.0", "data_pack": "15.0",
                             "java_version": 17, "stable": True})
            self.assertTrue(result["needs_class_remapping"])
            self.assertTrue(result["client_matches_supplied_metadata"])
            self.assertFalse(result["metadata_authenticity_verified"])
            self.assertEqual(client.read_bytes(), original)
            self.assertEqual(metadata.read_bytes(), before)
            self.assertEqual(sorted(p.name for p in root.iterdir()), ["1.20.1.json", "client.jar"])

    def test_refuses_mismatched_metadata_and_client(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            client = jar(root / "client.jar", {"version.json": {"id": "1.20.1"}})
            metadata = root / "manifest.json"
            metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {"client": {"sha1": "0" * 40}}}))
            with self.assertRaisesRegex(ValueError, "does not match"):
                port.inspect_local("1.20.1", client, metadata)
            with self.assertRaisesRegex(ValueError, "metadata is for"):
                port.inspect_local("26.3", client, metadata)
            with self.assertRaisesRegex(ValueError, "Unsafe version"):
                port.inspect_local("../1.20.1", client, metadata)

            forged = {"id": "1.20.1", "downloads": {"client": {
                "sha1": hashlib.sha1(client.read_bytes()).hexdigest()}}}
            metadata.write_text(json.dumps(forged))
            with self.assertRaisesRegex(ValueError, "client jar says"):
                other = jar(root / "other.jar", {"version.json": {"id": "26.3", "world_version": 5023,
                            "protocol_version": 777, "java_version": 25, "pack_version": 1}})
                forged["downloads"]["client"]["sha1"] = hashlib.sha1(other.read_bytes()).hexdigest()
                metadata.write_text(json.dumps(forged))
                port.inspect_local("1.20.1", other, metadata)


class RemapClientTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        repo = self.root / "source-repository"
        repo.mkdir()
        root_patch = patch.object(port.lab, "ROOT", repo)
        root_patch.start()
        self.addCleanup(root_patch.stop)
        self.info = {"id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                     "java_version": 17, "pack_version": {"resource": 15, "data": 15}}
        self.client = jar(self.root / "client.jar", {"version.json": self.info, "a.class": b"test"})
        self.mapping = self.root / "mapping.txt"
        self.mapping.write_text("example.World -> a:\n", encoding="utf-8")
        self.metadata = self.root / "metadata.json"
        self.metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {
            "client": {"sha1": hashlib.sha1(self.client.read_bytes()).hexdigest()},
            "client_mappings": {"sha1": hashlib.sha1(self.mapping.read_bytes()).hexdigest()},
        }}))
        self.remapper = self.root / "tool.jar"
        self.remapper.write_bytes(b"fixture")
        self.output = self.root / "named.jar"

    def run_with(self, fake):
        with patch.object(port.subprocess, "run", side_effect=fake):
            return port.remap_local_client("1.20.1", self.client, self.metadata, self.mapping,
                                           self.remapper, self.output, java="java")

    def test_verified_input_is_remapped_into_a_new_separate_file(self):
        before = self.client.read_bytes()

        def fake(args, **kwargs):
            self.assertIn("--reverse", args)
            dest = Path(args[args.index("--output") + 1])
            jar(dest, {"version.json": self.info, "example/World.class": b"named"})
            return SimpleNamespace(returncode=0, stdout="ok", stderr="")

        result = self.run_with(fake)
        self.assertEqual(result["class_names"], 1)
        self.assertIn("not a playable", result["note"])
        self.assertEqual(self.client.read_bytes(), before)
        with zipfile.ZipFile(self.output) as archive:
            self.assertIn("example/World.class", archive.namelist())
        with self.assertRaises(FileExistsError):
            self.run_with(fake)

    def test_failed_or_incomplete_remap_never_creates_an_output(self):
        failure = lambda args, **kwargs: SimpleNamespace(returncode=1, stdout="", stderr="failed")
        with self.assertRaisesRegex(RuntimeError, "remapping tool failed"):
            self.run_with(failure)
        self.assertFalse(self.output.exists())

        def incomplete(args, **kwargs):
            jar(Path(args[args.index("--output") + 1]), {"version.json": self.info})
            return SimpleNamespace(returncode=0, stdout="ok", stderr="")

        with self.assertRaisesRegex(ValueError, "missing 1 expected class"):
            self.run_with(incomplete)
        self.assertFalse(self.output.exists())

    def test_refuses_output_in_the_source_repository(self):
        output = port.lab.ROOT / "never-create-named.jar"
        with self.assertRaisesRegex(ValueError, "outside the source repository"):
            port.remap_local_client("1.20.1", self.client, self.metadata, self.mapping,
                                    self.remapper, output, java="java")
        self.assertFalse(output.exists())


class LocalReportsTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        repo = self.root / "source-repository"
        repo.mkdir()
        root_patch = patch.object(port.lab, "ROOT", repo)
        root_patch.start()
        self.addCleanup(root_patch.stop)
        self.server = jar(self.root / "server.jar", {"version.json": {
            "id": "1.20.1", "world_version": 3465, "protocol_version": 763,
            "java_version": 17, "pack_version": {"resource": 15, "data": 15},
        }})
        self.metadata = self.root / "manifest.json"
        self.metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {"server": {
            "sha1": hashlib.sha1(self.server.read_bytes()).hexdigest()}}}))
        self.output = self.root / "reports"

    def run_with(self, fake):
        with patch.object(port.subprocess, "run", side_effect=fake):
            return port.local_reports("1.20.1", self.server, self.metadata, self.output, java="java")

    def test_local_server_reports_go_to_a_new_separate_folder(self):
        before = self.server.read_bytes()

        def fake(command, **kwargs):
            self.assertIn("-DbundlerMainClass=net.minecraft.data.Main", command)
            target = Path(command[command.index("--output") + 1]) / "reports"
            target.mkdir(parents=True)
            for name in ("blocks.json", "registries.json", "commands.json"):
                (target / name).write_text('{"test":{}}')
            return SimpleNamespace(returncode=0, stdout="ok", stderr="")

        result = self.run_with(fake)
        self.assertEqual(result["version"], "1.20.1")
        self.assertFalse(result["metadata_authenticity_verified"])
        self.assertEqual(result["reports"], ["blocks.json", "commands.json", "registries.json"])
        self.assertEqual(self.server.read_bytes(), before)
        self.assertTrue((self.output / "blocks.json").is_file())
        with self.assertRaises(FileExistsError):
            self.run_with(fake)

    def test_failed_or_incomplete_generation_leaves_no_output(self):
        failure = lambda command, **kwargs: SimpleNamespace(returncode=1, stdout="", stderr="failed")
        with self.assertRaisesRegex(RuntimeError, "data generator failed"):
            self.run_with(failure)
        self.assertFalse(self.output.exists())

        def incomplete(command, **kwargs):
            target = Path(command[command.index("--output") + 1]) / "reports"
            target.mkdir(parents=True)
            (target / "blocks.json").write_text("{}")
            return SimpleNamespace(returncode=0, stdout="ok", stderr="")

        with self.assertRaisesRegex(ValueError, "registries.json"):
            self.run_with(incomplete)
        self.assertFalse(self.output.exists())

    def test_wrong_server_is_rejected_before_execution(self):
        self.server.write_bytes(b"wrong server")
        with patch.object(port.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "does not match"):
                port.local_reports("1.20.1", self.server, self.metadata, self.output, java="java")
            run.assert_not_called()
        self.assertFalse(self.output.exists())


class RemapServerTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        repo = self.root / "source-repository"
        repo.mkdir()
        root_patch = patch.object(port.lab, "ROOT", repo)
        root_patch.start()
        self.addCleanup(root_patch.stop)
        self.info = {"id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                     "java_version": 17, "pack_version": {"resource": 15, "data": 15}}
        inner = jar(self.root / "inner.jar", {"version.json": self.info, "a.class": b"test"}).read_bytes()
        self.server = jar(self.root / "server.jar", {
            "version.json": self.info,
            "META-INF/versions.list": f'{hashlib.sha256(inner).hexdigest()}\t1.20.1\t1.20.1/server.jar\n',
            "META-INF/versions/1.20.1/server.jar": inner,
        })
        self.mapping = self.root / "mapping.txt"
        self.mapping.write_text("example.World -> a:\n", encoding="utf-8")
        self.metadata = self.root / "metadata.json"
        self.metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {
            "server": {"sha1": hashlib.sha1(self.server.read_bytes()).hexdigest()},
            "server_mappings": {"sha1": hashlib.sha1(self.mapping.read_bytes()).hexdigest()},
        }}))
        self.remapper = self.root / "tool.jar"
        self.remapper.write_bytes(b"fixture")
        self.output = self.root / "named-server.jar"

    def test_bundled_game_is_verified_and_remapped_without_modifying_server(self):
        before = self.server.read_bytes()

        def fake(command, **kwargs):
            source = Path(command[command.index("--input") + 1])
            with zipfile.ZipFile(source) as archive:
                self.assertIn("a.class", archive.namelist())
            jar(Path(command[command.index("--output") + 1]),
                {"version.json": self.info, "example/World.class": b"named"})
            return SimpleNamespace(returncode=0, stdout="ok", stderr="")

        with patch.object(port.subprocess, "run", side_effect=fake):
            result = port.remap_local_server("1.20.1", self.server, self.metadata, self.mapping,
                                             self.remapper, self.output, java="java")
        self.assertEqual(result["class_names"], 1)
        self.assertEqual(self.server.read_bytes(), before)
        self.assertTrue(self.output.is_file())
        self.assertFalse(any(p.name.startswith("horizon-1201-server-") for p in self.root.iterdir()))

    def test_wrong_mapping_is_rejected_before_unpack_or_run(self):
        self.mapping.write_text("different mapping", encoding="utf-8")
        with patch.object(port.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "server_mappings"):
                port.remap_local_server("1.20.1", self.server, self.metadata, self.mapping,
                                        self.remapper, self.output, java="java")
            run.assert_not_called()
        self.assertFalse(self.output.exists())


class CompareLocalReportsTests(unittest.TestCase):
    def test_compares_available_data_without_claiming_missing_protocol_support(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            old, new = root / "old", root / "new"
            old.mkdir()
            new.mkdir()
            stone = {"states": [{"id": 1, "default": True}]}
            for path, blocks in [(old, {"minecraft:stone": stone}),
                                 (new, {"minecraft:stone": stone, "minecraft:dirt": stone})]:
                (path / "blocks.json").write_text(json.dumps(blocks))
                (path / "registries.json").write_text('{}')
                (path / "commands.json").write_text('{"type":"root","children":{}}')
            compared = port.compare_local_reports(old, new)
            self.assertEqual(compared["blocks"]["added"], ["minecraft:dirt"])
            self.assertIn("network packets", compared["not_compared"])
            self.assertNotIn("packets", compared)
            self.assertIn("not a playable", compared["note"])
            output = io.StringIO()
            with patch("sys.argv", ["version_port.py", "compare-reports", "--old", str(old),
                                    "--new", str(new)]), contextlib.redirect_stdout(output):
                self.assertEqual(port.main(), 0)
            self.assertEqual(json.loads(output.getvalue()), compared)

    def test_missing_data_is_an_error_not_an_empty_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with self.assertRaises(FileNotFoundError):
                port.compare_local_reports(root, root)


class LegacyPacketProbeTests(unittest.TestCase):
    LINES = [
        'HANDSHAKING\tSERVERBOUND\t0\tnet.minecraft.network.protocol.handshake.ClientIntentionPacket\n',
        'STATUS\tCLIENTBOUND\t0\tnet.minecraft.network.protocol.status.ClientboundStatusResponsePacket\n',
        'LOGIN\tCLIENTBOUND\t2\tnet.minecraft.network.protocol.login.ClientboundGameProfilePacket\n',
        'PLAY\tCLIENTBOUND\t1\tnet.minecraft.network.protocol.game.ClientboundAddEntityPacket\n',
    ]

    def test_packet_classes_and_ids_are_not_described_as_full_compatibility(self):
        records = port.legacy_packet_classes(self.LINES)
        self.assertEqual(len(records), 4)
        self.assertIn({'phase': 'login', 'direction': 'clientbound', 'id': 2,
                       'class': 'net.minecraft.network.protocol.login.ClientboundGameProfilePacket'}, records)
        for invalid in (self.LINES[:-1], self.LINES + [self.LINES[0]],
                        self.LINES + ['CONFIGURATION\tCLIENTBOUND\t0\tnet.minecraft.network.protocol.Fake\n'],
                        self.LINES + ['PLAY\tSERVERBOUND\t-1\tnet.minecraft.network.protocol.Fake\n']):
            with self.assertRaises(ValueError):
                port.legacy_packet_classes(invalid)

    def test_export_checks_inputs_and_writes_only_a_new_local_report(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            (repo / "tools").mkdir(parents=True)
            (repo / "tools/Legacy1201PacketProbe.java").write_text("fixture")
            info = {"id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                    "java_version": 17, "pack_version": {"resource": 15, "data": 15}}
            server = jar(root / "server.jar", {"version.json": info})
            named = jar(root / "named.jar", {"version.json": info})
            metadata = root / "manifest.json"
            metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {"server": {
                "sha1": hashlib.sha1(server.read_bytes()).hexdigest()}}}))
            output = root / "packets.json"

            def fake(command, **kwargs):
                if "Legacy1201PacketProbe" in command:
                    Path(command[-1]).write_text(''.join(self.LINES), encoding="utf-8")
                return SimpleNamespace(returncode=0, stdout="ok", stderr="")

            with patch.object(port.lab, "ROOT", repo), \
                    patch.object(port.lab, "unpack_server", return_value=(server, [])), \
                    patch.object(port.subprocess, "run", side_effect=fake):
                result = port.export_legacy_packets(server, named, metadata, output, java="java")
                self.assertEqual(result["packets"], 4)
                saved = json.loads(output.read_text())
                self.assertEqual(saved["protocol"], 763)
                self.assertFalse(saved["metadata_authenticity_verified"])
                self.assertFalse(saved["named_server_provenance_verified"])
                self.assertIn("not verified", saved["note"])
                with self.assertRaises(FileExistsError):
                    port.export_legacy_packets(server, named, metadata, output, java="java")


class LegacyBlockFactsTests(unittest.TestCase):
    BLOCKS = {
        "minecraft:air": {"states": [{"id": 0, "default": True}]},
        "minecraft:test": {"properties": {"powered": ["true", "false"]}, "states": [
            {"id": 1, "properties": {"powered": "true"}},
            {"id": 2, "properties": {"powered": "false"}, "default": True},
        ]},
    }
    FACTS = {
        "states": [
            ["minecraft:air", True, False, "Block{minecraft:air}", {"id": "minecraft:air"}],
            ["minecraft:test", False, False, "on", {"id": "minecraft:test", "properties": {"powered": "true"}}],
            ["minecraft:test", False, True, "off", {"id": "minecraft:test", "properties": {"powered": "false"}}],
        ],
        "defaults": {"minecraft:air": 0, "minecraft:test": 2},
    }

    def test_fact_identities_and_defaults_must_match_report(self):
        self.assertEqual(port.validate_legacy_block_facts(self.FACTS, self.BLOCKS), (3, 2))
        bad = json.loads(json.dumps(self.FACTS))
        bad["states"][1][4]["properties"]["powered"] = "false"
        with self.assertRaisesRegex(ValueError, "does not match"):
            port.validate_legacy_block_facts(bad, self.BLOCKS)
        bad = json.loads(json.dumps(self.FACTS))
        bad["defaults"]["minecraft:test"] = 1
        with self.assertRaisesRegex(ValueError, "Default"):
            port.validate_legacy_block_facts(bad, self.BLOCKS)
        bad = json.loads(json.dumps(self.BLOCKS))
        bad["minecraft:test"]["states"][1]["id"] = 1
        with self.assertRaisesRegex(ValueError, "duplicate"):
            port.validate_legacy_block_facts(self.FACTS, bad)

    def test_export_only_publishes_checked_facts_to_new_external_file(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            (repo / "tools").mkdir(parents=True)
            (repo / "tools/Legacy1201BlockFactsExporter.java").write_text("fixture")
            info = {"id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                    "java_version": 17, "pack_version": {"resource": 15, "data": 15}}
            server = jar(root / "server.jar", {"version.json": info})
            named = jar(root / "named.jar", {"version.json": info})
            metadata = root / "manifest.json"
            metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {"server": {
                "sha1": hashlib.sha1(server.read_bytes()).hexdigest()}}}))
            report = root / "blocks.json"
            report.write_text(json.dumps(self.BLOCKS))
            output = root / "facts.json"

            def fake(command, **kwargs):
                if "Legacy1201BlockFactsExporter" in command:
                    Path(command[-1]).write_text(json.dumps(self.FACTS))
                return SimpleNamespace(returncode=0, stdout="", stderr="")

            with patch.object(port.lab, "ROOT", repo), \
                    patch.object(port.lab, "unpack_server", return_value=(server, [])), \
                    patch.object(port.subprocess, "run", side_effect=fake):
                result = port.export_legacy_block_facts(server, named, metadata, report, output, java="java")
                self.assertEqual((result["states"], result["blocks"]), (3, 2))
                self.assertEqual(json.loads(output.read_text()), self.FACTS)
                self.assertFalse(result["named_server_provenance_verified"])
                with self.assertRaises(FileExistsError):
                    port.export_legacy_block_facts(server, named, metadata, report, output, java="java")
                with self.assertRaises(ValueError):
                    port.export_legacy_block_facts(server, named, metadata, report, repo / "facts.json", java="java")
                bad = json.loads(json.dumps(self.BLOCKS))
                bad["minecraft:test"]["states"][0]["properties"]["powered"] = "false"
                report.write_text(json.dumps(bad))
                rejected = root / "rejected.json"
                with self.assertRaisesRegex(ValueError, "does not match"):
                    port.export_legacy_block_facts(server, named, metadata, report, rejected, java="java")
                self.assertFalse(rejected.exists())


class LegacyGameplayFactsTests(unittest.TestCase):
    @staticmethod
    def state(index):
        flags = ("skylight_down", "shape_occlusion", "can_occlude", "solid_render", "requires_tool",
                 "replaceable", "random_ticks", "block_entity", "liquid", "ignited_by_lava", "offset",
                 "full_collision", "outline_offset")
        return {**dict.fromkeys(flags, False), "id": index, "hardness": 1.5, "friction": 0.6,
                "speed_factor": 1.0, "jump_factor": 1.0, "explosion_resistance": 6.0,
                "emission": 0, "dampening": 15, "render_shape": "MODEL", "push": "NORMAL",
                "item": "minecraft:stone", "outline": [], "outline_edges": [], "sound": {}}

    def test_requires_complete_states_in_registry_order(self):
        data = {"states": [self.state(0), self.state(1)]}
        self.assertEqual(port.validate_legacy_gameplay_facts(data, 2), 2)
        with self.assertRaisesRegex(ValueError, "count"):
            port.validate_legacy_gameplay_facts(data, 3)
        bad = {"states": [self.state(1), self.state(0)]}
        with self.assertRaisesRegex(ValueError, "wrong ID"):
            port.validate_legacy_gameplay_facts(bad, 2)
        bad = {"states": [self.state(0)]}
        bad["states"][0]["hardness"] = float("nan")
        with self.assertRaisesRegex(ValueError, "incomplete"):
            port.validate_legacy_gameplay_facts(bad, 1)

    def test_only_valid_gameplay_facts_are_published(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            (repo / "tools").mkdir(parents=True)
            (repo / "tools/Legacy1201GameplayFactsExporter.java").write_text("fixture")
            info = {"id": "1.20.1", "world_version": 3465, "protocol_version": 763,
                    "java_version": 17, "pack_version": {"resource": 15, "data": 15}}
            server = jar(root / "server.jar", {"version.json": info})
            named = jar(root / "named.jar", {"version.json": info})
            metadata = root / "manifest.json"
            metadata.write_text(json.dumps({"id": "1.20.1", "downloads": {"server": {
                "sha1": hashlib.sha1(server.read_bytes()).hexdigest()}}}))
            blocks, facts = root / "blocks.json", root / "facts.json"
            blocks.write_text(json.dumps(LegacyBlockFactsTests.BLOCKS))
            facts.write_text(json.dumps(LegacyBlockFactsTests.FACTS))
            output = root / "gameplay.json"
            generated = {"states": [self.state(0), self.state(1), self.state(2)]}

            def fake(command, **kwargs):
                if "Legacy1201GameplayFactsExporter" in command:
                    Path(command[-1]).write_text(json.dumps(generated))
                return SimpleNamespace(returncode=0, stdout="", stderr="")

            with patch.object(port.lab, "ROOT", repo), \
                    patch.object(port.lab, "unpack_server", return_value=(server, [])), \
                    patch.object(port.subprocess, "run", side_effect=fake):
                result = port.export_legacy_gameplay_facts(server, named, metadata, blocks, facts, output, java="java")
                self.assertEqual(result["states"], 3)
                self.assertEqual(json.loads(output.read_text()), generated)
                with self.assertRaises(FileExistsError):
                    port.export_legacy_gameplay_facts(server, named, metadata, blocks, facts, output, java="java")
                generated["states"][2]["id"] = 0
                rejected = root / "bad-gameplay.json"
                with self.assertRaisesRegex(ValueError, "wrong ID"):
                    port.export_legacy_gameplay_facts(server, named, metadata, blocks, facts, rejected, java="java")
                self.assertFalse(rejected.exists())


class LegacyBehaviourFactsTests(unittest.TestCase):
    @staticmethod
    def behaviour():
        block = lambda name: {"name": name, "class": ["Block"], "interfaces": [],
                              "fields": {}, "dynamic_shape": False, "item": name}
        return {"blocks": [block("minecraft:air"), block("minecraft:test")],
                "states": [[0, 0, 0, 0, False, 0, 0, False, "HARP"] for _ in range(3)],
                "fluids": ["minecraft:empty", "minecraft:flowing_water", "minecraft:water",
                           "minecraft:flowing_lava", "minecraft:lava"], "items": {}, "fire": {}}

    def test_requires_matching_registry_order_and_no_exporter_errors(self):
        data = self.behaviour()
        self.assertEqual(port.validate_legacy_behaviour_facts(data, LegacyBlockFactsTests.FACTS), (3, 2))
        data["blocks"].reverse()
        with self.assertRaisesRegex(ValueError, "order"):
            port.validate_legacy_behaviour_facts(data, LegacyBlockFactsTests.FACTS)
        data = self.behaviour()
        data["states"][1][0] |= 1 << 30
        with self.assertRaisesRegex(ValueError, "exporter error"):
            port.validate_legacy_behaviour_facts(data, LegacyBlockFactsTests.FACTS)
        data = self.behaviour()
        data["states"][0][7] = True
        with self.assertRaisesRegex(ValueError, "exporter error"):
            port.validate_legacy_behaviour_facts(data, LegacyBlockFactsTests.FACTS)
        data = self.behaviour()
        data["blocks"][1]["fields"]["nested"] = {"key": "<error>"}
        with self.assertRaisesRegex(ValueError, "exporter error"):
            port.validate_legacy_behaviour_facts(data, LegacyBlockFactsTests.FACTS)


class LegacyBlockPreparationTests(unittest.TestCase):
    def test_prepares_three_files_without_overwriting_or_publishing_partial_output(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            repo.mkdir()
            server, metadata, mappings, remapper = [root / name for name in ("server", "metadata", "maps", "remapper")]
            output = root / "prepared"

            def fake_remap(version, source, meta, maps, tool, named, java):
                named.write_text("private temporary server")

            def fake_reports(version, source, meta, out, java):
                out.mkdir()
                (out / "blocks.json").write_text("{}")

            def fake_block(source, named, meta, report, out, java):
                out.write_text("block data")
                return {"states": 3, "blocks": 2}

            def fake_game(source, named, meta, report, facts, out, java):
                out.write_text("gameplay data")

            def fake_behaviour(source, named, meta, report, facts, out, java):
                out.write_text("behaviour data")

            with patch.object(port.lab, "ROOT", repo), \
                    patch.object(port, "remap_local_server", side_effect=fake_remap), \
                    patch.object(port, "local_reports", side_effect=fake_reports), \
                    patch.object(port, "export_legacy_block_facts", side_effect=fake_block), \
                    patch.object(port, "export_legacy_gameplay_facts", side_effect=fake_game), \
                    patch.object(port, "export_legacy_behaviour_facts", side_effect=fake_behaviour):
                result = port.prepare_legacy_block_data(server, metadata, mappings, remapper, output, java="java")
                self.assertEqual((result["states"], result["blocks"]), (3, 2))
                self.assertEqual(sorted(path.name for path in output.iterdir()),
                                 ["block-behaviours.json", "facts.json", "gameplay-facts.json"])
                with self.assertRaises(FileExistsError):
                    port.prepare_legacy_block_data(server, metadata, mappings, remapper, output, java="java")
                with patch.object(port, "export_legacy_gameplay_facts", side_effect=RuntimeError("failure")):
                    rejected = root / "rejected"
                    with self.assertRaises(RuntimeError):
                        port.prepare_legacy_block_data(server, metadata, mappings, remapper, rejected, java="java")
                    self.assertFalse(rejected.exists())


class DynamicStatesTests(unittest.TestCase):
    def test_rejects_corruption_and_duplicate_ids(self):
        classes = [("piston.MovingPistonBlock", "block_entity", 12),
                   ("ShulkerBoxBlock", "block_entity", 102),
                   ("BambooStalkBlock", "position_offset", 12),
                   ("PointedDripstoneBlock", "position_offset", 20),
                   ("ScaffoldingBlock", "collision_context", 32),
                   ("PowderSnowBlock", "collision_context", 1)]
        rows, blocks = [], {}
        for cls, dependency, count in classes:
            name = "minecraft:" + cls.rsplit(".", 1)[-1].lower()
            states = []
            for i in range(count):
                index = len(rows)
                properties = {"test": str(i)}
                states.append({"id": index, "properties": properties})
                rows.append({"id": index, "block": name, "properties": properties.copy(),
                             "class": "net.minecraft.world.level.block." + cls,
                             "shape_dependency": dependency, "dynamic_shape": True})
            blocks[name] = {"states": states}
        self.assertEqual(sum(port.validate_legacy_dynamic_states(rows, blocks).values()), 179)
        for key, value in [("id", rows[1]["id"]), ("properties", {"test": "wrong"}),
                           ("dynamic_shape", False), ("shape_dependency", "other")]:
            changed = [dict(row) for row in rows]
            changed[0][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                port.validate_legacy_dynamic_states(changed, blocks)
        with self.assertRaises(ValueError):
            port.validate_legacy_dynamic_states(rows[:-1], blocks)


class LegacyCollisionExportTests(unittest.TestCase):
    def test_publishes_only_after_every_comparison_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            repo.mkdir()
            server, named, metadata, blocks, facts = [root / name for name in ("server", "named", "meta", "blocks", "facts")]
            blocks.write_text("{}")
            facts.write_text("{}")
            version = {"java_version": 17}
            cases = {
                "Legacy1201CachedCollisionOracle": ("legacy_cached_collision_probe", 23956),
                "Legacy1201DynamicCollisionOracle": ("legacy_dynamic_collision_probe", 1074),
                "Legacy1201EntityCollisionOracle": ("legacy_entity_collision_probe", 3270),
                "Legacy1201OutlineOracle": ("legacy_outline_probe", 3752),
                "Legacy1201LiquidCollisionOracle": ("legacy_liquid_collision_probe", 768),
                "Legacy1201ContextCollisionOracle": ("legacy_context_collision_probe", 632),
                "Legacy1201AllContextOracle": ("legacy_all_context_probe", 48270),
            }
            rows = {example: [f"{example} {i}" for i in range(count)] for example, count in cases.values()}
            def exporter(_server, _named, _java, scratch, name, filename):
                out = scratch / filename
                if name == "Legacy1201CollisionExporter":
                    out.write_bytes(b"checked collision")
                else:
                    out.write_text("\n".join(rows[cases[name][0]]) + "\n")
                return out
            def cargo(command, **_kwargs):
                example = command[command.index("--example") + 1]
                output = ("states=24135 cached=23956 uncached=179 shapes=4294\n"
                          if example == "legacy_collision_probe" else "\n".join(rows[example]) + "\n")
                return SimpleNamespace(returncode=0, stdout=output, stderr="")
            output = root / "collision.nbt"
            with patch.object(port.lab, "ROOT", repo), \
                    patch.object(port, "verified_local_server", return_value=({}, version)), \
                    patch.object(port, "jar_facts", return_value=version), \
                    patch.object(port, "validate_legacy_block_facts", return_value=(24135, 1003)), \
                    patch.object(port, "run_legacy_facts_exporter", side_effect=exporter), \
                    patch.object(port.subprocess, "run", side_effect=cargo):
                result = port.export_legacy_collision(server, named, metadata, blocks, facts, output, java="java")
                self.assertEqual(result["matched_fluid_cases"], 768)
                self.assertEqual(output.read_bytes(), b"checked collision")
                with self.assertRaises(FileExistsError):
                    port.export_legacy_collision(server, named, metadata, blocks, facts, output, java="java")
                rows["legacy_outline_probe"][0] = "different"
                rejected = root / "rejected.nbt"
                with self.assertRaisesRegex(ValueError, "Outline comparison"):
                    # Change only Rust's response; the Java reference stays intact.
                    def bad_cargo(command, **kwargs):
                        result = cargo(command, **kwargs)
                        if "legacy_outline_probe" in command:
                            result.stdout = result.stdout.replace("different", "mismatch", 1)
                        return result
                    with patch.object(port.subprocess, "run", side_effect=bad_cargo):
                        port.export_legacy_collision(server, named, metadata, blocks, facts, rejected, java="java")
                self.assertFalse(rejected.exists())


class LegacyHeightmapExportTests(unittest.TestCase):
    def test_publishes_only_after_chunk_parity_and_never_overwrites(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            repo = root / "repo"
            repo.mkdir()
            server, named, metadata, blocks, facts = [root / name for name in ("server", "named", "meta", "blocks", "facts")]
            blocks.write_text("{}")
            facts.write_text("{}")
            version = {"java_version": 17}
            rows = [f"{i} 1 0 0 0 0 0 0 0 0 0 0" for i in range(24135)]
            column_rows = [f"column {i}" for i in range(36)]
            def exporter(_server, _named, _java, scratch, name, filename):
                out = scratch / filename
                if name == "Legacy1201HeightmapExporter":
                    out.write_bytes(b"checked heightmaps")
                else:
                    selected = column_rows if name == "Legacy1201HeightmapColumnsOracle" else rows
                    out.write_text("\n".join(selected) + "\n")
                return out
            def cargo(command, **_kwargs):
                selected = column_rows if "legacy_heightmap_columns_probe" in command else rows
                return SimpleNamespace(returncode=0, stdout="\n".join(selected) + "\n", stderr="")
            output = root / "heightmap.nbt"
            with patch.object(port.lab, "ROOT", repo), \
                    patch.object(port, "verified_local_server", return_value=({}, version)), \
                    patch.object(port, "jar_facts", return_value=version), \
                    patch.object(port, "validate_legacy_block_facts", return_value=(24135, 1003)), \
                    patch.object(port, "run_legacy_facts_exporter", side_effect=exporter), \
                    patch.object(port.subprocess, "run", side_effect=cargo):
                result = port.export_legacy_heightmap(server, named, metadata, blocks, facts, output, java="java")
                self.assertEqual(result["matched_terrain_height_updates"], 144810)
                self.assertEqual(result["matched_column_checkpoints"], 36)
                self.assertEqual(output.read_bytes(), b"checked heightmaps")
                with self.assertRaises(FileExistsError):
                    port.export_legacy_heightmap(server, named, metadata, blocks, facts, output, java="java")
                rejected = root / "rejected.nbt"
                def bad_cargo(command, **kwargs):
                    result = cargo(command, **kwargs)
                    result.stdout = result.stdout.replace("5 1 0 0 0 0", "5 0 0 0 0 0", 1)
                    return result
                with patch.object(port.subprocess, "run", side_effect=bad_cargo), \
                        self.assertRaisesRegex(ValueError, "Heightmap chunk comparison"):
                    port.export_legacy_heightmap(server, named, metadata, blocks, facts, rejected, java="java")
                self.assertFalse(rejected.exists())
                def bad_column(command, **kwargs):
                    result = cargo(command, **kwargs)
                    if "legacy_heightmap_columns_probe" in command:
                        result.stdout = result.stdout.replace("column 5", "mismatch", 1)
                    return result
                with patch.object(port.subprocess, "run", side_effect=bad_column), \
                        self.assertRaisesRegex(ValueError, "legacy_heightmap_columns_probe"):
                    port.export_legacy_heightmap(server, named, metadata, blocks, facts, rejected, java="java")
                self.assertFalse(rejected.exists())


class SelectedJavaTests(unittest.TestCase):
    def test_relative_executable_survives_changed_working_directory(self):
        self.assertEqual(port.selected_java(Path("jdk/bin/java"), 17), str(Path("jdk/bin/java").resolve()))
        self.assertEqual(port.selected_java("java", 17), "java")
        self.assertEqual(port.selected_java(Path("/opt/jdk/bin/java"), 17), str(Path("/opt/jdk/bin/java").resolve()))


class GcTests(unittest.TestCase):
    def test_keeps_pinned_newest_two_and_their_diffs(self):
        entries = [("26.1", 1, True), ("26.2", 2, True), ("26.3", 3, True), ("26.4-snapshot-1", 4, True),
                   ("26.4-snapshot-2", 5, True), ("diff-26.2-26.3", 6, False),
                   ("diff-26.3-26.4-snapshot-2", 7, False), (".datagen-x", 8, False), ("detect-state.json", 9, False)]
        self.assertEqual(port.to_remove(entries, "26.3", set()),
                         [".datagen-x", "26.1", "26.2", "diff-26.2-26.3"])
        self.assertEqual(port.to_remove(entries, "26.3", {"26.1"}),
                         [".datagen-x", "26.2", "diff-26.2-26.3"])


if __name__ == "__main__":
    unittest.main()
