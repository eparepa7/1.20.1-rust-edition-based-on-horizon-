#!/usr/bin/env python3
"""Compare local 1.20.1 Java movement with a Rust resident-chunk scene.

The Java side clips against a single captured shape. This checks Rust's chunk
lookup and collision scan as well as movement, including shulker and piston
snapshots, but not Java's world lifecycle, entity ticks, stepping, or a game.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

sys.path.insert(0, str(Path(__file__).resolve().parent))
import version_port as port


def compare_rows(expected, actual, count=48270):
    if len(expected) != count or len(actual) != count:
        raise ValueError(f"Expected {count} movement rows, got Java={len(expected)} Rust={len(actual)}")
    for index, (left, right) in enumerate(zip(expected, actual), 1):
        if left != right:
            raise ValueError(f"Movement differs at row {index}: Java={left}, Rust={right}")
    return count


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "named-server", "metadata", "blocks", "facts", "collision-dir"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--java", default=None)
    parser.add_argument("--cargo", default="cargo")
    args = parser.parse_args()
    _, version = port.verified_local_server("1.20.1", args.server, args.metadata)
    if port.jar_facts(args.named_server) != version:
        raise ValueError("Named server has different version facts")
    blocks = json.loads(args.blocks.read_text(encoding="utf-8"))
    facts = json.loads(args.facts.read_text(encoding="utf-8"))
    if port.validate_legacy_block_facts(facts, blocks) != (24135, 1003):
        raise ValueError("Unexpected 1.20.1 block registry size")
    java = port.selected_java(args.java, version["java_version"])
    cargo = args.cargo
    if os.sep in cargo or (os.altsep and os.altsep in cargo):
        cargo = str(Path(cargo).resolve())
    cases = [
        ("Legacy1201SceneMovementOracle", "horizon-client-render", "legacy_scene_movement_probe", 48270),
        ("Legacy1201SceneEntityMovementOracle", "horizon-client-render", "legacy_scene_entity_movement_probe", 300),
        ("Legacy1201HeightmapOracle", "horizon-world", "legacy_heightmap_probe", 24135),
    ]
    counts = []
    for oracle, package, probe, count in cases:
        with tempfile.TemporaryDirectory(prefix="horizon-1201-scene-oracle-") as directory:
            reference = port.run_legacy_facts_exporter(
                args.server, args.named_server, java, Path(directory), oracle, "reference.txt",
            )
            expected = reference.read_text(encoding="utf-8").splitlines()
            probe_args = ([str(reference)] if package == "horizon-world" else
                          [str(args.facts.resolve()), str(args.collision_dir.resolve())])
            command = [cargo, "run", "--quiet", "-p", package, "--example", probe, "--", *probe_args]
            result = subprocess.run(command, cwd=port.lab.ROOT, capture_output=True, text=True)
            if result.returncode:
                raise RuntimeError(f"Rust probe {probe} failed: " + (result.stderr or result.stdout)[-3000:])
            try:
                counts.append(compare_rows(expected, result.stdout.splitlines(), count))
            except ValueError as error:
                raise ValueError(f"{probe}: {error}") from error
    print(json.dumps({"version": "1.20.1", "states": 24135, "movement_cases": counts[0],
                      "block_entity_movement_cases": counts[1], "heightmap_predicate_cases": counts[2] * 6,
                      "metadata_authenticity_verified": False, "named_server_provenance_verified": False,
                      "note": "Captured single-block scenes; no Java world lifecycle or ticks."}, indent=2))


if __name__ == "__main__":
    main()
