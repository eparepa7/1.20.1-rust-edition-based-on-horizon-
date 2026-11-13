import contextlib
import io
import json
from pathlib import Path
import tempfile
import unittest

from tools import inspect_legacy_generator as audit


class GeneratorInventoryTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.write("block_tags", {name: {"values": []} for name in audit.MOTION_TAGS})
        self.write("carvers", {})
        self.write("biomes", {"test:empty": {"carvers": []}})

    def write(self, name, entries):
        (self.root / f"{name}.json").write_text(
            json.dumps({key: json.dumps(value) for key, value in entries.items()}), encoding="utf-8")

    def codes(self):
        return [entry["code"] for entry in audit.inspect(self.root)["issues"]]

    def test_empty_sets_are_not_a_parity_verdict(self):
        before = {p.name: p.read_bytes() for p in self.root.iterdir()}
        report = audit.inspect(self.root)
        self.assertEqual(report["issues"], [])
        self.assertEqual(report["runtime_parity"], "not_run")
        self.assertEqual(report["counts"], {"block_tags": 3, "carvers": 0, "biomes": 1})
        self.assertEqual(before, {p.name: p.read_bytes() for p in self.root.iterdir()})

    def test_reports_all_missing_tags_without_replacing_them(self):
        self.write("block_tags", {})
        self.assertEqual(self.codes(), ["missing_modern_tag"] * 3)

    def test_configured_carver_rules_and_steps_are_not_flattened(self):
        self.write("carvers", {"minecraft:cave": {
            "type": "minecraft:cave", "config": {
                "replaceable": "#test:replaceable", "lava_level": {"absolute": -54},
                "debug_settings": {"debug_mode": False}}}})
        self.write("biomes", {"test:biome": {"carvers": {"air": ["cave"], "liquid": []}}})
        self.assertEqual(self.codes(), ["configured_carver"] + ["per_carver_rule"] * 3 + ["carving_steps"])

    def test_inline_configured_holder_is_also_reported(self):
        self.write("biomes", {"test:b": {"carvers": [{"type": "minecraft:cave", "config": {}}]}})
        self.assertEqual(self.codes(), ["configured_carver"])

    def test_missing_fields_do_not_get_defaults(self):
        self.write("carvers", {"test:c": {"type": "cave", "probability": 0.1}})
        issues = audit.inspect(self.root)["issues"]
        self.assertEqual({x["detail"] for x in issues}, set(audit.CAVE_FIELDS) - {"probability"})

    def test_unknown_and_tagged_holders(self):
        self.write("biomes", {"test:b": {"carvers": ["test:missing", "#test:set"]}})
        self.assertEqual(self.codes(), ["unknown_carver", "unsupported_carver_tag"])

    def test_unknown_type_and_missing_biome_field(self):
        self.write("carvers", {"test:unknown": {"type": "test:future"}})
        self.write("biomes", {"test:b": {}})
        self.assertEqual(self.codes(), ["unsupported_carver_type", "missing_biome_carvers"])

    def test_normalized_ids_and_duplicate_aliases(self):
        definition = {"type": "cave", **dict.fromkeys(audit.CAVE_FIELDS, 1)}
        self.write("carvers", {"cave": definition})
        self.write("biomes", {"test:b": {"carvers": "minecraft:cave"}})
        self.assertEqual(self.codes(), [])
        self.write("carvers", {"cave": definition, "minecraft:cave": definition})
        with self.assertRaisesRegex(ValueError, "duplicate normalized"):
            audit.inspect(self.root)
        for name in ("Test:Cave", "minecraft:", "a:b:c"):
            with self.subTest(name=name), self.assertRaises(ValueError):
                audit.identifier(name)

    def test_rejects_ambiguous_or_nonfinite_json(self):
        for value in ('{"id":1,"id":2}', '{"config":{"a":1,"a":2}}', '{"x":NaN}'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                audit.decode(value)

    def test_rejects_wrong_bundle_encoding(self):
        (self.root / "carvers.json").write_text('{"test:c":{"type":"cave"}}', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "JSON text"):
            audit.inspect(self.root)

    def test_rejects_malformed_steps_config_and_holders(self):
        for carvers in ({"air": None}, [None], [{"type": "cave", "config": []}]):
            self.write("biomes", {"test:b": {"carvers": carvers}})
            with self.subTest(carvers=carvers), self.assertRaises(ValueError):
                audit.inspect(self.root)

    def test_single_holder_in_carving_step(self):
        self.write("carvers", {"nether_cave": {"type": "nether_cave", "config": {}}})
        self.write("biomes", {"test:b": {"carvers": {"air": "minecraft:nether_cave"}}})
        self.assertEqual(self.codes(), ["configured_carver", "carving_steps"])

    def test_cli_exit_codes_and_no_output_file(self):
        with contextlib.redirect_stdout(io.StringIO()) as output:
            self.assertEqual(audit.main([str(self.root)]), 0)
        self.assertEqual(json.loads(output.getvalue())["runtime_parity"], "not_run")
        self.write("block_tags", {})
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(audit.main([str(self.root)]), 1)
        (self.root / "carvers.json").unlink()
        with contextlib.redirect_stderr(io.StringIO()):
            self.assertEqual(audit.main([str(self.root)]), 2)


if __name__ == "__main__":
    unittest.main()
