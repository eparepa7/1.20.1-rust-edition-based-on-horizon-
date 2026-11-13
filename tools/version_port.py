#!/usr/bin/env python3
"""Port a new Minecraft release to Horizon: detect it, prepare it, diff it against the pinned one
(docs/version-ports.md).

    version_port.py detect [--snapshots]          what Mojang released that Horizon doesn't pin
    version_port.py prepare VERSION               download, verify, run the data generator, read its facts,
                                                  index its classes (tools/class_index.py)
    version_port.py inspect VERSION --client JAR --metadata JSON [--mappings TXT]
                                                 verify a local client and inspect its version facts
    version_port.py remap-client VERSION --client JAR --metadata JSON --mappings TXT --remapper JAR --out JAR
                                                 create a separate named client for port research
    version_port.py remap-server VERSION --server JAR --metadata JSON --mappings TXT --remapper JAR --out JAR
                                                 create a separate named server for port research
    version_port.py local-reports VERSION --server JAR --metadata JSON --out DIR
                                                 generate local reports from a checked server jar
    version_port.py compare-reports --old DIR --new DIR
                                                 compare only the reports both directories contain
    version_port.py legacy-packets --server JAR --named-server JAR --metadata JSON --out JSON
                                                 export 1.20.1 packet classes and IDs locally
    version_port.py legacy-block-facts --server JAR --named-server JAR --metadata JSON --blocks JSON --out JSON
                                                 prepare local 1.20.1 block identities for the Rust registry
    version_port.py legacy-heightmap --server JAR --named-server JAR --metadata JSON --blocks JSON --facts JSON --out NBT
                                                 prepare verified local 1.20.1 heightmap data
    version_port.py legacy-gameplay-facts --server JAR --named-server JAR --metadata JSON --blocks JSON --facts JSON --out JSON
                                                 prepare 1.20.1 per-state block properties locally
    version_port.py legacy-behaviour-facts --server JAR --named-server JAR --metadata JSON --blocks JSON --facts JSON --out JSON
                                                 prepare 1.20.1 block behaviour data locally
    version_port.py legacy-prepare-block-data --server JAR --metadata JSON --mappings TXT --remapper JAR --out DIR
                                                 prepare the three block-data files together outside the repo
    version_port.py diff OLD NEW                  facts, packets, registries, blocks, items, commands,
                                                  data packs, assets, lang and classes, with the Horizon
                                                  code each changed class concerns: diff-OLD-NEW/diff.json
    version_port.py regen NEW [--base REF]        a port/NEW worktree pinned to NEW with every generated table
                                                  regenerated (tools/port_regen.py): diff-OLD-NEW/regen.json
    version_port.py harnesses NEW [--baseline S]  horizon-prepare and every harness in the candidate, each pass,
                                                  oracle (no longer compiles) or fail, and newly failing against
                                                  a run on the pinned version (tools/port_harness.py)
    version_port.py report OLD NEW [--results S]  the ranked port report: diff-OLD-NEW/report.md and .json
    version_port.py publish OLD NEW [--yes]       the report's work items as GitHub issues (tools/port_publish.py);
                                                  a dry run without --yes
    version_port.py port NEW [--old OLD]          prepare both, diff, regen, report
    version_port.py gc [--keep VERSION...]        keep the pinned, the newest two others and --keep

The downloads are for development only: they go to the version store (~/code/horizon-parity/versions,
HORIZON_VERSIONS overrides it), never into the repository, and nothing from them is shipped. Candidate
worktrees go to ~/code/horizon-parity/ports/NEW (HORIZON_PORTS overrides it). The pinned
version is not downloaded: its files are read where tools/lab.py prepared them (.local/).

detect exits 0 when nothing is new, 10 for a new release and 11 for a new snapshot (with --snapshots).
"""

import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import urllib.request
import zipfile

try:
    from . import class_index, lab, mojang_class_map, port_harness, port_publish, port_regen, port_report, vanilla_map, version_facts
except ImportError:
    import class_index
    import lab
    import mojang_class_map
    import port_harness
    import port_publish
    import port_regen
    import port_report
    import vanilla_map
    import version_facts

MANIFEST_URL = "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json"
STORE = Path(os.environ.get("HORIZON_VERSIONS", Path.home() / "code" / "horizon-parity" / "versions"))
PORTS = Path(os.environ.get("HORIZON_PORTS", STORE.parent / "ports"))
STATES = ["handshake", "status", "login", "configuration", "play"]
# How many changed JSON paths one changed file keeps in diff.json.
PATH_LIMIT = 50


def safe_version(version):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+", version) or version in {".", ".."}:
        raise ValueError(f"Unsafe version identifier: {version!r}")
    return version


def fetch_json(url, sha1=None):
    request = urllib.request.Request(url, headers={"User-Agent": "horizon/0.1"})
    with urllib.request.urlopen(request, timeout=60) as response:
        data = response.read()
    if sha1 is not None and hashlib.sha1(data).hexdigest() != sha1:
        raise ValueError(f"{url}: sha1 mismatch")
    return data


def pinned():
    return json.loads(lab.LOCK.read_text(encoding="utf-8"))["minecraft"]["version"]


# --- detect ------------------------------------------------------------------------------------------------

def news(manifest, pin, seen, snapshots=False):
    """[(kind, version)] Mojang lists that are neither pinned nor seen before, newest kind first."""
    found = []
    for kind in ["release"] + (["snapshot"] if snapshots else []):
        latest = manifest["latest"][kind]
        if latest != pin and latest != seen.get(kind):
            # The latest snapshot is the latest release right after a release: report it once.
            if kind == "snapshot" and latest == manifest["latest"]["release"]:
                continue
            found.append((kind, latest))
    return found


def detect(args):
    manifest = json.loads(fetch_json(MANIFEST_URL))
    state_path = STORE / "detect-state.json"
    seen = json.loads(state_path.read_text()) if state_path.exists() else {}
    found = news(manifest, pinned(), seen, args.snapshots)
    if not found:
        print(f"Nothing new: latest release {manifest['latest']['release']}, Horizon pins {pinned()}")
        return 0
    for kind, version in found:
        print(f"New {kind}: {version} (Horizon pins {pinned()})")
        seen[kind] = version
    if not args.dry_run:
        lab.write_json(state_path, seen)
    return 10 if found[0][0] == "release" else 11


# --- prepare -----------------------------------------------------------------------------------------------

def version_dir(version):
    return STORE / safe_version(version)


def paths(version):
    """The prepared files of a version: {client, server, server_game, reports, version_json}."""
    record = version_dir(version) / "prepared.json"
    if not record.exists():
        raise SystemExit(f"{version} is not prepared; run version_port.py prepare {version}")
    return {key: Path(value) for key, value in json.loads(record.read_text())["paths"].items()}


def jar_facts(client_jar):
    """The facts the game states about itself in the client jar's version.json."""
    with zipfile.ZipFile(client_jar) as archive:
        info = json.loads(archive.read("version.json"))
    resource, data = version_facts.pack_formats(info)
    return {
        "id": info["id"],
        "world_version": info["world_version"],
        "protocol_version": info["protocol_version"],
        "resource_pack": f"{resource[0]}.{resource[1]}",
        "data_pack": f"{data[0]}.{data[1]}",
        "java_version": info["java_version"],
        "stable": info.get("stable"),
    }


def inspect_local(version, client, metadata, mappings=None):
    """Inspect a player's local version without preparing, changing or launching it.

    The client is checked against the supplied metadata; this alone does not
    authenticate the metadata, or mean that Horizon can play this version.
    """
    safe_version(version)
    meta = json.loads(metadata.read_text(encoding="utf-8-sig"))
    if meta.get("id") != version:
        raise ValueError(f"The metadata is for {meta.get('id')!r}, not {version}")
    expected = meta["downloads"]["client"]["sha1"]
    if not re.fullmatch(r"[0-9a-f]{40}", expected):
        raise ValueError("The client checksum in the metadata is invalid")
    if lab.digest(client, "sha1") != expected:
        raise ValueError("The client jar does not match the supplied metadata")
    facts = jar_facts(client)
    if facts["id"] != version:
        raise ValueError(f"The client jar says it is {facts['id']}, not {version}")
    result = {
        "version": facts,
        "client_matches_supplied_metadata": True,
        "metadata_authenticity_verified": False,
        "needs_class_remapping": any(f"{side}_mappings" in meta["downloads"]
                                     for side in ("client", "server")),
        "note": "Inspection is not a playable port; the existing engine is unchanged.",
    }
    if mappings is not None:
        mapping = meta["downloads"].get("client_mappings")
        if not mapping:
            raise ValueError("The supplied metadata does not specify client mappings")
        if lab.digest(mappings, "sha1") != mapping["sha1"]:
            raise ValueError("The client mappings do not match the supplied metadata")
        with mappings.open(encoding="utf-8") as stream:
            names = mojang_class_map.read_classes(stream)
        with zipfile.ZipFile(client) as archive:
            classes = [entry.filename[:-6] for entry in archive.infolist()
                       if entry.filename.endswith(".class") and not entry.filename.startswith("META-INF/")]
        matched = sum(name in names for name in classes)
        result["class_names"] = {"mapped": matched, "unmapped": len(classes) - matched,
                                 "scope": "class names only; bytecode is not remapped"}
        result["client_mappings_match_supplied_metadata"] = True
    return result


def inspect(args):
    print(json.dumps(inspect_local(args.version, args.client, args.metadata, args.mappings), indent=2))
    return 0


def selected_java(java, major):
    """Keep PATH commands usable and resolve explicit paths before changing cwd."""
    if java is None:
        return java_command(major)
    path = Path(java)
    return str(path.resolve()) if path.is_absolute() or path.parent != Path(".") else str(path)


def remap_local_client(version, client, metadata, mappings, remapper, output, java=None):
    """Create a separate local named jar; never modify the input or advertise a playable port."""
    inspection = inspect_local(version, client, metadata, mappings)
    if not inspection["needs_class_remapping"]:
        raise ValueError("This version does not declare mappings for an obfuscated client")
    java = selected_java(java, inspection["version"]["java_version"])
    return remap_named_jar(version, client, mappings, remapper, output, java)


def external_new_path(output):
    """Resolve a new output outside the project, without following a final symlink."""
    if output.is_symlink():
        raise FileExistsError(f"The output is a symbolic link: {output}")
    output = output.resolve()
    if output.is_relative_to(lab.ROOT.resolve()):
        raise ValueError("Write Minecraft files outside the source repository")
    if output.exists():
        raise FileExistsError(f"The output already exists: {output}")
    if not output.parent.is_dir():
        raise ValueError(f"The output folder does not exist: {output.parent}")
    return output


