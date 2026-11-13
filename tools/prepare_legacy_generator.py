#!/usr/bin/env python3
"""Prepare private 1.20.1 worldgen bundles from a verified local server; no download."""
import argparse
import json
from pathlib import Path
import shutil
import tempfile
import zipfile

try:
    from tools import lab, version_port as port
except ModuleNotFoundError:
    import lab
    import version_port as port


BUNDLES = {
    "noises": "worldgen/noise", "densities": "worldgen/density_function",
    "settings": "worldgen/noise_settings", "carvers": "worldgen/configured_carver",
    "biomes": "worldgen/biome", "biome_tags": "tags/worldgen/biome",
    "block_tags": "tags/blocks", "world_presets": "worldgen/world_preset",
    "features": "worldgen/configured_feature", "placed_features": "worldgen/placed_feature",
    "fluid_tags": "tags/fluids",
}


def prepare(server, metadata, output, java=None):
    meta, facts = port.verified_local_server("1.20.1", server, metadata)
    if facts["world_version"] != 3465:
        raise ValueError("Expected DataVersion 3465")
    output = port.external_new_path(output)
    counts = {}
    with tempfile.TemporaryDirectory(prefix="horizon-1201-generator-", dir=output.parent) as directory:
        scratch = Path(directory)
        game, _ = lab.unpack_server(server.resolve(), scratch, "1.20.1")
        staged = scratch / "worldgen"
        staged.mkdir()
        with zipfile.ZipFile(game) as archive:
            for name, resource in BUNDLES.items():
                prefix = f"data/minecraft/{resource}/"
                entries = {"minecraft:" + path[len(prefix):-5]: archive.read(path).decode("utf-8")
                           for path in archive.namelist() if path.startswith(prefix) and path.endswith(".json")}
                if not entries:
                    raise ValueError(f"Missing 1.20.1 resources: {resource}")
                for text in entries.values():
                    value = json.loads(text)
                    if name != "densities" and not isinstance(value, dict):
                        raise ValueError(f"Invalid {resource} resource")
                (staged / f"{name}.json").write_text(json.dumps(entries), encoding="utf-8")
                counts[name] = len(entries)
        reports = scratch / "reports"
        port.local_reports("1.20.1", server, metadata, reports, java)
        # GeneratorData expects direct report names, whereas 1.20.1 namespaces its reports.
        parameters = staged / "biome_parameters"
        parameters.mkdir()
        for name in ("overworld", "nether"):
            source = reports / "biome_parameters/minecraft" / f"{name}.json"
            if not source.is_file():
                raise ValueError(f"Missing biome parameter report {name}")
            shutil.copyfile(source, parameters / source.name)
        (staged / "legacy-generator.json").write_text(json.dumps({
            "schema": 1, "version": "1.20.1", "DataVersion": 3465,
            "source_server_sha1": meta["downloads"]["server"]["sha1"],
        }), encoding="utf-8")
        # A same-parent rename publishes a complete new directory, never partial resources.
        if output.exists():
            raise FileExistsError(output)
        staged.rename(output)
    return {"version": "1.20.1", "bundles": counts, "output": str(output),
            "scope": "private resource transport; not a playable game or generation parity proof"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server", "metadata", "out"):
        parser.add_argument("--" + name, required=True, type=Path)
    parser.add_argument("--java")
    args = parser.parse_args()
    print(json.dumps(prepare(args.server, args.metadata, args.out, args.java), indent=2))


if __name__ == "__main__":
    main()
