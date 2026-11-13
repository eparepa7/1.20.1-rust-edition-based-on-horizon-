#!/usr/bin/env python3
"""Validate and finish the portable Windows bundle after the signed engine install."""
import hashlib
import json
import os
from pathlib import Path
import re
import sys


def prepare_bundle(root: Path, expected: dict) -> dict:
    root = root.resolve()
    launcher = root / "data" / "launcher"
    settings_path = launcher / "settings.json"
    settings = json.loads(settings_path.read_text(encoding="utf-8"))
    tag = settings.get("selected", "")
    if not isinstance(tag, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", tag):
        raise ValueError("The signed engine installer did not select a valid version")
    version = launcher / "versions" / tag
    for required in [version / ".installed", version / "horizon-client.exe",
                     version / "horizon-prepare.exe", root / "horizon-launcher.exe",
                     root / "horizon-portable.txt"]:
        if not required.is_file():
            raise ValueError(f"Missing bundle file: {required.relative_to(root)}")
    installed = json.loads((version / "research-lock.json").read_text(encoding="utf-8"))["minecraft"]
    for key in ["version", "client_sha1", "server_sha1", "java_major"]:
        if installed.get(key) != expected.get(key):
            raise ValueError(f"Incompatible engine: {key} differs from this fork. "
                             "Run the workflow with a compatible game_tag.")
    # Preparing Minecraft happens on the player's PC, never while building this archive.
    private_parts = {".local", "minecraft", "linked-minecraft", "server-jars", "sessions"}
    private_names = {"secrets.json", "accounts.secrets", "horizon-data.bin", "options.txt"}
    for path in root.rglob("*"):
        relative = path.relative_to(root)
        if path.is_symlink():
            raise ValueError(f"A portable bundle must not contain links: {relative}")
        if (private_parts.intersection(relative.parts) or path.name in private_names
                or path.suffix.lower() in {".jar", ".jfr", ".hprof"}):
            raise ValueError(f"Private game files or credentials in bundle: {relative}")
    accounts_path = launcher / "accounts.json"
    if accounts_path.exists():
        accounts = json.loads(accounts_path.read_text(encoding="utf-8"))
        if accounts.get("accounts") or accounts.get("selected"):
            raise ValueError("A bundle must never contain signed-in accounts")
        accounts_path.unlink()  # Horizon 1.20.1 creates the local Developer name on the player's PC.
    settings_path.write_text(json.dumps({
        "selected": tag, "dev_offline": True, "desktop_entry": False,
        "install_new_versions": False, "skip_opening": True, "language": "ru",
    }, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    record = {
        "project": "Horizon 1.20.1 Rust Edition", "minecraft": installed["version"],
        "launcher_source_commit": os.environ.get("GITHUB_SHA"),
        "engine_source": "https://github.com/horizon-rust/horizon", "engine_tag": tag,
        "local_resources_required": True,
    }
    (root / "FORK_BUILD.json").write_text(json.dumps(record, indent=2) + "\n", encoding="utf-8")
    sums = []
    for path in sorted(root.rglob("*")):
        if path.is_file() and path.name != "SHA256SUMS":
            with path.open("rb") as source:
                digest = hashlib.file_digest(source, "sha256").hexdigest()
            sums.append(f"{digest}  {path.relative_to(root).as_posix()}")
    (root / "SHA256SUMS").write_text("\n".join(sums) + "\n", encoding="utf-8")
    return record


if __name__ == "__main__":
    repo = Path(__file__).resolve().parents[1]
    if len(sys.argv) != 2:
        sys.exit("Usage: python tools/horizon_1201_package.py <portable-folder>")
    expected = json.loads((repo / "research-lock.json").read_text(encoding="utf-8"))["minecraft"]
    try:
        print(json.dumps(prepare_bundle(Path(sys.argv[1]), expected), indent=2))
    except (OSError, KeyError, ValueError) as error:
        sys.exit(str(error))
