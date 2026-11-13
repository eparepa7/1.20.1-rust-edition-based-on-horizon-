#!/usr/bin/env python3
"""Compare local 1.20.1 carver config decoding with vanilla, not path generation."""
import argparse
import copy
import json
import os
from pathlib import Path
import subprocess
import tempfile
import zipfile

try:
    from tools import lab, version_port as port
except ModuleNotFoundError:
    import lab
    import version_port as port


def corpus(definitions):
    result = dict(definitions)
    cave = json.loads(result["minecraft:cave"])

    def case(name, update):
        value = copy.deepcopy(cave)
        update(value["config"])
        result[f"test:{name}"] = json.dumps(value)

    case("debug_absent", lambda c: c.pop("debug_settings"))
    case("debug_empty", lambda c: c.update(debug_settings={}))
    case("debug_enabled", lambda c: c["debug_settings"].update(debug_mode=True))
    case("direct_replaceable", lambda c: c.update(replaceable=["minecraft:stone", "minecraft:air", "minecraft:water"]))
    case("single_replaceable", lambda c: c.update(replaceable="minecraft:stone"))
    case("negative_zero", lambda c: c.update(yScale=-0.0, floor_level=0.0))
    case("constant_object", lambda c: c.update(yScale={"type": "minecraft:constant", "value": {"value": 0.25}}))
    case("clamped_normal", lambda c: c.update(yScale={"type": "minecraft:clamped_normal", "value": {"mean": 0.5, "deviation": 0.25, "min": -1.0, "max": 2.0}}))
    case("below_top_lava", lambda c: c.update(lava_level={"below_top": 7}))
    case("absolute_lava", lambda c: c.update(lava_level={"absolute": -54}))
    case("invalid_floor", lambda c: c.update(floor_level=0.5))
    case("invalid_uniform", lambda c: c.update(yScale={"type": "minecraft:uniform", "value": {"min_inclusive": 1.0, "max_exclusive": 1.0}}))
    case("missing_replaceable", lambda c: c.pop("replaceable"))
    case("unknown_tag", lambda c: c.update(replaceable="#test:missing"))
    case("invalid_probability", lambda c: c.update(probability=1.01))
    return result


def compare_rows(expected, actual):
    if len(expected) != len(actual):
        raise ValueError(f"Row count differs: Java={len(expected)}, Rust={len(actual)}")
    for index, (left, right) in enumerate(zip(expected, actual)):
        if left != right:
            raise ValueError(f"Row {index} differs: Java={left!r}, Rust={right!r}")


def run(args):
    _, version = port.verified_local_server("1.20.1", args.server, args.metadata)
    if port.jar_facts(args.named_server) != version:
        raise ValueError("Named server version mismatch")
    with args.facts.open(encoding="utf-8") as stream:
        states, blocks = port.validate_legacy_block_facts(json.load(stream), json.loads(args.blocks.read_text(encoding="utf-8")))
    java = port.selected_java(args.java, 17)
    with tempfile.TemporaryDirectory(prefix="horizon-1201-carvers-") as directory:
        scratch = Path(directory)
        game, libraries = lab.unpack_server(args.server.resolve(), scratch, "1.20.1")
        bundles = scratch / "worldgen"
        bundles.mkdir()
        with zipfile.ZipFile(game) as archive:
            for name, prefix in (("carvers", "worldgen/configured_carver"), ("block_tags", "tags/blocks"), ("biomes", "worldgen/biome")):
                prefix = f"data/minecraft/{prefix}/"
                entries = {"minecraft:" + path[len(prefix):-5]: archive.read(path).decode("utf-8")
                           for path in archive.namelist() if path.startswith(prefix) and path.endswith(".json")}
                if not entries:
                    raise ValueError(f"Missing vanilla {name}")
                if name == "carvers":
                    if len(entries) != 4:
                        raise ValueError("Expected the four vanilla 1.20.1 configured carvers")
                    entries = corpus(entries)
                (bundles / f"{name}.json").write_text(json.dumps(entries), encoding="utf-8")
        classpath = os.pathsep.join(map(str, [args.named_server.resolve(), *libraries]))
        source = lab.ROOT / "tools/Legacy1201CarverConfigOracle.java"
        subprocess.run([java, "--module", "jdk.compiler/com.sun.tools.javac.Main", "--release", "17", "-cp", classpath, "-d", str(scratch), str(source)], cwd=scratch, check=True, capture_output=True, text=True)
        reference = scratch / "java.txt"
        subprocess.run([java, "-cp", os.pathsep.join([str(scratch), classpath]), "Legacy1201CarverConfigOracle", str(bundles), str(reference)], cwd=scratch, check=True, capture_output=True, text=True)
        result = subprocess.run([args.cargo, "run", "--quiet", "-p", "horizon-world", "--example", "legacy_carver_config_probe", "--", str(args.facts.resolve()), str(bundles)], cwd=lab.ROOT, check=True, capture_output=True, text=True)
        expected = reference.read_text(encoding="utf-8").splitlines()
        actual = result.stdout.splitlines()
        compare_rows(expected, actual)
        counts = {kind: sum(row.startswith(kind + " ") for row in expected) for kind in "CBLRSE"}
        if counts != {"C": 14, "B": states * 14, "L": 42, "R": 8064, "S": 128, "E": 5}:
            raise ValueError(f"Unexpected coverage: {counts}")
        return {"version": "1.20.1", "states": states, "blocks": blocks, "matched_rows": len(expected), "counts": counts,
                "scope": "configured inputs, replacement predicates, scalar samples and RNG tails, biome step lists; not carving paths or terrain generation"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "metadata", "named-server", "facts", "blocks"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--java")
    parser.add_argument("--cargo", default="cargo")
    args = parser.parse_args()
    print(json.dumps(run(args), ensure_ascii=False, indent=2))


if __name__ == "__main__":
    main()