def remap_named_jar(version, source, mappings, remapper, output, java):
    """Remap a verified game jar into a new file; names are not proof of parity."""
    output = external_new_path(output)
    if not remapper.is_file():
        raise ValueError("The remapping tool was not found")
    with mappings.open(encoding="utf-8") as stream:
        mapping = mojang_class_map.read_classes(stream)
    with zipfile.ZipFile(source) as archive:
        source_classes = [entry.filename[:-6] for entry in archive.infolist()
                          if entry.filename.endswith(".class") and not entry.filename.startswith("META-INF/")]
        version_json = archive.read("version.json")
    missing = [name for name in source_classes if name not in mapping]
    if not source_classes or missing:
        raise ValueError(f"Mappings are incomplete: {len(missing)} unmapped classes")
    expected = {mapping[name] + ".class" for name in source_classes}
    with tempfile.TemporaryDirectory(prefix="horizon-1201-remap-", dir=output.parent) as directory:
        staged = Path(directory) / "named.jar"
        cmd = [java, "-jar", str(remapper), "--input", str(source), "--output", str(staged),
               "--map", str(mappings), "--reverse", "--threads", "2", "--ann-fix", "--ids-fix",
               "--src-fix", "--strip-sigs"]
        result = subprocess.run(cmd, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("The remapping tool failed: " + (result.stderr or result.stdout)[-2000:])
        with zipfile.ZipFile(staged) as archive:
            if archive.testzip() is not None or archive.read("version.json") != version_json:
                raise ValueError("The remapped jar is damaged or has different version metadata")
            missing_output = expected - set(archive.namelist())
        if missing_output:
            raise ValueError(f"The remapped jar is missing {len(missing_output)} expected class names")
        created = False
        try:
            with output.open("xb") as target, staged.open("rb") as source:
                created = True
                shutil.copyfileobj(source, target)
        except Exception:
            if created:
                output.unlink(missing_ok=True)
            raise
    return {"version": version, "class_names": len(expected), "output": str(output),
            "note": "Development input only; this is not a playable Rust port."}


def remap_client(args):
    print(json.dumps(remap_local_client(args.version, args.client, args.metadata, args.mappings,
                                        args.remapper, args.out, args.java), indent=2))
    return 0


def verified_local_server(version, server, metadata):
    """Check a local server against supplied metadata, without claiming metadata authenticity."""
    safe_version(version)
    meta = json.loads(metadata.read_text(encoding="utf-8-sig"))
    if meta.get("id") != version:
        raise ValueError(f"The metadata is for {meta.get('id')!r}, not {version}")
    expected = meta["downloads"]["server"]["sha1"]
    if not re.fullmatch(r"[0-9a-f]{40}", expected) or lab.digest(server, "sha1") != expected:
        raise ValueError("The server jar does not match the supplied metadata")
    facts = jar_facts(server)
    if facts["id"] != version:
        raise ValueError(f"The server jar says it is {facts['id']}, not {version}")
    return meta, facts


def remap_local_server(version, server, metadata, mappings, remapper, output, java=None):
    """Verify a bundled server, unpack its game jar, then remap it outside the project."""
    meta, facts = verified_local_server(version, server, metadata)
    expected = meta["downloads"]["server_mappings"]["sha1"]
    if not re.fullmatch(r"[0-9a-f]{40}", expected) or lab.digest(mappings, "sha1") != expected:
        raise ValueError("The server_mappings file does not match the supplied metadata")
    output = external_new_path(output)
    java = selected_java(java, facts["java_version"])
    with tempfile.TemporaryDirectory(prefix="horizon-1201-server-", dir=output.parent) as directory:
        source = unpack_game(server, Path(directory) / "server-game.jar", version)
        if jar_facts(source) != facts:
            raise ValueError("The bundled server game has different version facts")
        return remap_named_jar(version, source, mappings, remapper, output, java)


def remap_server(args):
    print(json.dumps(remap_local_server(args.version, args.server, args.metadata, args.mappings,
                                        args.remapper, args.out, args.java), indent=2))
    return 0


def local_reports(version, server, metadata, output, java=None):
    """Generate server reports into a new external folder, leaving the jar and repository unchanged."""
    _, facts = verified_local_server(version, server, metadata)
    output = external_new_path(output)
    java = selected_java(java, facts["java_version"])
    server = server.resolve()
    with tempfile.TemporaryDirectory(prefix="horizon-1201-reports-", dir=output.parent) as directory:
        scratch = Path(directory)
        generated = scratch / "output"
        command = [java, "-DbundlerMainClass=net.minecraft.data.Main", "-jar", str(server),
                   "--reports", "--output", str(generated)]
        result = subprocess.run(command, cwd=scratch, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("The data generator failed: " + (result.stderr or result.stdout)[-2000:])
        reports = generated / "reports"
        expected_reports = ("blocks.json", "registries.json", "commands.json")
        for name in expected_reports:
            path = reports / name
            if not path.is_file() or not isinstance(json.loads(path.read_text(encoding="utf-8")), dict):
                raise ValueError(f"The data generator did not produce a valid {name}")
        names = sorted(path.name for path in reports.iterdir())
        output.mkdir()
        try:
            for path in reports.iterdir():
                target = output / path.name
                if path.is_dir():
                    shutil.copytree(path, target)
                else:
                    shutil.copy2(path, target)
        except Exception:
            shutil.rmtree(output)
            raise
    return {"version": version, "reports": names, "output": str(output),
            "metadata_authenticity_verified": False,
            "note": "Development reports only; this is not a playable Rust port."}


def make_local_reports(args):
    print(json.dumps(local_reports(args.version, args.server, args.metadata, args.out, args.java), indent=2))
    return 0


def compare_local_reports(old, new):
    """Compare common data reports without pretending missing packet or item reports are empty."""
    def read(directory, name):
        path = directory / name
        value = json.loads(path.read_text(encoding="utf-8"))
        if not isinstance(value, dict):
            raise ValueError(f"Expected an object in {path}")
        return value

    return {
        "blocks": diff_blocks(read(old, "blocks.json"), read(new, "blocks.json")),
        "registries": diff_registries(read(old, "registries.json"), read(new, "registries.json")),
        "commands": diff_commands(read(old, "commands.json"), read(new, "commands.json")),
        "not_compared": ["network packets", "item components", "gameplay behavior", "rendering", "world generation"],
        "note": "A report comparison is not a playable Rust port or proof of compatibility.",
    }


def compare_reports(args):
    print(json.dumps(compare_local_reports(args.old, args.new), indent=2))
    return 0


def validate_legacy_block_facts(facts, blocks):
    """Check all identities/defaults against the original server's blocks report.

    The report does not independently contain the air/fluid flags; those come
    from the remapped server's block states. It does not verify game behavior.
    """
    if not isinstance(facts, dict) or not isinstance(blocks, dict) or not blocks:
        raise ValueError("Missing block facts or original block report")
    rows, defaults = facts.get("states"), facts.get("defaults")
    if not isinstance(rows, list) or not rows or not isinstance(defaults, dict):
        raise ValueError("Invalid block facts")
    if set(defaults) != set(blocks):
        raise ValueError("The block defaults do not match the report")
    seen = set()
    for name, block in blocks.items():
        if not isinstance(name, str) or not isinstance(block, dict):
            raise ValueError("Invalid block in report")
        states = block.get("states")
        if not isinstance(states, list) or not states:
            raise ValueError(f"No states for {name}")
        found_default = []
        for state in states:
            if not isinstance(state, dict):
                raise ValueError(f"Invalid state for {name}")
            index = state.get("id")
            if type(index) is not int or index < 0 or index >= len(rows) or index in seen:
                raise ValueError(f"Invalid or duplicate state ID for {name}")
            seen.add(index)
            row = rows[index]
            properties = state.get("properties", {})
            if (not isinstance(properties, dict)
                    or any(not isinstance(k, str) or not isinstance(v, str) for k, v in properties.items())):
                raise ValueError(f"Invalid properties for {name}")
            if (not isinstance(row, list) or len(row) < 5 or row[0] != name
                    or type(row[1]) is not bool or type(row[2]) is not bool
                    or not isinstance(row[3], str) or not isinstance(row[4], dict)
                    or row[4].get("id") != name
                    or row[4].get("properties", {}) != properties):
                raise ValueError(f"Block state {index} does not match the report")
            if state.get("default") is True:
                found_default.append(index)
        if (len(found_default) != 1 or type(defaults[name]) is not int
                or defaults[name] != found_default[0]):
            raise ValueError(f"Default block state for {name} does not match the report")
    if len(seen) != len(rows):
        raise ValueError("Some block facts have no matching report state")
    return len(seen), len(blocks)


def run_legacy_facts_exporter(server, named_server, java, scratch, class_name, filename):
    """Compile one project-owned exporter against the player's local server and libraries."""
    if class_name not in {"Legacy1201BlockFactsExporter", "Legacy1201GameplayFactsExporter",
                          "Legacy1201BlockBehaviourExporter", "Legacy1201DynamicStatesExporter",
                          "Legacy1201CollisionExporter", "Legacy1201CachedCollisionOracle",
                          "Legacy1201DynamicCollisionOracle", "Legacy1201EntityCollisionOracle",
                          "Legacy1201OutlineOracle", "Legacy1201LiquidCollisionOracle",
                          "Legacy1201ContextCollisionOracle", "Legacy1201AllContextOracle",
                          "Legacy1201SceneMovementOracle", "Legacy1201SceneEntityMovementOracle", "Legacy1201HeightmapOracle", "Legacy1201HeightmapExporter", "Legacy1201HeightmapChunkOracle", "Legacy1201HeightmapColumnsOracle"}:
        raise ValueError("Unknown local facts exporter")
    source = lab.ROOT / "tools" / f"{class_name}.java"
    if not source.is_file():
        raise ValueError("The local facts exporter source is missing")
    _, libraries = lab.unpack_server(server.resolve(), scratch, "1.20.1")
    classpath = os.pathsep.join(map(str, [named_server.resolve(), *libraries]))
    command = [java, "--module", "jdk.compiler/com.sun.tools.javac.Main", "--release", "17",
               "-cp", classpath, "-d", str(scratch), str(source)]
    compiled = subprocess.run(command, cwd=scratch, capture_output=True, text=True)
    if compiled.returncode:
        raise RuntimeError("Facts exporter compilation failed; use a JDK: "
                           + (compiled.stderr or compiled.stdout)[-2000:])
    staged = scratch / filename
    command = [java, "-cp", os.pathsep.join([str(scratch), classpath]), class_name, str(staged)]
    ran = subprocess.run(command, cwd=scratch, capture_output=True, text=True)
    if ran.returncode:
        raise RuntimeError("Facts export failed: " + (ran.stderr or ran.stdout)[-2000:])
    return staged


def publish_new_local_file(staged, output):
    """Publish a completely validated file without replacing an existing one."""
    created = False
    try:
        with output.open("xb") as target, staged.open("rb") as generated:
            created = True
            shutil.copyfileobj(generated, target)
    except Exception:
        if created:
            output.unlink(missing_ok=True)
        raise


def export_legacy_block_facts(server, named_server, metadata, blocks_report, output, java=None):
    """Make local 1.20.1 Rust-registry block facts; never overwrite existing data."""
    _, facts = verified_local_server("1.20.1", server, metadata)
    if jar_facts(named_server) != facts:
        raise ValueError("The named server has different version facts")
    output = external_new_path(output)
    java = selected_java(java, facts["java_version"])
    blocks = json.loads(blocks_report.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix="horizon-1201-blocks-", dir=output.parent) as directory:
        scratch = Path(directory)
        staged = run_legacy_facts_exporter(server, named_server, java, scratch,
                                          "Legacy1201BlockFactsExporter", "facts.json")
        with staged.open(encoding="utf-8") as stream:
            state_count, block_count = validate_legacy_block_facts(json.load(stream), blocks)
        publish_new_local_file(staged, output)
    return {"version": "1.20.1", "states": state_count, "blocks": block_count, "output": str(output),
            "metadata_authenticity_verified": False, "named_server_provenance_verified": False,
            "note": "Block identities, defaults, air and fluid only; not a playable Rust port."}


def make_legacy_block_facts(args):
    print(json.dumps(export_legacy_block_facts(args.server, args.named_server, args.metadata,
                                               args.blocks, args.out, args.java), indent=2))
    return 0


def validate_legacy_dynamic_states(rows, blocks):
    """Check each uncached state against the original server report."""
    expected = {state["id"]: (name, state.get("properties", {}))
                for name, block in blocks.items() for state in block["states"]}
    classes = {"MovingPistonBlock": ("piston.", "block_entity", 12),
               "ShulkerBoxBlock": ("", "block_entity", 102),
               "BambooStalkBlock": ("", "position_offset", 12),
               "PointedDripstoneBlock": ("", "position_offset", 20),
               "ScaffoldingBlock": ("", "collision_context", 32),
               "PowderSnowBlock": ("", "collision_context", 1)}
    if not isinstance(rows, list) or len(rows) != 179:
        raise ValueError("Expected 179 uncached states")
    seen, counts = set(), {key: 0 for key in classes}
    for row in rows:
        if not isinstance(row, dict) or type(row.get("id")) is not int:
            raise ValueError("Invalid uncached state")
        index = row["id"]
        name = row.get("class", "").rsplit(".", 1)[-1]
        if name not in classes:
            raise ValueError("Unknown dynamic class")
        prefix, dependency, _ = classes[name]
        if (row.get("class") != "net.minecraft.world.level.block." + prefix + name
                or row.get("shape_dependency") != dependency or row.get("dynamic_shape") is not True
                or index in seen or expected.get(index) != (row.get("block"), row.get("properties"))):
            raise ValueError(f"Invalid uncached state {index}")
        seen.add(index)
        counts[name] += 1
    if any(counts[name] != value[2] for name, value in classes.items()):
        raise ValueError("Unexpected class counts")
    return counts


def make_legacy_dynamic_states(args):
    _, version = verified_local_server("1.20.1", args.server, args.metadata)
    if jar_facts(args.named_server) != version:
        raise ValueError("The named server has different version facts")
    output = external_new_path(args.out)
    java = selected_java(args.java, version["java_version"])
    blocks = json.loads(args.blocks.read_text(encoding="utf-8"))
    with tempfile.TemporaryDirectory(prefix="horizon-1201-dynamic-", dir=output.parent) as directory:
        staged = run_legacy_facts_exporter(args.server, args.named_server, java, Path(directory),
                                          "Legacy1201DynamicStatesExporter", "dynamic.json")
        counts = validate_legacy_dynamic_states(json.loads(staged.read_text(encoding="utf-8")), blocks)
        publish_new_local_file(staged, output)
    print(json.dumps({"states": sum(counts.values()), "classes": counts, "output": str(output),
                      "named_server_provenance_verified": False}, indent=2))
    return 0


def export_legacy_collision(server, named_server, metadata, blocks_report, block_facts, output, java=None, cargo="cargo"):
    """Export and validate a private 1.20.1 collision table before publication."""
    _, version = verified_local_server("1.20.1", server, metadata)
    if jar_facts(named_server) != version:
        raise ValueError("The named server has different version facts")
    blocks = json.loads(blocks_report.read_text(encoding="utf-8"))
    facts = json.loads(block_facts.read_text(encoding="utf-8"))
    state_count, block_count = validate_legacy_block_facts(facts, blocks)
    if (state_count, block_count) != (24135, 1003):
        raise ValueError("Unexpected 1.20.1 block registry size")
    output = external_new_path(output)
    java = selected_java(java, version["java_version"])
    with tempfile.TemporaryDirectory(prefix="horizon-1201-collision-", dir=output.parent) as directory:
        staged = run_legacy_facts_exporter(server, named_server, java, Path(directory),
                                          "Legacy1201CollisionExporter", "cached-collision.nbt")
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_collision_probe", "--", str(block_facts.resolve()), str(staged)]
        result = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if result.returncode:
            raise RuntimeError("1.20.1 Rust collision validation failed: " + (result.stderr or result.stdout)[-2000:])
        if result.stdout.strip() != "states=24135 cached=23956 uncached=179 shapes=4294":
            raise ValueError("Unexpected 1.20.1 collision table counts: " + result.stdout.strip())
        with tempfile.TemporaryDirectory(prefix="oracle-", dir=directory) as oracle_dir:
            reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                  "Legacy1201CachedCollisionOracle", "cached-reference.txt")
            expected = reference.read_text(encoding="utf-8").splitlines()
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_cached_collision_probe", "--", str(block_facts.resolve()), str(staged)]
        compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if compared.returncode:
            raise RuntimeError("Rust collision comparison failed: " + (compared.stderr or compared.stdout)[-2000:])
        actual = compared.stdout.splitlines()
        if len(expected) != 23956 or len(actual) != len(expected):
            raise ValueError("Unexpected number of collision comparison rows")
        for index, (left, right) in enumerate(zip(expected, actual), 1):
            if left != right:
                raise ValueError(f"Cached collision comparison differs at row {index}")
        with tempfile.TemporaryDirectory(prefix="dynamic-oracle-", dir=directory) as oracle_dir:
            reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                  "Legacy1201DynamicCollisionOracle", "dynamic-reference.txt")
            dynamic_expected = reference.read_text(encoding="utf-8").splitlines()
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_dynamic_collision_probe", "--", str(block_facts.resolve()), str(staged)]
        compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if compared.returncode:
            raise RuntimeError("Rust dynamic collision comparison failed: " + (compared.stderr or compared.stdout)[-2000:])
        dynamic_actual = compared.stdout.splitlines()
        if len(dynamic_expected) != 1074 or len(dynamic_actual) != len(dynamic_expected):
            raise ValueError("Unexpected number of dynamic collision comparison rows")
        for index, (left, right) in enumerate(zip(dynamic_expected, dynamic_actual), 1):
            if left != right:
                raise ValueError(f"Dynamic collision comparison differs at row {index}")
        with tempfile.TemporaryDirectory(prefix="entity-oracle-", dir=directory) as oracle_dir:
            reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                  "Legacy1201EntityCollisionOracle", "entity-reference.txt")
            entity_expected = reference.read_text(encoding="utf-8").splitlines()
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_entity_collision_probe", "--", str(block_facts.resolve()), str(staged)]
        compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if compared.returncode:
            raise RuntimeError("Rust block-entity collision comparison failed: " + (compared.stderr or compared.stdout)[-2000:])
        entity_actual = compared.stdout.splitlines()
        if len(entity_expected) != 3270 or len(entity_actual) != len(entity_expected):
            raise ValueError("Unexpected number of block-entity collision comparison rows")
        for index, (left, right) in enumerate(zip(entity_expected, entity_actual), 1):
            if left != right:
                raise ValueError(f"Block-entity collision comparison differs at row {index}")
        with tempfile.TemporaryDirectory(prefix="outline-oracle-", dir=directory) as oracle_dir:
            reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                  "Legacy1201OutlineOracle", "outline-reference.txt")
            outline_expected = reference.read_text(encoding="utf-8").splitlines()
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_outline_probe", "--", str(block_facts.resolve()), str(staged)]
        compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if compared.returncode:
            raise RuntimeError("Rust outline comparison failed: " + (compared.stderr or compared.stdout)[-2000:])
        outline_actual = compared.stdout.splitlines()
        if len(outline_expected) != 3752 or len(outline_actual) != len(outline_expected):
            raise ValueError("Unexpected number of outline comparison rows")
        for index, (left, right) in enumerate(zip(outline_expected, outline_actual), 1):
            if left != right:
                raise ValueError(f"Outline comparison differs at row {index}")
        with tempfile.TemporaryDirectory(prefix="liquid-oracle-", dir=directory) as oracle_dir:
            reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                  "Legacy1201LiquidCollisionOracle", "liquid-reference.txt")
            liquid_expected = reference.read_text(encoding="utf-8").splitlines()
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_liquid_collision_probe", "--", str(block_facts.resolve()), str(staged)]
        compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if compared.returncode:
            raise RuntimeError("Rust fluid collision comparison failed: " + (compared.stderr or compared.stdout)[-2000:])
        liquid_actual = compared.stdout.splitlines()
        if len(liquid_expected) != 768 or len(liquid_actual) != len(liquid_expected):
            raise ValueError("Unexpected number of fluid collision comparison rows")
        for index, (left, right) in enumerate(zip(liquid_expected, liquid_actual), 1):
            if left != right:
                raise ValueError(f"Fluid collision comparison differs at row {index}")
        with tempfile.TemporaryDirectory(prefix="context-oracle-", dir=directory) as oracle_dir:
            reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                  "Legacy1201ContextCollisionOracle", "context-reference.txt")
            context_expected = reference.read_text(encoding="utf-8").splitlines()
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_context_collision_probe", "--", str(block_facts.resolve()), str(staged)]
        compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if compared.returncode:
            raise RuntimeError("Rust contextual collision comparison failed: " + (compared.stderr or compared.stdout)[-2000:])
        context_actual = compared.stdout.splitlines()
        if len(context_expected) != 632 or len(context_actual) != len(context_expected):
            raise ValueError("Unexpected number of contextual collision comparison rows")
        for index, (left, right) in enumerate(zip(context_expected, context_actual), 1):
            if left != right:
                raise ValueError(f"Contextual collision comparison differs at row {index}")
        with tempfile.TemporaryDirectory(prefix="all-context-oracle-", dir=directory) as oracle_dir:
            reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                  "Legacy1201AllContextOracle", "all-context-reference.txt")
            all_expected = reference.read_text(encoding="utf-8").splitlines()
        command = [str(cargo), "run", "--quiet", "-p", "horizon-world", "--example",
                   "legacy_all_context_probe", "--", str(block_facts.resolve()), str(staged)]
        compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
        if compared.returncode:
            raise RuntimeError("Rust all-state collision comparison failed: " + (compared.stderr or compared.stdout)[-2000:])
        actual = compared.stdout.splitlines()
        if len(all_expected) != 48270 or len(actual) != len(all_expected):
            raise ValueError("Unexpected number of all-state comparison rows")
        for index, (left, right) in enumerate(zip(all_expected, actual), 1):
            if left != right:
                raise ValueError(f"All-state collision comparison differs at row {index}")
        publish_new_local_file(staged, output)
    return {"version": "1.20.1", "states": state_count, "cached": 23956, "uncached": 179,
            "shapes": 4294, "matched_cached_shapes": 23956, "matched_dynamic_cases": 1074,
            "matched_block_entity_cases": 3270, "matched_outline_cases": 3752,
            "matched_fluid_cases": 768, "matched_context_cases": 632, "matched_all_context_cases": 48270,
            "output": str(output), "metadata_authenticity_verified": False,
            "named_server_provenance_verified": False,
            "note": "Local collision data; not a playable game or complete world integration."}


