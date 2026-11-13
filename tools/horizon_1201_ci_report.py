#!/usr/bin/env python3
"""Summarise Horizon 1.20.1 CI failures without changing any failing gate.

Only tracked *paths* and Rust compiler diagnostics are reported. No game assets,
prepared data, JARs, mappings, or decompiled source files are read or uploaded.
"""

from __future__ import annotations

import argparse
import os
import re
import subprocess
import sys
from pathlib import Path
from typing import Callable, Iterable

ERROR = re.compile(
    r"^\s*(?:error(?:\[[A-Z]\d+\])?:|fatal:|Caused by:|"
    r"error occurred:|failed to (?:get|load|parse|select|run|compile|resolve|download)\b|"
    r"test result:\s*FAILED\b|test\s+\S+\s+\.\.\.\s+FAILED\b|"
    r"thread ['\"].+?['\"] panicked at|process didn't exit successfully)",
    re.IGNORECASE,
)
# Cargo enables ANSI colours when its output is captured via `tee` in CI.
# Strip CSI escapes before matching error[E0502] or displaying excerpts.
ANSI_ESCAPE = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")
MAX_ITEMS = 12
MAX_LINE = 300
TAIL_LINES = 24


def rejected_paths(paths: Iterable[str], is_prohibited: Callable[[str], bool]) -> list[str]:
    """Use the canonical gate's policy; NEVER create a second, weaker allowlist."""
    return sorted({path for path in paths if path and is_prohibited(path)})


def tracked_rejections() -> list[str]:
    # Import the EXACT same function that fails the mandatory public-tree gate.
    from check_public_tree import prohibited

    output = subprocess.check_output(["git", "ls-files", "-z"])
    return rejected_paths(output.decode("utf-8").split("\0"), prohibited)


def md(text: str) -> str:
    """Display source-controlled paths and errors as inert Markdown text."""
    return ANSI_ESCAPE.sub("", text).replace("`", "'").replace("\r", " ").replace("\n", " ")[:MAX_LINE]


def public_tree_section(paths: list[str]) -> str:
    if not paths:
        return "## Private game file gate\n\nRejected tracked paths: **0**.\n"
    lines = [
        "## Private game file gate",
        "",
        f"Rejected tracked paths: **{len(paths)}**. The gate remains mandatory.",
        "Review these paths before deletion or a narrowly reviewed policy change:",
        "",
    ]
    lines.extend(f"- `{md(path)}`" for path in paths[:80])
    if len(paths) > 80:
        lines.append(f"- ... {len(paths)-80} additional paths omitted")
    lines.append("")
    return "\n".join(lines)


def error_excerpts(log: str, max_items: int = MAX_ITEMS) -> list[str]:
    lines = [ANSI_ESCAPE.sub("", line) for line in log.splitlines()]
    excerpts: list[str] = []
    for index, line in enumerate(lines):
        if not ERROR.match(line):
            continue
        excerpt = "\n".join(md(value) for value in lines[index:index+3])
        if excerpt not in excerpts:
            excerpts.append(excerpt)
        if len(excerpts) >= max_items:
            break
    return excerpts


def failure_tail(log: str, limit: int = TAIL_LINES) -> list[str]:
    """Provide bounded, useful context when Cargo fails without error: lines."""
    return [md(line) for line in log.splitlines() if line.strip()][-limit:]


def rust_section(label: str, log: str | None, status: str) -> str:
    result = "passed" if status == "0" else "failed" if status.isdigit() else "unknown (setup may have failed)"
    lines = [f"### Rust {md(label)}: {result}", ""]
    if not log:
        lines.append("No compiler log was produced. Inspect checkout/toolchain/cache setup steps.")
    else:
        errors = error_excerpts(log)
        if errors:
            lines.append(f"First {len(errors)} distinct error locations (full log in CI artifact):")
            lines.append("")
            for block in errors:
                lines.extend(["```text", block, "```", ""])
        elif result != "passed":
            lines.append("No recognised compiler/test error marker. Last non-empty log lines:")
            lines.extend(["", "```text", *failure_tail(log), "```", ""])
        else:
            lines.append("No error markers in the successful Rust log.")
    lines.append("")
    return "\n".join(lines)


def append_summary(text: str) -> None:
    print(text)
    destination = os.environ.get("GITHUB_STEP_SUMMARY")
    if destination:
        with open(destination, "a", encoding="utf-8") as stream:
            stream.write(text + "\n")


def parse_log_argument(raw: str) -> tuple[str, Path]:
    if ":" not in raw:
        raise ValueError("--rust-log must be LABEL:PATH")
    label, path = raw.split(":", 1)
    if not label or not path:
        raise ValueError("--rust-log must be LABEL:PATH")
    return label, Path(path)


def parse_status(raw: str) -> tuple[str, str]:
    if "=" not in raw:
        raise ValueError("--status must be LABEL=EXIT_CODE")
    return tuple(raw.split("=", 1))  # type: ignore[return-value]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--public-tree", action="store_true")
    parser.add_argument("--rust-log", action="append", default=[], metavar="LABEL:PATH")
    parser.add_argument("--status", action="append", default=[], metavar="LABEL=EXIT_CODE")
    args = parser.parse_args(argv)
    try:
        if args.public_tree:
            append_summary(public_tree_section(tracked_rejections()))
        status_by_label = dict(parse_status(s) for s in args.status)
        if args.rust_log:
            append_summary("## Rust compiler / test diagnostics\n")
        for raw in args.rust_log:
            label, path = parse_log_argument(raw)
            log = path.read_text(encoding="utf-8", errors="replace") if path.is_file() else None
            append_summary(rust_section(label, log, status_by_label.get(label, "")))
        return 0
    except (OSError, UnicodeError, ValueError, subprocess.CalledProcessError) as exc:
        # Diagnostic reporting never masks or replaces the original CI gate.
        print(f"Diagnostic report could not be generated: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
