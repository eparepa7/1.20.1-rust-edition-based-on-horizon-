#!/usr/bin/env python3
"""Read-only inventory of known GeneratorData format barriers; no parity verdict.

Input: a private worldgen bundle directory (JSON objects of id -> JSON text).
This does not prepare game data, translate carvers or certify either version.
"""
import argparse
import json
from pathlib import Path
import re
import sys


MOTION_TAGS = (
    "minecraft:blocks_motion_in_heightmap",
    "minecraft:blocks_motion_in_heightmap_no_leaves",
    "minecraft:uncarvable",
)
CAVE_FIELDS = (
    "probability", "y", "count", "thickness", "room_vertical_radius_multiplier",
    "horizontal_radius_multiplier", "vertical_radius_multiplier", "floor_level",
)
CANYON_FIELDS = ("probability", "y", "vertical_rotation", "shape")


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def decode(text):
    def invalid_constant(value):
        raise ValueError(f"non-finite JSON number: {value}")
    return json.loads(text, object_pairs_hook=unique_object, parse_constant=invalid_constant)


def identifier(name):
    namespace, separator, path = name.partition(":")
    if not separator:
        namespace, path = "minecraft", namespace
    namespace = namespace or "minecraft"
    if not re.fullmatch(r"[a-z0-9_.-]+", namespace) or not re.fullmatch(r"[a-z0-9_./-]+", path):
        raise ValueError(f"invalid resource id: {name}")
    return f"{namespace}:{path}"


def read_bundle(directory, name):
    bundle = decode((directory / f"{name}.json").read_text(encoding="utf-8"))
    if not isinstance(bundle, dict) or any(not isinstance(v, str) for v in bundle.values()):
        raise ValueError(f"{name}: expected an object of resource id -> JSON text")
    result = {}
    for key, value in bundle.items():
        key = identifier(key)
        if key in result:
            raise ValueError(f"{name}: duplicate normalized resource id: {key}")
        parsed = decode(value)
        if not isinstance(parsed, dict):
            raise ValueError(f"{name}/{key}: expected a resource object")
        result[key] = parsed
    return result


def inspect(directory):
    tags = read_bundle(directory, "block_tags")
    carvers = read_bundle(directory, "carvers")
    biomes = read_bundle(directory, "biomes")
    issues = []

    def issue(location, code, detail):
        issues.append({"location": location, "code": code, "detail": detail})

    for tag in MOTION_TAGS:
        if tag not in tags:
            issue("block_tags", "missing_modern_tag", tag)

    def inspect_carver(location, definition):
        kind = definition.get("type")
        # Configured carvers cannot be unwrapped into the current mask pipeline:
        # replacement, lava/debug rules and per-carver ordering need an oracle.
        if "config" in definition:
            issue(location, "configured_carver", "config must not be discarded or flattened")
            config = definition["config"]
            if not isinstance(config, dict):
                raise ValueError(f"{location}: config must be an object")
            for name in ("replaceable", "lava_level", "debug_settings"):
                if name in config:
                    issue(location, "per_carver_rule", name)
            return
        normalized = identifier(kind) if isinstance(kind, str) else None
        if normalized == "minecraft:cave":
            required = CAVE_FIELDS
        elif normalized == "minecraft:canyon":
            required = CANYON_FIELDS
        else:
            issue(location, "unsupported_carver_type", str(kind))
            return
        for name in required:
            if name not in definition:
                issue(location, "missing_modern_carver_field", name)

    for name, definition in sorted(carvers.items()):
        inspect_carver(f"carvers/{name}", definition)

    def inspect_holder(location, holder):
        if isinstance(holder, str):
            if holder.startswith("#"):
                issue(location, "unsupported_carver_tag", holder)
            else:
                name = identifier(holder)
                if name not in carvers:
                    issue(location, "unknown_carver", holder)
        elif isinstance(holder, dict):
            inspect_carver(location, holder)
        else:
            raise ValueError(f"{location}: expected a carver id or inline object")

    for name, biome in sorted(biomes.items()):
        location = f"biomes/{name}/carvers"
        if "carvers" not in biome:
            issue(location, "missing_biome_carvers", "field is absent")
            continue
        holders = biome["carvers"]
        if isinstance(holders, dict) and "type" not in holders:
            issue(location, "carving_steps", "step boundaries and order must be preserved")
            for step, entries in sorted(holders.items()):
                if not isinstance(entries, list):
                    entries = [entries]
                for index, entry in enumerate(entries):
                    inspect_holder(f"{location}/{step}/{index}", entry)
        else:
            for index, entry in enumerate(holders if isinstance(holders, list) else [holders]):
                inspect_holder(f"{location}/{index}", entry)

    return {
        "scope": "known format barriers only; not a full Rust decoder or version check",
        "runtime_parity": "not_run",
        "counts": {"block_tags": len(tags), "carvers": len(carvers), "biomes": len(biomes)},
        "issues": issues,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    args = parser.parse_args(argv)
    try:
        report = inspect(args.directory)
    except (OSError, ValueError) as error:
        print(f"Cannot inspect generator bundles: {error}", file=sys.stderr)
        return 2
    print(json.dumps(report, ensure_ascii=False, indent=2))
    return 1 if report["issues"] else 0


if __name__ == "__main__":
    sys.exit(main())
