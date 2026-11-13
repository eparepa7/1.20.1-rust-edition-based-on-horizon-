import hashlib
import json
from pathlib import Path
import tempfile
import unittest

from tools.horizon_1201_package import prepare_bundle


class PortableBundle(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.launcher = self.root / "data" / "launcher"
        self.version = self.launcher / "versions" / "v0.2.100"
        self.version.mkdir(parents=True)
        self.pin = {"version": "26.3", "client_sha1": "a" * 40,
                    "server_sha1": "b" * 40, "java_major": 25}
        (self.version / "research-lock.json").write_text(json.dumps({"minecraft": self.pin}))
        for name in [".installed", "horizon-client.exe", "horizon-prepare.exe"]:
            (self.version / name).write_bytes(b"test fixture")
        (self.root / "horizon-launcher.exe").write_bytes(b"launcher fixture")
        (self.root / "horizon-portable.txt").write_text("portable")
        (self.launcher / "settings.json").write_text(json.dumps({
            "selected": "v0.2.100", "minecraft_dir": "C:/CI/private", "dev_offline": False,
        }))
        (self.launcher / "accounts.json").write_text('{"accounts":[],"selected":null}')

    def test_bundle_is_relocatable_offline_and_checksums_match(self):
        record = prepare_bundle(self.root, self.pin)
        self.assertEqual(record["minecraft"], "26.3")
        settings = json.loads((self.launcher / "settings.json").read_text())
        self.assertTrue(settings["dev_offline"])
        self.assertFalse(settings["install_new_versions"])
        self.assertNotIn("minecraft_dir", settings)
        self.assertFalse((self.launcher / "accounts.json").exists())
        sums = (self.root / "SHA256SUMS").read_text().splitlines()
        self.assertGreater(len(sums), 5)
        for line in sums:
            digest, relative = line.split("  ", 1)
            self.assertEqual(digest, hashlib.sha256((self.root / relative).read_bytes()).hexdigest())

    def test_future_minecraft_is_not_mislabeled_as_supported(self):
        other = dict(self.pin, version="26.4")
        (self.version / "research-lock.json").write_text(json.dumps({"minecraft": other}))
        with self.assertRaisesRegex(ValueError, "Incompatible engine"):
            prepare_bundle(self.root, self.pin)

    def test_missing_engine_is_not_packaged_as_playable(self):
        (self.version / "horizon-client.exe").unlink()
        with self.assertRaisesRegex(ValueError, "Missing bundle file"):
            prepare_bundle(self.root, self.pin)

    def test_mojang_jar_is_rejected(self):
        (self.version / "client.jar").write_bytes(b"not redistributable")
        with self.assertRaisesRegex(ValueError, "Private game files"):
            prepare_bundle(self.root, self.pin)

    def test_signed_in_account_is_rejected(self):
        (self.launcher / "accounts.json").write_text('{"accounts":[{"name":"Test"}]}')
        with self.assertRaisesRegex(ValueError, "signed-in accounts"):
            prepare_bundle(self.root, self.pin)

    def test_version_path_cannot_escape_the_bundle(self):
        (self.launcher / "settings.json").write_text('{"selected":"../../../outside"}')
        with self.assertRaisesRegex(ValueError, "valid version"):
            prepare_bundle(self.root, self.pin)


if __name__ == "__main__":
    unittest.main()