def export_legacy_heightmap(server, named_server, metadata, blocks_report, block_facts, output, java=None, cargo="cargo"):
    """Prepare a private 1.20.1 heightmap table only after chunk-level parity."""
    _, version = verified_local_server("1.20.1", server, metadata)
    if jar_facts(named_server) != version:
        raise ValueError("The named server has different version facts")
    blocks = json.loads(blocks_report.read_text(encoding="utf-8"))
    facts = json.loads(block_facts.read_text(encoding="utf-8"))
    state_count, block_count = validate_legacy_block_facts(facts, blocks)
    if (state_count, block_count) != (24135, 1003):
        raise ValueError("Unexpected 1.20.1 block registry size")
    output = external_new_path(output)
    java = selected_java(java, version["java_version"])
    cargo = str(cargo)
    if os.sep in cargo or (os.altsep and os.altsep in cargo):
        cargo = str(Path(cargo).resolve())
    with tempfile.TemporaryDirectory(prefix="horizon-1201-heightmap-", dir=output.parent) as directory:
        scratch = Path(directory)
        staged = run_legacy_facts_exporter(server, named_server, java, scratch,
                                          "Legacy1201HeightmapExporter", "heightmap.nbt")
        cases = [
            ("Legacy1201HeightmapChunkOracle", "legacy_heightmap_chunk_probe", state_count),
            ("Legacy1201HeightmapColumnsOracle", "legacy_heightmap_columns_probe", 36),
        ]
        for oracle, probe, count in cases:
            with tempfile.TemporaryDirectory(prefix="oracle-", dir=scratch) as oracle_dir:
                reference = run_legacy_facts_exporter(server, named_server, java, Path(oracle_dir),
                                                      oracle, "heightmap-reference.txt")
                expected = reference.read_text(encoding="utf-8").splitlines()
            command = [cargo, "run", "--quiet", "-p", "horizon-world", "--example",
                       probe, "--", str(block_facts.resolve()), str(staged)]
            compared = subprocess.run(command, cwd=lab.ROOT, capture_output=True, text=True)
            if compared.returncode:
                raise RuntimeError(f"Rust heightmap chunk comparison {probe} failed: "
                                   + (compared.stderr or compared.stdout)[-2000:])
            actual = compared.stdout.splitlines()
            if len(expected) != count or len(actual) != count:
                raise ValueError(f"Unexpected number of heightmap chunk comparison rows in {probe}")
            for index, (left, right) in enumerate(zip(expected, actual), 1):
                if left != right:
                    raise ValueError(f"Heightmap chunk comparison {probe} differs at row {index}: Java={left}, Rust={right}")
        publish_new_local_file(staged, output)
    return {"version": "1.20.1", "states": state_count,
            "matched_terrain_height_updates": state_count * 6, "matched_column_checkpoints": 36, "output": str(output),
            "metadata_authenticity_verified": False, "named_server_provenance_verified": False,
            "note": "Local terrain snapshots only; not a playable game or complete generator."}


