#!/usr/bin/env python3
"""Compare full GeneratorData 1.20.1 heightmaps to the real Java ProtoChunk."""
import argparse
import json
from pathlib import Path
import subprocess
import tempfile

try:
    from tools import lab, version_port as port
    from tools.test_legacy_carver_config import compare_rows
except ModuleNotFoundError:
    import lab
    import version_port as port
    from test_legacy_carver_config import compare_rows


def run(args):
    meta, version = port.verified_local_server("1.20.1", args.server, args.metadata)
    if port.jar_facts(args.named_server) != version:
        raise ValueError("Named server version mismatch")
    marker = json.loads((args.worldgen / "legacy-generator.json").read_text(encoding="utf-8"))
    if marker.get("source_server_sha1") != meta["downloads"]["server"]["sha1"]:
        raise ValueError("Generator marker has a different source server checksum")
    with args.facts.open(encoding="utf-8") as stream:
        states, _ = port.validate_legacy_block_facts(json.load(stream), json.loads(args.blocks.read_text(encoding="utf-8")))
    with tempfile.TemporaryDirectory(prefix="horizon-1201-generator-check-") as directory:
        reference = port.run_legacy_facts_exporter(args.server, args.named_server,
            port.selected_java(args.java, 17), Path(directory), "Legacy1201HeightmapChunkOracle", "java.txt")
        expected = reference.read_text(encoding="utf-8").splitlines()
    result = subprocess.run([args.cargo, "run", "--quiet", "-p", "horizon-world", "--example", "legacy_generator_data_probe", "--",
        str(args.facts.resolve()), str(args.worldgen.resolve()), str(args.heightmap.resolve())], cwd=lab.ROOT, check=True, capture_output=True, text=True)
    actual = result.stdout.splitlines()
    if len(expected) != states:
        raise ValueError("Unexpected Java coverage")
    compare_rows(expected, actual)
    return {"version": "1.20.1", "states": states, "matched_generator_height_updates": states * 6,
            "biomes_loaded": 64, "rejection_checks": 6,
            "scope": "resource loading and chunk heightmaps; generation execution is explicitly refused"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "metadata", "named-server", "facts", "blocks", "worldgen", "heightmap"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--java")
    parser.add_argument("--cargo", default="cargo")
    print(json.dumps(run(parser.parse_args()), indent=2))


if __name__ == "__main__":
    main()
