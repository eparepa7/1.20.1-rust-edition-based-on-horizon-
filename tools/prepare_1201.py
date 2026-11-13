#!/usr/bin/env python3
"""Prepare local Minecraft 1.20.1 resources for Horizon 1.20.1 (Rust Edition).

Copies or prepares the user's own Minecraft 1.20.1 client jar and assets into
`.local/horizon-client/1.20.1` (which is git-ignored) and runs `horizon-prepare`
if needed.

Usage:
    python tools/prepare_1201.py
    python tools/prepare_1201.py --minecraft-dir "C:/Users/<you>/AppData/Roaming/.minecraft"
    python tools/prepare_1201.py --prepare
"""

import argparse
import os
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
TARGET_DIR = ROOT / ".local" / "horizon-client" / "1.20.1"


def default_minecraft_dir() -> Path | None:
    if os.name == "nt":
        appdata = os.environ.get("APPDATA")
        if appdata:
            candidate = Path(appdata) / ".minecraft"
            if candidate.is_dir():
                return candidate
    elif sys.platform == "darwin":
        candidate = Path.home() / "Library" / "Application Support" / "minecraft"
        if candidate.is_dir():
            return candidate
    else:
        candidate = Path.home() / ".minecraft"
        if candidate.is_dir():
            return candidate
    return None


def copy_if_present(src: Path, dst: Path) -> bool:
    if not src.exists():
        return False
    dst.parent.mkdir(parents=True, exist_ok=True)
    if src.is_dir():
        if not dst.exists():
            shutil.copytree(src, dst)
    else:
        shutil.copy2(src, dst)
    return True


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Prepare .local/horizon-client/1.20.1 from your Minecraft 1.20.1 install."
    )
    parser.add_argument(
        "--minecraft-dir",
        type=Path,
        default=None,
        help="Path to your .minecraft directory (auto-detected if omitted).",
    )
    parser.add_argument(
        "--prepare",
        action="store_true",
        help="Also run `horizon-prepare --prepare` after staging 1.20.1 files.",
    )
    args = parser.parse_args()

    mc_dir = args.minecraft_dir or default_minecraft_dir()
    if mc_dir is None or not mc_dir.is_dir():
        sys.stderr.write(
            "Could not locate .minecraft automatically. "
            "Pass --minecraft-dir <path-to-.minecraft>.\n"
        )
        return 1

    version_dir = mc_dir / "versions" / "1.20.1"
    jar_src = version_dir / "1.20.1.jar"
    json_src = version_dir / "1.20.1.json"
    if not jar_src.is_file():
        sys.stderr.write(
            f"Minecraft 1.20.1 jar not found at {jar_src}.\n"
            "Launch Minecraft: Java Edition 1.20.1 once in the official launcher first.\n"
        )
        return 1

    TARGET_DIR.mkdir(parents=True, exist_ok=True)
    copy_if_present(jar_src, TARGET_DIR / "1.20.1.jar")
    copy_if_present(json_src, TARGET_DIR / "1.20.1.json")

    assets_src = mc_dir / "assets"
    if assets_src.is_dir():
        copy_if_present(assets_src / "indexes", TARGET_DIR / "assets" / "indexes")
        copy_if_present(assets_src / "objects", TARGET_DIR / "assets" / "objects")

    print(f"Staged Minecraft 1.20.1 resources in {TARGET_DIR}")

    if args.prepare:
        prep_cmd = [
            sys.executable,
            str(ROOT / "tools" / "prepare_native_client.py"),
            "--prepare",
            "--minecraft-dir",
            str(mc_dir),
        ]
        return subprocess.call(prep_cmd, cwd=ROOT)
    return 0


if __name__ == "__main__":
    sys.exit(main())