def make_legacy_heightmap(args):
    print(json.dumps(export_legacy_heightmap(args.server, args.named_server, args.metadata,
                                             args.blocks, args.facts, args.out, args.java, args.cargo), indent=2))
    return 0


def make_legacy_collision(args):
    print(json.dumps(export_legacy_collision(args.server, args.named_server, args.metadata,
                                              args.blocks, args.facts, args.out, args.java, args.cargo), indent=2))
    return 0


def validate_legacy_gameplay_facts(gameplay, state_count):
    """Reject incomplete or out-of-order per-state data, before publishing it."""
    states = gameplay.get("states") if isinstance(gameplay, dict) else None
    if not isinstance(states, list) or len(states) != state_count:
        raise ValueError("Gameplay facts do not match the block-state count")
    flags = ("skylight_down", "shape_occlusion", "can_occlude", "solid_render", "requires_tool",
             "replaceable", "random_ticks", "block_entity", "liquid", "ignited_by_lava", "offset",
             "full_collision", "outline_offset")
    numbers = ("hardness", "friction", "speed_factor", "jump_factor", "explosion_resistance")
    for index, state in enumerate(states):
        if not isinstance(state, dict) or type(state.get("id")) is not int or state["id"] != index:
            raise ValueError(f"Gameplay state {index} has the wrong ID")
        if (any(type(state.get(flag)) is not bool for flag in flags)
                or any(type(state.get(n)) not in (int, float) or not math.isfinite(state[n]) for n in numbers)
                or any(type(state.get(n)) is not int or not 0 <= state[n] <= 15
                       for n in ("emission", "dampening"))
                or any(not isinstance(state.get(k), str) for k in ("render_shape", "push", "item"))
                or not isinstance(state.get("outline"), list) or not isinstance(state.get("outline_edges"), list)
                or not isinstance(state.get("sound"), dict)):
            raise ValueError(f"Gameplay state {index} is incomplete")
    return len(states)


def export_legacy_gameplay_facts(server, named_server, metadata, blocks_report, block_facts, output, java=None):
    """Make local 1.20.1 per-state block properties without claiming a playable engine."""
    _, version = verified_local_server("1.20.1", server, metadata)
    if jar_facts(named_server) != version:
        raise ValueError("The named server has different version facts")
    output = external_new_path(output)
    java = selected_java(java, version["java_version"])
    blocks = json.loads(blocks_report.read_text(encoding="utf-8"))
    facts = json.loads(block_facts.read_text(encoding="utf-8"))
    state_count, block_count = validate_legacy_block_facts(facts, blocks)
    del facts, blocks
    with tempfile.TemporaryDirectory(prefix="horizon-1201-gameplay-", dir=output.parent) as directory:
        staged = run_legacy_facts_exporter(server, named_server, java, Path(directory),
                                          "Legacy1201GameplayFactsExporter", "gameplay-facts.json")
        with staged.open(encoding="utf-8") as stream:
            count = validate_legacy_gameplay_facts(json.load(stream), state_count)
        publish_new_local_file(staged, output)
    return {"version": "1.20.1", "states": count, "blocks": block_count, "output": str(output),
            "metadata_authenticity_verified": False, "named_server_provenance_verified": False,
            "note": "Per-state block properties only; full game behavior and rendering are not ported."}


def make_legacy_gameplay_facts(args):
    print(json.dumps(export_legacy_gameplay_facts(args.server, args.named_server, args.metadata,
                                                  args.blocks, args.facts, args.out, args.java), indent=2))
    return 0


def validate_legacy_behaviour_facts(behaviour, facts):
    """Check registry order, field types and exporter error flags before publishing."""
    if not isinstance(behaviour, dict) or not isinstance(facts, dict):
        raise ValueError("Missing block behaviour data")
    rows = facts.get("states")
    blocks, states = behaviour.get("blocks"), behaviour.get("states")
    if not isinstance(rows, list) or not rows or not isinstance(blocks, list) or not isinstance(states, list):
        raise ValueError("Invalid block behaviour data")
    names = list(dict.fromkeys(row[0] for row in rows))
    if len(blocks) != len(names) or len(states) != len(rows):
        raise ValueError("Behaviour data does not match the block-state count")
    fluids = ["minecraft:empty", "minecraft:flowing_water", "minecraft:water",
              "minecraft:flowing_lava", "minecraft:lava"]
    if behaviour.get("fluids") != fluids:
        raise ValueError("Unexpected fluid registry")
    if not isinstance(behaviour.get("items"), dict) or not isinstance(behaviour.get("fire"), dict):
        raise ValueError("Missing block item or fire data")
    def has_field_error(value):
        if isinstance(value, dict):
            return any(has_field_error(v) for v in value.values())
        if isinstance(value, list):
            return any(has_field_error(v) for v in value)
        return value == "<error>"
    if has_field_error(behaviour.get("items")) or has_field_error(behaviour.get("fire")):
        raise ValueError("Block item or fire data contains an exporter error")
    for name, block in zip(names, blocks):
        if (not isinstance(block, dict) or block.get("name") != name
                or not isinstance(block.get("class"), list) or not all(isinstance(c, str) for c in block["class"])
                or not isinstance(block.get("interfaces"), list)
                or not all(isinstance(c, str) for c in block["interfaces"])
                or not isinstance(block.get("fields"), dict) or type(block.get("dynamic_shape")) is not bool
                or not isinstance(block.get("item"), str)):
            raise ValueError(f"Block behaviour for {name} is incomplete or out of order")
        if has_field_error(block["fields"]):
            raise ValueError(f"Block behaviour for {name} contains an exporter error")
    for index, state in enumerate(states):
        if (not isinstance(state, list) or len(state) != 9
                or any(type(state[i]) is not int or state[i] < 0 for i in (0, 1, 2, 3, 5, 6))
                or type(state[4]) is not bool or type(state[7]) is not bool
                or not isinstance(state[8], str) or state[2] >= len(fluids) or state[3] > 8):
            raise ValueError(f"Block behaviour state {index} is incomplete")
        if state[0] & (1 << 30) or state[1] & ((1 << 20) | (1 << 21) | (1 << 22)) or state[7]:
            raise ValueError(f"Block behaviour state {index} contains an exporter error")
    return len(states), len(names)


