"""Read class names from a local Mojang mapping for development, without publishing it.

These are class names only. Member names, inheritance and bytecode still need a
real remapping stage; this module must not be used to mark a port as complete.
"""
import re

CLASS = re.compile(r"([^\s]+) -> ([^\s]+):")


def read_classes(lines):
    """Map obfuscated JVM class names to named JVM class names."""
    by_obfuscated, by_named = {}, {}
    for number, line in enumerate(lines, 1):
        if not line.strip() or line[0].isspace() or line.startswith("#"):
            continue
        match = CLASS.fullmatch(line.rstrip("\r\n"))
        if not match:
            raise ValueError(f"Invalid class mapping on line {number}")
        named, obfuscated = (part.replace(".", "/") for part in match.groups())
        if obfuscated in by_obfuscated or named in by_named:
            raise ValueError(f"Duplicate class mapping on line {number}")
        by_obfuscated[obfuscated] = named
        by_named[named] = obfuscated
    if not by_obfuscated:
        raise ValueError("No class mappings found")
    return by_obfuscated
