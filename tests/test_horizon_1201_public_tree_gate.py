"""Offline regression tests for the mandatory public-tree gate (no game data)."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools"))
from check_public_tree import prohibited  # noqa: E402

ORACLES = (
    ".github/tools/Legacy1201CarverConfigOracle.java",
    ".github/tools/Legacy1201CarverGeometryOracle.java",
)


class PublicTreeGateTests(unittest.TestCase):
    def test_exact_two_original_oracles_allowed(self):
        for path in ORACLES:
            with self.subTest(path=path):
                self.assertFalse(prohibited(path))

    def test_same_name_under_other_roots_is_rejected(self):
        for path in (
            ".github/other/Legacy1201CarverConfigOracle.java",
            ".github/tools/copies/Legacy1201CarverGeometryOracle.java",
            ".github/tools/legacy1201carverconfigoracle.java",
            "other/Legacy1201CarverGeometryOracle.java",
        ):
            with self.subTest(path=path):
                self.assertTrue(prohibited(path))

    def test_other_java_sources_are_still_rejected(self):
        for path in (
            ".github/tools/UnexpectedOracle.java",
            "decompiled/net/minecraft/Block.java",
            "minecraft/net/minecraft/Block.java",
            "tools/net/minecraft/Block.java",
        ):
            with self.subTest(path=path):
                self.assertTrue(prohibited(path))

    def test_game_artifacts_remain_blocked(self):
        for path in (
            "minecraft.jar",
            "lib/client.JAR",
            "data/horizon-data.bin",
            "assets/blocks.json",
            "run/world/level.dat",
            ".local/1.20.1/client.zip",
            "logs/latest.log",
            "debug.jfr",
            "dump.class",
        ):
            with self.subTest(path=path):
                self.assertTrue(prohibited(path))

    def test_existing_original_tool_allowance_preserved(self):
        self.assertFalse(prohibited("tools/Legacy1201CarverConfigOracle.java"))
        self.assertFalse(prohibited("tools/Legacy1201CarverGeometryOracle.java"))
        self.assertFalse(prohibited("tests/java/JfrFixture.java"))

    def test_git_index_gate_accepts_oracles_but_rejects_jar(self):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)
            subprocess.run(["git", "init", "-q"], cwd=root, check=True)
            for path in ORACLES:
                dest = root / path
                dest.parent.mkdir(parents=True, exist_ok=True)
                dest.write_text("// original oracle placeholder; no game files\n", encoding="utf-8")
            subprocess.run(["git", "add", "-A"], cwd=root, check=True)
            gate = ROOT / "tools" / "check_public_tree.py"
            good = subprocess.run([sys.executable, str(gate)], cwd=root, capture_output=True, text=True)
            self.assertEqual(good.returncode, 0, good.stderr)
            unwanted = root / ".github/tools/AnotherOracle.java"
            unwanted.write_text("// forbidden Java\n", encoding="utf-8")
            subprocess.run(["git", "add", "-A"], cwd=root, check=True)
            bad = subprocess.run([sys.executable, str(gate)], cwd=root, capture_output=True, text=True)
            self.assertNotEqual(bad.returncode, 0)
            self.assertIn(".github/tools/AnotherOracle.java", bad.stderr)


if __name__ == "__main__":
    unittest.main()