def export_legacy_behaviour_facts(server, named_server, metadata, blocks_report, block_facts, output, java=None):
    """Make local 1.20.1 block metadata; this does not implement behavior in Rust."""
    _, version = verified_local_server("1.20.1", server, metadata)
    if jar_facts(named_server) != version:
        raise ValueError("The named server has different version facts")
    output = external_new_path(output)
    java = selected_java(java, version["java_version"])
    blocks = json.loads(blocks_report.read_text(encoding="utf-8"))
    facts = json.loads(block_facts.read_text(encoding="utf-8"))
    validate_legacy_block_facts(facts, blocks)
    del blocks
    with tempfile.TemporaryDirectory(prefix="horizon-1201-behaviour-", dir=output.parent) as directory:
        staged = run_legacy_facts_exporter(server, named_server, java, Path(directory),
                                          "Legacy1201BlockBehaviourExporter", "block-behaviours.json")
        with staged.open(encoding="utf-8") as stream:
            state_count, block_count = validate_legacy_behaviour_facts(json.load(stream), facts)
        publish_new_local_file(staged, output)
    return {"version": "1.20.1", "states": state_count, "blocks": block_count, "output": str(output),
            "metadata_authenticity_verified": False, "named_server_provenance_verified": False,
            "note": "Block metadata only; full game behavior and rendering are not ported."}


def make_legacy_behaviour_facts(args):
    print(json.dumps(export_legacy_behaviour_facts(args.server, args.named_server, args.metadata,
                                                   args.blocks, args.facts, args.out, args.java), indent=2))
    return 0


def prepare_legacy_block_data(server, metadata, mappings, remapper, output, java=None):
    """Prepare three checked 1.20.1 block files in a fresh private directory.

    This is not a complete resource preparation and cannot start the game.
    Inputs and any existing output stay unchanged on success or failure.
    """
    output = external_new_path(output)
    with tempfile.TemporaryDirectory(prefix="horizon-1201-1201-block-data-", dir=output.parent) as directory:
        scratch = Path(directory)
        named, reports, result = scratch / "server-named.jar", scratch / "reports", scratch / "result"
        result.mkdir()
        remap_local_server("1.20.1", server, metadata, mappings, remapper, named, java)
        local_reports("1.20.1", server, metadata, reports, java)
        block_report = reports / "blocks.json"
        facts = result / "facts.json"
        block_summary = export_legacy_block_facts(server, named, metadata, block_report, facts, java)
        export_legacy_gameplay_facts(server, named, metadata, block_report, facts,
                                     result / "gameplay-facts.json", java)
        export_legacy_behaviour_facts(server, named, metadata, block_report, facts,
                                      result / "block-behaviours.json", java)
        output.mkdir()
        created = []
        try:
            for name in ("facts.json", "gameplay-facts.json", "block-behaviours.json"):
                target = output / name
                publish_new_local_file(result / name, target)
                created.append(target)
        except Exception:
            for path in reversed(created):
                path.unlink(missing_ok=True)
            try:
                output.rmdir()
            except OSError:
                pass  # Another process may have added a file: keep it.
            raise
    return {"version": "1.20.1", "states": block_summary["states"], "blocks": block_summary["blocks"],
            "output": str(output), "files": ["facts.json", "gameplay-facts.json", "block-behaviours.json"],
            "metadata_authenticity_verified": False, "remapper_authenticity_verified": False,
            "note": "Private block data only; this is not a complete preparation or playable game."}


def make_legacy_block_data(args):
    print(json.dumps(prepare_legacy_block_data(args.server, args.metadata, args.mappings,
                                                args.remapper, args.out, args.java), indent=2))
    return 0


def legacy_packet_classes(lines):
    """Parse the 1.20.1 probe's class registrations, without inferring packet layouts."""
    phases = {"HANDSHAKING": "handshake", "STATUS": "status", "LOGIN": "login", "PLAY": "play"}
    directions = {"CLIENTBOUND": "clientbound", "SERVERBOUND": "serverbound"}
    records, seen = [], set()
    for line in lines:
        parts = line.rstrip("\r\n").split("\t")
        if len(parts) != 4:
            raise ValueError("Invalid packet class probe output")
        phase, direction, packet_id, class_name = parts
        if (phase not in phases or direction not in directions or not packet_id.isdigit()
                or not class_name.startswith("net.minecraft.network.protocol.")
                or not re.fullmatch(r"[A-Za-z0-9_.$]+", class_name)):
            raise ValueError("Invalid packet class probe output")
        identity = (phases[phase], directions[direction], int(packet_id))
        if identity in seen:
            raise ValueError("Duplicate packet ID in the same phase and direction")
        seen.add(identity)
        records.append({"phase": identity[0], "direction": identity[1], "id": identity[2],
                        "class": class_name})
    if {r["phase"] for r in records} != set(phases.values()):
        raise ValueError("Packet class probe output is incomplete")
    return sorted(records, key=lambda r: (r["phase"], r["direction"], r["id"]))


def export_legacy_packets(server, named_server, metadata, output, java=None):
    """Run a local 1.20.1 reflection probe; generated names stay outside the repository."""
    _, facts = verified_local_server("1.20.1", server, metadata)
    if jar_facts(named_server) != facts:
        raise ValueError("The named server has different version facts")
    output = external_new_path(output)
    java = selected_java(java, facts["java_version"])
    source = lab.ROOT / "tools" / "Legacy1201PacketProbe.java"
    if not source.is_file():
        raise ValueError("The development packet probe source is missing")
    with tempfile.TemporaryDirectory(prefix="horizon-1201-packets-", dir=output.parent) as directory:
        scratch = Path(directory)
        _, libraries = lab.unpack_server(server.resolve(), scratch, "1.20.1")
        compile_cmd = [java, "--module", "jdk.compiler/com.sun.tools.javac.Main", "--release", "17",
                       "-d", str(scratch), str(source)]
        compiled = subprocess.run(compile_cmd, capture_output=True, text=True)
        if compiled.returncode:
            raise RuntimeError("Packet probe compilation failed; use a JDK: "
                               + (compiled.stderr or compiled.stdout)[-2000:])
        classpath = os.pathsep.join(map(str, [scratch, named_server.resolve(), *libraries]))
        results = scratch / "packets.tsv"
        command = [java, "-cp", classpath, "Legacy1201PacketProbe", str(results)]
        ran = subprocess.run(command, cwd=scratch, capture_output=True, text=True)
        if ran.returncode:
            raise RuntimeError("Packet probe failed: " + (ran.stderr or ran.stdout)[-2000:])
        with results.open(encoding="utf-8") as stream:
            packets = legacy_packet_classes(stream)
        report = {"version": "1.20.1", "protocol": facts["protocol_version"], "packets": packets,
                  "metadata_authenticity_verified": False, "named_server_provenance_verified": False,
                  "note": "Class registrations and IDs only; packet layouts and compatibility are not verified."}
        created = False
        try:
            with output.open("x", encoding="utf-8") as stream:
                created = True
                json.dump(report, stream, indent=2)
                stream.write("\n")
        except Exception:
            if created:
                output.unlink(missing_ok=True)
            raise
    return {"version": "1.20.1", "packets": len(packets), "output": str(output), "note": report["note"]}


def make_legacy_packets(args):
    print(json.dumps(export_legacy_packets(args.server, args.named_server, args.metadata, args.out, args.java),
                     indent=2))
    return 0


def unpack_game(server, destination, version):
    """The server download is a bundler: take its game jar only (the libraries stay inside it)."""
    with zipfile.ZipFile(server) as archive:
        if "META-INF/versions.list" not in archive.namelist():
            shutil.copyfile(server, destination)
            return destination
        entries = list(lab.bundle_entries(archive, "META-INF/versions.list"))
        if len(entries) != 1 or entries[0][1] != version:
            raise ValueError(f"Expected one bundled server for {version}")
        checksum, _, relative = entries[0]
        lab.extract_verified(archive, f"META-INF/versions/{relative}", destination, checksum)
    return destination


def java_command(major):
    java = Path(os.environ["JAVA_HOME"]) / "bin" / "java" if os.environ.get("JAVA_HOME") else shutil.which("java")
    if not java:
        raise SystemExit("Java not found: set JAVA_HOME or put java on the PATH")
    result = subprocess.run([str(java), "-version"], capture_output=True, text=True)
    found = re.search(r'version "(\d+)', result.stderr + result.stdout)
    if not found or int(found[1]) < major:
        raise SystemExit(f"This version needs Java {major}; {java} is {found[1] if found else 'unknown'}. "
                         f"Install JDK {major} (a fleet toolchain change) and rerun.")
    return str(java)


def run_datagen(server, reports, major):
    """vanilla's data generator, --reports only. The bundler unpacks its libraries into a scratch folder."""
    scratch = Path(tempfile.mkdtemp(prefix=".datagen-", dir=STORE))
    out = scratch / "out"
    try:
        log = scratch / "datagen.log"
        with log.open("w") as stream:
            result = subprocess.run([java_command(major), "-DbundlerMainClass=net.minecraft.data.Main", "-jar",
                                     str(server), "--reports", "--output", str(out)],
                                    cwd=scratch, stdout=stream, stderr=subprocess.STDOUT)
        if result.returncode:
            raise SystemExit(f"The data generator failed:\n{log.read_text()[-3000:]}")
        if reports.exists():
            shutil.rmtree(reports)
        shutil.move(str(out / "reports"), reports)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)


def prepare_pinned(version, directory):
    """The pinned version's files are already in .local: record where, don't copy them."""
    game = lab.LOCAL / "minecraft" / version
    found = {
        "client": game / "jars" / "client.jar",
        "server": game / "jars" / "server.jar",
        "server_game": game / "jars" / "server-game.jar",
        "reports": lab.LOCAL / "datagen" / "out" / "reports",
        "version_json": game / "manifest.json",
    }
    missing = [str(p) for p in found.values() if not p.exists()]
    if missing:
        raise SystemExit("The pinned version is not prepared in .local (tools/lab.py prepare, horizon-prepare): "
                         + ", ".join(missing))
    return {key: path.resolve() for key, path in found.items()}


