#!/usr/bin/env python3
"""Reject local game artifacts even when somebody uses git add -f."""

import argparse
from pathlib import PurePosixPath
import subprocess
import sys
PRIVATE_ROOTS = {".local", "decompiled", "minecraft", "assets", "run", "logs"}
PRIVATE_EXTENSIONS = {".jar", ".class", ".jfr", ".hprof", ".java"}
ORIGINAL_JDK_TOOLS = {
    "tools/JfrSummary.java",
    "tools/Legacy1201BlockFactsExporter.java",
    "tools/Legacy1201CarverConfigOracle.java",
    "tools/Legacy1201CarverGeometryOracle.java",
    "tools/Legacy1201DynamicStatesExporter.java",
    "tools/Legacy1201CollisionExporter.java",
    "tools/Legacy1201CachedCollisionOracle.java",
    "tools/Legacy1201DynamicCollisionOracle.java",
    "tools/Legacy1201EntityCollisionOracle.java",
    "tools/Legacy1201OutlineOracle.java",
    "tools/Legacy1201LiquidCollisionOracle.java",
    "tools/Legacy1201ContextCollisionOracle.java",
    "tools/Legacy1201AllContextOracle.java",
    "tools/Legacy1201SceneMovementOracle.java", "tools/Legacy1201SceneEntityMovementOracle.java", "tools/Legacy1201HeightmapOracle.java", "tools/Legacy1201HeightmapExporter.java", "tools/Legacy1201HeightmapChunkOracle.java", "tools/Legacy1201HeightmapColumnsOracle.java",
    "tools/Legacy1201BlockBehaviourExporter.java",
    "tools/Legacy1201GameplayFactsExporter.java",
    "tools/Legacy1201PacketProbe.java",
    "tests/java/JfrFixture.java",
    # These two original, project-authored 1.20.1 oracle harnesses are tracked
    # under .github/tools/, not tools/. Allow precisely these full paths.
    # All other Java source, decompiled code, JARs, and private data stay denied.
    ".github/tools/Legacy1201CarverConfigOracle.java",
    ".github/tools/Legacy1201CarverGeometryOracle.java",
}
# The prepared folder's files in one file (crates/horizon-core/src/prepared_files.rs): Mojang's data.
PRIVATE_NAMES = {"horizon-data.bin"}

def prohibited(path):
    parsed = PurePosixPath(path)
    # Original JDK-only tools are an exact allowlist; recovered game Java stays private.
    if path in ORIGINAL_JDK_TOOLS:
        return False
    # Original Horizon mod sources may also be tracked.
    if parsed.suffix == ".java" and path.startswith(("mods/horizon/src/", "mods/horizon-lab/src/")):
        return not ("/dev/horizonlab/" in path)
    return (
        bool(parsed.parts and parsed.parts[0] in PRIVATE_ROOTS)
        or parsed.suffix.lower() in PRIVATE_EXTENSIONS
        or parsed.name in PRIVATE_NAMES
    )

def paths_in_repository(history=False, revisions=None):
    if not history:
        result = subprocess.check_output(["git", "ls-files", "-z"])
        return result.decode("utf-8").split("\0")
    # An empty push (including deletions only) must not fall back to HEAD or --all.
    if revisions is not None and not revisions:
        return []
    roots = ["--all"] if revisions is None else revisions
    # Include diffs against merge parents so merge-resolution-only artifacts are caught.
    # Git pushes the stored objects, not the history presented by local replace refs.
    result = subprocess.check_output(["git", "--no-replace-objects", "log", "--format=", "--name-only",
                                      "-z", "-m", *roots, "--"])
    return [path.lstrip("\n") for path in result.decode("utf-8").split("\0")]

def pushed_revisions(stream):
    """Read Git's pre-push ref updates and retain the non-deleted local object IDs."""
    revisions = []
    for line in stream:
        _local_ref, local_oid, _remote_ref, _remote_oid = line.split()
        if local_oid.strip("0"):
            revisions.append(local_oid)
    return revisions

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--history", action="store_true", help="Inspect paths in all local Git history")
    mode.add_argument("--pre-push", action="store_true", help="Inspect history reachable from Git's ref updates on stdin")
    # CI's pull requests: only the commits a merge would add, not every ref's whole history.
    mode.add_argument("--range", metavar="BASE..HEAD", help="Inspect the commits of a revision range")
    args = parser.parse_args()
    revisions = pushed_revisions(sys.stdin) if args.pre_push else [args.range] if args.range else None
    history = args.history or args.pre_push or bool(args.range)
    rejected = sorted({path for path in paths_in_repository(history, revisions)
                       if path and prohibited(path)})
    if rejected:
        print("Private game/profiling artifacts must stay outside Git:", file=sys.stderr)
        for path in rejected:
            print(f"  {path}", file=sys.stderr)
        return 1
    print("Public tree check passed.")
    return 0

if __name__ == "__main__":
    sys.exit(main())