def prepare_download(version, directory):
    manifest = json.loads(fetch_json(MANIFEST_URL))
    entry = next((v for v in manifest["versions"] if v["id"] == version), None)
    if entry is None:
        raise SystemExit(f"Mojang's manifest has no version {version}")
    version_json = directory / "version.json"
    if not (version_json.exists() and lab.digest(version_json, "sha1") == entry["sha1"]):
        version_json.write_bytes(fetch_json(entry["url"], entry["sha1"]))
    meta = json.loads(version_json.read_text())
    if "client_mappings" in meta["downloads"]:
        raise SystemExit(f"{version} is obfuscated; the pipeline handles unobfuscated releases (26.1 and later)")
    jars = directory / "jars"
    found = {"version_json": version_json}
    for side in ["client", "server"]:
        download = meta["downloads"][side]
        print(f"Fetching the {version} {side} jar ({download['size'] // 1_000_000} MB)", flush=True)
        found[side] = lab.download(download["url"], jars / f"{side}.jar", download["sha1"])
    found["server_game"] = unpack_game(found["server"], jars / "server-game.jar", version)
    found["reports"] = directory / "datagen" / "reports"
    if not found["reports"].exists():
        print(f"Running the {version} data generator", flush=True)
        run_datagen(found["server"], found["reports"], meta["javaVersion"]["majorVersion"])
    return found


def prepare(args):
    version = safe_version(args.version)
    directory = version_dir(version)
    directory.mkdir(parents=True, exist_ok=True)
    found = (prepare_pinned if version == pinned() and not args.download else prepare_download)(version, directory)
    facts = jar_facts(found["client"])
    if facts["id"] != version:
        raise ValueError(f"The client jar says it is {facts['id']}, not {version}")
    lab.write_json(directory / "facts.json", facts)
    classes = class_index.index_jars([found["client"], found["server_game"]], jobs=args.jobs)
    (directory / "classes.json").write_text(json.dumps(classes, separators=(",", ":")))
    print(f"Indexed {len(classes)} classes, {sum(len(c['methods']) for c in classes.values())} methods")
    lab.write_json(directory / "prepared.json", {
        "version": version, "pinned": version == pinned() and not args.download,
        "paths": {key: str(path) for key, path in found.items()},
    })
    print(json.dumps(facts, indent=2))
    return 0


# --- diff --------------------------------------------------------------------------------------------------

def clip(value, limit=200):
    text = json.dumps(value, sort_keys=True)
    return value if len(text) <= limit else text[:limit] + "..."


def json_diff(old, new, path="", limit=PATH_LIMIT):
    """[(path, old, new)] where two JSON values differ; lists of different lengths count as one change."""
    changes = []

    def walk(a, b, at):
        if len(changes) >= limit or a == b:
            return
        if isinstance(a, dict) and isinstance(b, dict):
            for key in sorted(a.keys() | b.keys()):
                walk(a.get(key), b.get(key), f"{at}/{key}")
        elif isinstance(a, list) and isinstance(b, list) and len(a) == len(b):
            for index, (x, y) in enumerate(zip(a, b)):
                walk(x, y, f"{at}[{index}]")
        else:
            changes.append((at or "/", clip(a), clip(b)))

    walk(old, new, path)
    return changes


def keyed_diff(old, new):
    """Added, removed and common keys of two mappings."""
    return sorted(new.keys() - old.keys()), sorted(old.keys() - new.keys()), sorted(old.keys() & new.keys())


def diff_facts(old, new):
    return {key: [old.get(key), new.get(key)] for key in sorted(old.keys() | new.keys())
            if key != "id" and old.get(key) != new.get(key)}


def diff_packets(old, new):
    """Per state and direction: packets added, removed and renumbered (the ids are on the wire)."""
    result = {}
    for state in sorted(old.keys() | new.keys(), key=lambda s: (STATES.index(s) if s in STATES else 99, s)):
        for direction in sorted(old.get(state, {}).keys() | new.get(state, {}).keys()):
            a = {name: e["protocol_id"] for name, e in old.get(state, {}).get(direction, {}).items()}
            b = {name: e["protocol_id"] for name, e in new.get(state, {}).get(direction, {}).items()}
            added, removed, common = keyed_diff(a, b)
            renumbered = {name: [a[name], b[name]] for name in common if a[name] != b[name]}
            if added or removed or renumbered:
                result[f"{state}/{direction}"] = {
                    "added": {name: b[name] for name in added}, "removed": {name: a[name] for name in removed},
                    "renumbered": renumbered, "count": [len(a), len(b)]}
    return result


def diff_registries(old, new):
    """Registries added and removed; per registry its entries added and removed, and how many common
    entries changed protocol id (the network sends those ids)."""
    added, removed, common = keyed_diff(old, new)
    result = {"added": {name: sorted(new[name].get("entries", {})) for name in added}, "removed": removed,
              "changed": {}}
    for name in common:
        a, b = old[name].get("entries", {}), new[name].get("entries", {})
        entries_added, entries_removed, shared = keyed_diff(a, b)
        shifted = [e for e in shared if a[e].get("protocol_id") != b[e].get("protocol_id")]
        change = {}
        if entries_added:
            change["added"] = entries_added
        if entries_removed:
            change["removed"] = entries_removed
        if shifted:
            change["ids_shifted"] = len(shifted)
        if old[name].get("default") != new[name].get("default"):
            change["default"] = [old[name].get("default"), new[name].get("default")]
        if change:
            result["changed"][name] = change
    return result


def diff_blocks(old, new):
    """Blocks added and removed; per block property changes, state counts and default state; and the
    global block-state ids (the network and the chunk palette use them)."""
    added, removed, common = keyed_diff(old, new)
    # A field one report has for no block at all is a change of the report's format, not of every block.
    fields = [{key for block in side.values() for key in block} for side in (old, new)]
    format_change = sorted(fields[0] ^ fields[1])

    def default(block):
        return next((s.get("properties", {}) for s in block["states"] if s.get("default")), {})

    def first_id(block):
        return min(s["id"] for s in block["states"])

    changed = {}
    for name in common:
        a, b = old[name], new[name]
        change = {}
        if a.get("properties", {}) != b.get("properties", {}):
            change["properties"] = json_diff(a.get("properties", {}), b.get("properties", {}))
        if len(a["states"]) != len(b["states"]):
            change["states"] = [len(a["states"]), len(b["states"])]
        if default(a) != default(b):
            change["default"] = [default(a), default(b)]
        if a.get("definition") != b.get("definition") and "definition" not in format_change:
            change["definition"] = json_diff(a.get("definition"), b.get("definition"))
        if change:
            changed[name] = change
    shifted = [name for name in common if first_id(old[name]) != first_id(new[name])]
    first = min(shifted, key=lambda n: first_id(new[n]), default=None)
    return {"added": added, "removed": removed, "changed": changed, "format": format_change,
            "state_count": [sum(len(b["states"]) for b in old.values()), sum(len(b["states"]) for b in new.values())],
            "state_ids_shifted": len(shifted), "first_shifted": first}


def diff_items(old, new):
    """Items added and removed; per item the default components added, removed and changed."""
    added, removed, common = keyed_diff(old, new)
    changed = {}
    for name in common:
        a, b = old[name], new[name]
        components_added, components_removed, shared = keyed_diff(a, b)
        values = {c: json_diff(a[c], b[c], limit=10) for c in shared if a[c] != b[c]}
        if components_added or components_removed or values:
            changed[name] = {"added": components_added, "removed": components_removed, "changed": values}
    components = {}
    for change in changed.values():
        for label in ("added", "removed", "changed"):
            for component in change[label]:
                components.setdefault(component, {"added": 0, "removed": 0, "changed": 0})[label] += 1
    return {"added": added, "removed": removed, "changed": changed, "components": components}


def command_nodes(tree):
    """Every node of a commands report by its path ("execute as targets"), with what defines it."""
    nodes = {}

    def walk(node, path):
        for name, child in node.get("children", {}).items():
            at = f"{path} {name}".strip()
            nodes[at] = {k: child[k] for k in ("type", "parser", "properties", "executable", "redirect", "permissions")
                         if k in child}
            walk(child, at)

    walk(tree, "")
    return nodes


def diff_commands(old, new):
    a, b = command_nodes(old), command_nodes(new)
    added, removed, common = keyed_diff(a, b)
    return {"added": added, "removed": removed,
            "changed": {path: json_diff(a[path], b[path], limit=10) for path in common if a[path] != b[path]}}


def jar_entries(jar, prefix):
    """{path: crc} of a jar's files under prefix."""
    with zipfile.ZipFile(jar) as archive:
        return {i.filename: i.CRC for i in archive.infolist() if i.filename.startswith(prefix) and not i.is_dir()}


def content_kind(path):
    """data/minecraft/worldgen/biome/x.json -> worldgen/biome; assets/minecraft/models/block/x.json -> models/block."""
    parts = path.split("/")[2:-1]
    if parts and parts[0] in {"worldgen", "tags"} and len(parts) > 1:
        return "/".join(parts[:2])
    return parts[0] if parts else "(root)"


def diff_contents(old_jar, new_jar, prefix):
    """Files under prefix added, removed and changed (by CRC), by kind; changed JSON files with their paths."""
    a, b = jar_entries(old_jar, prefix), jar_entries(new_jar, prefix)
    added, removed, common = keyed_diff(a, b)
    changed = [p for p in common if a[p] != b[p]]
    kinds = {}
    for label, names in [("added", added), ("removed", removed), ("changed", changed)]:
        for name in names:
            kinds.setdefault(content_kind(name), {"added": [], "removed": [], "changed": []})[label].append(name)
    details = {}
    with zipfile.ZipFile(old_jar) as left, zipfile.ZipFile(new_jar) as right:
        for name in changed:
            if name.endswith(".json") and not name.endswith("/lang/en_us.json"):
                try:
                    details[name] = json_diff(json.loads(left.read(name)), json.loads(right.read(name)))
                except ValueError:
                    pass
    return {"kinds": dict(sorted(kinds.items())), "json": details,
            "counts": {"added": len(added), "removed": len(removed), "changed": len(changed)}}


def diff_lang(old_jar, new_jar, name="assets/minecraft/lang/en_us.json"):
    with zipfile.ZipFile(old_jar) as left, zipfile.ZipFile(new_jar) as right:
        a, b = json.loads(left.read(name)), json.loads(right.read(name))
    added, removed, common = keyed_diff(a, b)
    return {"added": added, "removed": removed, "changed": [k for k in common if a[k] != b[k]]}


def read_reports(reports):
    items = {}
    for path in sorted((reports / "minecraft" / "components" / "item").glob("*.json")):
        items[f"minecraft:{path.stem}"] = json.loads(path.read_text())["components"]
    return {
        "packets": json.loads((reports / "packets.json").read_text()),
        "registries": json.loads((reports / "registries.json").read_text()),
        "blocks": json.loads((reports / "blocks.json").read_text()),
        "commands": json.loads((reports / "commands.json").read_text()),
        "items": items,
    }


def class_names(old, new, moved):
    """Every class name of either version, but a moved class only under its new name, so that its simple
    name stays unambiguous."""
    return sorted(set(new) | (set(old) - set(moved)))


def diff_classes(result, mapping):
    """The class diff, each changed or removed class with the Horizon files and harnesses that concern it."""
    horizon = {}
    for name in result["changed"]:
        entry = vanilla_map.lookup(mapping, name)
        if entry["rust"] or entry["harnesses"]:
            horizon[name] = entry
    for name in result["removed"]:
        entry = vanilla_map.lookup(mapping, name)
        if entry["rust"] or entry["harnesses"]:
            horizon[name] = entry
    result["horizon"] = horizon
    return result


def diff_versions(old_paths, new_paths, old_facts, new_facts, classes=None, mapping=None):
    a, b = read_reports(old_paths["reports"]), read_reports(new_paths["reports"])
    classes = diff_classes(classes, mapping) if classes is not None else None
    return {
        "facts": diff_facts(old_facts, new_facts),
        "packets": diff_packets(a["packets"], b["packets"]),
        "registries": diff_registries(a["registries"], b["registries"]),
        "blocks": diff_blocks(a["blocks"], b["blocks"]),
        "items": diff_items(a["items"], b["items"]),
        "commands": diff_commands(a["commands"], b["commands"]),
        "data": diff_contents(old_paths["client"], new_paths["client"], "data/minecraft/"),
        "assets": diff_contents(old_paths["client"], new_paths["client"], "assets/minecraft/"),
        "lang": diff_lang(old_paths["client"], new_paths["client"]),
        "classes": classes,
    }


def summary(old, new, result):
    """A few lines a person reads first."""
    lines = [f"Minecraft {old} -> {new}"]
    for key, (a, b) in result["facts"].items():
        lines.append(f"  {key}: {a} -> {b}")
    for flow, change in result["packets"].items():
        lines.append(f"  packets {flow}: {change['count'][0]} -> {change['count'][1]}, +{len(change['added'])} "
                     f"-{len(change['removed'])}, {len(change['renumbered'])} renumbered")
    registries = result["registries"]
    lines.append(f"  registries: +{len(registries['added'])} -{len(registries['removed'])}, "
                 f"{len(registries['changed'])} changed")
    for name, change in registries["changed"].items():
        lines.append(f"    {name}: +{len(change.get('added', []))} -{len(change.get('removed', []))}"
                     + (f", {change['ids_shifted']} ids shifted" if change.get("ids_shifted") else ""))
    blocks = result["blocks"]
    lines.append(f"  blocks: +{len(blocks['added'])} -{len(blocks['removed'])}, {len(blocks['changed'])} changed, "
                 f"states {blocks['state_count'][0]} -> {blocks['state_count'][1]}, "
                 f"{blocks['state_ids_shifted']} blocks' state ids shifted")
    if blocks["format"]:
        lines.append(f"    report format: fields {', '.join(blocks['format'])} appear or disappear")
    items = result["items"]
    lines.append(f"  items: +{len(items['added'])} -{len(items['removed'])}, {len(items['changed'])} with "
                 f"changed default components")
    ranked = sorted(items["components"].items(), key=lambda kv: -sum(kv[1].values()))
    for component, counts in ranked[:10]:
        lines.append(f"    {component}: " + ", ".join(f"{k} on {n}" for k, n in counts.items() if n))
    commands = result["commands"]
    lines.append(f"  command nodes: +{len(commands['added'])} -{len(commands['removed'])}, "
                 f"{len(commands['changed'])} changed")
    for key in ["data", "assets"]:
        counts = result[key]["counts"]
        lines.append(f"  {key} files: +{counts['added']} -{counts['removed']} ~{counts['changed']}")
    lang = result["lang"]
    lines.append(f"  en_us keys: +{len(lang['added'])} -{len(lang['removed'])} ~{len(lang['changed'])}")
    classes = result.get("classes")
    if classes:
        methods = sum(class_index.changed_methods(c) for c in classes["changed"].values())
        lines.append(f"  classes: +{len(classes['added'])} -{len(classes['removed'])}, {len(classes['moved'])} moved, "
                     f"{len(classes['changed'])} changed ({methods} methods); {len(classes['horizon'])} of the "
                     f"changed or removed ones are ported or proved by Horizon")
    return "\n".join(lines)


def diff_dir(old, new):
    return STORE / f"diff-{safe_version(old)}-{safe_version(new)}"


def load(version, name):
    return json.loads((version_dir(version) / name).read_text())


def diff(args):
    directory = diff_dir(args.old, args.new)
    old_classes, new_classes = load(args.old, "classes.json"), load(args.new, "classes.json")
    classes = class_index.diff(old_classes, new_classes)
    mapping = vanilla_map.build(class_names(old_classes, new_classes, classes["moved"]),
                                dependencies=vanilla_map.recorded_dependencies(vanilla_map.default_shared()))
    lab.write_json(directory / "vanilla_map.json", mapping)
    result = diff_versions(paths(args.old), paths(args.new), load(args.old, "facts.json"), load(args.new, "facts.json"),
                           classes, mapping)
    lab.write_json(directory / "diff.json", result)
    print(summary(args.old, args.new, result))
    print(f"\n{directory / 'diff.json'}")
    return 0


# --- regen -------------------------------------------------------------------------------------------------

def regen(args):
    version = safe_version(args.new)
    result = port_regen.regen(version, paths(version), version_dir(version), args.base,
                              Path(args.worktree) if args.worktree else PORTS / version, args.only)
    lab.write_json(diff_dir(result["old"], version) / "regen.json", result)
    for name, entry in result["generators"].items():
        stat = ", ".join(f"{p} +{a} -{r}" for p, (a, r) in entry.get("stat", {}).items())
        print(f"{name}: {entry['status']}" + (f" ({stat})" if stat else ""))
    print(f"\n{result['candidate']} at {result['commit'][:10]}; {diff_dir(result['old'], version) / 'regen.json'}")
    return 1 if any(e["status"] == "failed" for e in result["generators"].values()) else 0


# --- harnesses ---------------------------------------------------------------------------------------------

def harnesses(args):
    version, old = safe_version(args.new), args.old or pinned()
    directory = diff_dir(old, version)
    previous = json.loads((directory / "harnesses.json").read_text()) if args.no_run else None
    result = port_harness.harnesses(version, version_dir(version),
                                    Path(args.worktree) if args.worktree else PORTS / version,
                                    args.baseline, args.names or None, args.timeout, previous)
    lab.write_json(directory / "harnesses.json", result)
    port_harness.keep_logs(result, directory / "harness-logs")
    counts = result["summary"]
    print(f"horizon-prepare: {result['prepare']['status']}")
    print(f"harnesses: {counts['pass']} pass, {counts['oracle']} oracle no longer compiles, {counts['fail']} fail; "
          f"{counts['new']} newly failing, {counts['known']} failing on the baseline too")
    print(f"{directory / 'harnesses.json'}")
    return 0


# --- report ------------------------------------------------------------------------------------------------

def harness_results(state_path):
    """{harness: "pass" | "fail"} from tools/parity.py's state; a harness fails if one of its shards does."""
    found = {}
    for name, record in json.loads(Path(state_path).read_text()).get("harnesses", {}).items():
        harness = name.split("#")[0]
        found[harness] = "fail" if record.get("status") != "pass" or found.get(harness) == "fail" else "pass"
    return found


def report(args):
    directory = diff_dir(args.old, args.new)
    if not (directory / "diff.json").exists():
        raise SystemExit(f"No diff yet; run version_port.py diff {args.old} {args.new}")
    result = json.loads((directory / "diff.json").read_text())
    mapping = json.loads((directory / "vanilla_map.json").read_text())
    results = harness_results(args.results) if args.results else None
    regenerated, ran = directory / "regen.json", directory / "harnesses.json"
    regenerated = json.loads(regenerated.read_text()) if regenerated.exists() else None
    ran = json.loads(ran.read_text()) if ran.exists() and not args.results else None
    built = port_report.build(result, mapping, args.old, args.new, load(pinned(), "facts.json"), results,
                              regen=regenerated, harnesses=ran)
    lab.write_json(directory / "report.json", built)
    (directory / "report.md").write_text(port_report.markdown(built))
    print("\n".join(port_report.markdown(built).splitlines()[:args.lines]))
    print(f"\n{directory / 'report.md'} ({len(built['items'])} items)")
    return 0


# --- publish -----------------------------------------------------------------------------------------------

def publish(args):
    directory = diff_dir(args.old, args.new)
    if not (directory / "report.json").exists():
        raise SystemExit(f"No report yet; run version_port.py report {args.old} {args.new}")
    built = json.loads((directory / "report.json").read_text())
    result = json.loads((directory / "diff.json").read_text())
    issues = port_publish.open_version_port_issues() if args.yes or args.check_github else {}
    actions = port_publish.plan(built, result, directory, issues)
    creates = sum(1 for a in actions if a[0] == "create")
    if not args.yes:
        for action, name, text, number in actions:
            print(f"would {action}{f' #{number}' if number else ''}: {name}")
        for action, name, text, number in actions[:args.show]:
            print(f"\n--- {name}\n{text}")
        missing = port_publish.missing_labels(args.new) if args.check_github else [port_publish.LABEL,
                                                                                    f"port-{args.new}"]
        print(f"\nDry run: {creates} issues to create, {len(actions) - creates} to update"
              f"{'' if args.check_github else ' (open issues not checked: --check-github)'}; labels to create: "
              f"{', '.join(missing) or 'none'}; then one Kira card for the tracking issue. Rerun with --yes to do it.")
        return 0
    urls = port_publish.apply(actions, built, issues)
    tracking = urls[actions[-1][1]]
    card = port_publish.kira_card(args.new, tracking)
    print(f"Tracking issue {tracking}" + (f"; Kira card {card}" if card else "; Kira card already there"))
    return 0


def port(args):
    """prepare both, diff, regen, report."""
    old = args.old or pinned()
    for version in (old, args.new):
        prepare(argparse.Namespace(version=version, download=False, jobs=args.jobs))
    diff(argparse.Namespace(old=old, new=args.new))
    if not args.no_regen:
        regen(argparse.Namespace(new=args.new, base=args.base, worktree=None, only=None))
    return report(argparse.Namespace(old=old, new=args.new, results=args.results, lines=args.lines))


# --- gc ----------------------------------------------------------------------------------------------------

def to_remove(entries, pin, keep):
    """Version store entries to delete: every version but the pinned one, --keep and the newest two others;
    diffs whose versions are gone; scratch folders. entries: [(name, mtime, is_version)]."""
    versions = sorted(((m, n) for n, m, is_version in entries if is_version and n not in keep | {pin}), reverse=True)
    kept = keep | {pin} | {n for _, n in versions[:2]}
    remove = []
    for name, _, is_version in entries:
        if name.startswith("."):
            remove.append(name)
        elif is_version and name not in kept:
            remove.append(name)
        elif name.startswith("diff-") and not any(name == f"diff-{a}-{b}" for a in kept for b in kept):
            remove.append(name)
    return sorted(remove)


def gc(args):
    if not STORE.exists():
        return 0
    entries = [(p.name, p.stat().st_mtime, (p / "prepared.json").exists()) for p in STORE.iterdir() if p.is_dir()]
    for name in to_remove(entries, pinned(), set(args.keep)):
        print(f"Removing {STORE / name}")
        if not args.dry_run:
            shutil.rmtree(STORE / name)
    return 0


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    commands = parser.add_subparsers(dest="command", required=True)
    command = commands.add_parser("detect", help="what Mojang released that Horizon doesn't pin")
    command.add_argument("--snapshots", action="store_true")
    command.add_argument("--dry-run", action="store_true", help="don't remember what was found")
    command = commands.add_parser("prepare", help="download, verify and run the data generator")
    command.add_argument("version")
    command.add_argument("--download", action="store_true", help="download even the pinned version")
    command.add_argument("-j", "--jobs", type=int, default=int(os.environ.get("FLEET_LOAD_JOBS", 0)) or None,
                         help="class indexing processes (default: $FLEET_LOAD_JOBS, else the CPU count)")
    command = commands.add_parser("inspect", help="check a local client and inspect its version facts")
    command.add_argument("version", help="expected Minecraft version, e.g. 1.20.1")
    command.add_argument("--client", required=True, type=Path, help="path to the local client jar")
    command.add_argument("--metadata", required=True, type=Path, help="path to that version's launcher JSON")
    command.add_argument("--mappings", type=Path, help="optional local client mappings to check")
    command = commands.add_parser("remap-client", help="create a named local client jar for port research")
    command.add_argument("version")
    command.add_argument("--client", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--mappings", required=True, type=Path)
    command.add_argument("--remapper", required=True, type=Path, help="local ForgeAutoRenamingTool jar")
    command.add_argument("--out", required=True, type=Path, help="new output path outside the repository")
    command.add_argument("--java", type=Path, help="Java executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("compare-reports", help="compare common local reports")
    command.add_argument("--old", required=True, type=Path)
    command.add_argument("--new", required=True, type=Path)
    command = commands.add_parser("legacy-packets", help="export local 1.20.1 packet classes and IDs")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--named-server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--out", required=True, type=Path, help="new JSON file outside the repository")
    command.add_argument("--java", type=Path, help="JDK executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("legacy-block-facts", help="prepare local 1.20.1 block data for the Rust registry")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--named-server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--blocks", required=True, type=Path, help="blocks.json from local-reports")
    command.add_argument("--out", required=True, type=Path, help="new JSON file outside the repository")
    command.add_argument("--java", type=Path, help="JDK executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("legacy-dynamic-states", help="inventory uncached 1.20.1 states")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--named-server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--blocks", required=True, type=Path)
    command.add_argument("--out", required=True, type=Path)
    command.add_argument("--java", type=Path)
    command = commands.add_parser("legacy-collision", help="export and validate local 1.20.1 collision data")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--named-server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--blocks", required=True, type=Path)
    command.add_argument("--facts", required=True, type=Path)
    command.add_argument("--out", required=True, type=Path)
    command.add_argument("--java", type=Path)
    command.add_argument("--cargo", default="cargo")
    command = commands.add_parser("legacy-heightmap", help="export and validate local 1.20.1 terrain heightmaps")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--named-server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--blocks", required=True, type=Path)
    command.add_argument("--facts", required=True, type=Path)
    command.add_argument("--out", required=True, type=Path)
    command.add_argument("--java", type=Path)
    command.add_argument("--cargo", default="cargo")
    command = commands.add_parser("legacy-gameplay-facts", help="prepare local 1.20.1 block properties")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--named-server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--blocks", required=True, type=Path, help="blocks.json from local-reports")
    command.add_argument("--facts", required=True, type=Path, help="file from legacy-block-facts")
    command.add_argument("--out", required=True, type=Path, help="new JSON file outside the repository")
    command.add_argument("--java", type=Path, help="JDK executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("legacy-behaviour-facts", help="prepare local 1.20.1 block behaviour data")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--named-server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--blocks", required=True, type=Path, help="blocks.json from local-reports")
    command.add_argument("--facts", required=True, type=Path, help="file from legacy-block-facts")
    command.add_argument("--out", required=True, type=Path, help="new JSON file outside the repository")
    command.add_argument("--java", type=Path, help="JDK executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("legacy-prepare-block-data", help="prepare local 1.20.1 block data together")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--mappings", required=True, type=Path)
    command.add_argument("--remapper", required=True, type=Path)
    command.add_argument("--out", required=True, type=Path, help="new folder outside the repository")
    command.add_argument("--java", type=Path, help="JDK executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("remap-server", help="create a named local server jar for port research")
    command.add_argument("version")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--mappings", required=True, type=Path)
    command.add_argument("--remapper", required=True, type=Path, help="local ForgeAutoRenamingTool jar")
    command.add_argument("--out", required=True, type=Path, help="new output path outside the repository")
    command.add_argument("--java", type=Path, help="Java executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("local-reports", help="generate reports from a local checked server jar")
    command.add_argument("version")
    command.add_argument("--server", required=True, type=Path)
    command.add_argument("--metadata", required=True, type=Path)
    command.add_argument("--out", required=True, type=Path, help="new folder outside the repository")
    command.add_argument("--java", type=Path, help="Java executable (default: JAVA_HOME or PATH)")
    command = commands.add_parser("diff", help="diff two prepared versions")
    command.add_argument("old")
    command.add_argument("new")
    command = commands.add_parser("regen", help="a candidate worktree pinned to NEW, every table regenerated")
    command.add_argument("new")
    command.add_argument("--base", default="origin/main", help="what port/NEW starts from (default origin/main)")
    command.add_argument("--worktree", help="where (default: $HORIZON_PORTS/NEW)")
    command.add_argument("--only", nargs="*", help="these generators only (names in tools/port_regen.py)")
    command = commands.add_parser("harnesses", help="horizon-prepare and every harness in the candidate")
    command.add_argument("new")
    command.add_argument("names", nargs="*", help="these harnesses only (default: all)")
    command.add_argument("--old", help="the version the port starts from (default: the pinned one)")
    command.add_argument("--worktree", help="the candidate (default: $HORIZON_PORTS/NEW)")
    command.add_argument("--baseline", help="tools/parity.py's state.json from a run on the pinned version: what "
                                            "fails there too is not newly failing")
    command.add_argument("--timeout", type=int, help="per harness, seconds (tools/parity.py --timeout)")
    command.add_argument("--no-run", action="store_true", help="classify the last run again (another --baseline)")
    command = commands.add_parser("report", help="the ranked port report of a diff")
    command.add_argument("old")
    command.add_argument("new")
    command.add_argument("--results", help="tools/parity.py's state.json from a run against NEW (default: "
                                           "harnesses.json, when version_port.py harnesses ran)")
    command.add_argument("--lines", type=int, default=60, help="how much of report.md to print")
    command = commands.add_parser("publish", help="the report's work items as GitHub issues (dry run by default)")
    command.add_argument("old")
    command.add_argument("new")
    command.add_argument("--yes", action="store_true", help="create and update the issues, labels and Kira card")
    command.add_argument("--check-github", action="store_true",
                         help="dry run: read the open issues and labels to tell creates from updates")
    command.add_argument("--show", type=int, default=1, help="dry run: print this many issue bodies")
    command = commands.add_parser("port", help="prepare, diff and report in one go")
    command.add_argument("new")
    command.add_argument("--old", help="default: the pinned version")
    command.add_argument("--results", help="tools/parity.py's state.json from a run against NEW")
    command.add_argument("--lines", type=int, default=60)
    command.add_argument("--base", default="origin/main", help="what the candidate starts from")
    command.add_argument("--no-regen", action="store_true", help="diff and report only (snapshots)")
    command.add_argument("-j", "--jobs", type=int, default=int(os.environ.get("FLEET_LOAD_JOBS", 0)) or None)
    command = commands.add_parser("gc", help="delete versions and scratch the store doesn't need")
    command.add_argument("--keep", nargs="*", default=[])
    command.add_argument("--dry-run", action="store_true")
    args = parser.parse_args()
    if args.command not in {"inspect", "remap-client", "remap-server", "local-reports", "compare-reports",
                            "legacy-packets", "legacy-block-facts", "legacy-gameplay-facts", "legacy-behaviour-facts",
                            "legacy-prepare-block-data", "legacy-dynamic-states", "legacy-collision", "legacy-heightmap"}:
        STORE.mkdir(parents=True, exist_ok=True)
    return {"detect": detect, "prepare": prepare, "inspect": inspect, "remap-client": remap_client,
            "remap-server": remap_server, "local-reports": make_local_reports,
            "compare-reports": compare_reports,
            "legacy-packets": make_legacy_packets,
            "legacy-block-facts": make_legacy_block_facts,
            "legacy-dynamic-states": make_legacy_dynamic_states,
            "legacy-collision": make_legacy_collision,
            "legacy-heightmap": make_legacy_heightmap,
            "legacy-gameplay-facts": make_legacy_gameplay_facts,
            "legacy-behaviour-facts": make_legacy_behaviour_facts,
            "legacy-prepare-block-data": make_legacy_block_data,
            "diff": diff, "regen": regen, "harnesses": harnesses,
            "report": report, "publish": publish, "port": port, "gc": gc}[args.command](args)


if __name__ == "__main__":
    sys.exit(main())
