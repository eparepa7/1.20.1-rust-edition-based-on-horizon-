#!/usr/bin/env python3
"""Audit creative tabs, recipes, command registries, and renamed IDs against Minecraft 1.20.1 registries.json."""

from __future__ import annotations

import argparse
import json
import os
import re
import sys
from pathlib import Path

KNOWN_RENAMES_1201_TO_263 = {
    "minecraft:grass": "minecraft:short_grass",
    "minecraft:scute": "minecraft:turtle_scute",
    "minecraft:chain": "minecraft:iron_chain",
}

BAD_SNBT_MARKERS = (
    "minecraft:wind_charged",
    "minecraft:long_wind_charged",
    "minecraft:weaving",
    "minecraft:long_weaving",
    "minecraft:oozing",
    "minecraft:long_oozing",
    "minecraft:infested",
    "minecraft:long_infested",
    "minecraft:breach",
    "minecraft:density",
    "minecraft:wind_burst",
    "minecraft:lunge",
)

def strip_components(stack_str: str) -> str:
    s = stack_str.strip()
    m = re.search(r'(?:^|[,{])id:"([^"]+)"', s)
    if m:
        return m.group(1)
    m = re.match(r'^([a-z0-9_.-]+:[a-z0-9_./-]+)', s)
    if m:
        return m.group(1)
    return s.split('[', 1)[0].split('{', 1)[0].strip()

def find_default_resources(repo: Path) -> Path:
    p1201 = repo / ".local" / "horizon-client" / "1.20.1"
    if (p1201 / "creative-inventory.json").exists():
        return p1201
    appdata = os.environ.get("APPDATA", "")
    if appdata:
        versions_dir = Path(appdata) / "horizon-1201-dev-launcher" / "versions"
        if versions_dir.exists():
            for cand in sorted(versions_dir.iterdir(), reverse=True):
                res = cand / ".local" / "horizon-client" / "26.3"
                if (res / "creative-inventory.json").exists():
                    return res
    return p1201

def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--registries", default=None, help="Path to 1.20.1 registries.json")
    parser.add_argument("--resources", default=None, help="Path to prepared horizon-client resources")
    parser.add_argument("--strict", action="store_true", help="Exit non-zero if any extra/unmapped items exist")
    args = parser.parse_args()

    repo = Path(__file__).resolve().parent.parent
    reg_path = Path(args.registries) if args.registries else repo / ".local" / "minecraft" / "1.20.1" / "generated" / "reports" / "registries.json"
    res_dir = Path(args.resources) if args.resources else find_default_resources(repo)

    if not reg_path.exists():
        print(f"ERROR: registries.json not found at {reg_path}", file=sys.stderr)
        return 2
    if not res_dir.exists():
        print(f"ERROR: resources dir not found at {res_dir}", file=sys.stderr)
        return 2

    reg = json.loads(reg_path.read_text(encoding="utf-8"))
    ref_blocks = set(reg["minecraft:block"]["entries"].keys())
    ref_items = set(reg["minecraft:item"]["entries"].keys())

    print(f"Reference 1.20.1 ({reg_path}):")
    print(f"  minecraft:block = {len(ref_blocks)}")
    print(f"  minecraft:item  = {len(ref_items)}")
    print(f"Inspecting resources: {res_dir}")

    creative_items = set()
    extra_creative_items = set()
    bad_snbt_count = 0
    combat_items = []
    combat_extra = []
    nautilus_shell_tabs = []

    ci_path = res_dir / "creative-inventory.json"
    if ci_path.exists():
        ci = json.loads(ci_path.read_text(encoding="utf-8"))
        for tab in ci.get("tabs", []):
            tab_id = tab.get("id", "")
            for raw in tab.get("items", []):
                item_id = strip_components(raw)
                if any(m in raw for m in BAD_SNBT_MARKERS):
                    bad_snbt_count += 1
                if item_id == "minecraft:nautilus_shell":
                    nautilus_shell_tabs.append(tab_id)
                if tab_id == "minecraft:combat":
                    combat_items.append(item_id)
                    if item_id not in ref_items or any(m in raw for m in BAD_SNBT_MARKERS):
                        combat_extra.append(item_id)
                if tab_id != "minecraft:op_blocks":
                    creative_items.add(item_id)
                    if item_id not in ref_items:
                        extra_creative_items.add(item_id)

    cmd_extra_items = set()
    cmd_extra_blocks = set()
    cr_path = res_dir / "command-registries.json"
    if cr_path.exists():
        cr = json.loads(cr_path.read_text(encoding="utf-8"))
        for it in cr.get("items", []):
            if it not in ref_items:
                cmd_extra_items.add(it)
        for bl in cr.get("blocks", []):
            if bl not in ref_blocks:
                cmd_extra_blocks.add(bl)

    recipe_extra = []
    rec_dir = res_dir / "data" / "minecraft" / "recipe"
    recipe_count = 0
    if rec_dir.is_dir():
        for fp in rec_dir.glob("*.json"):
            recipe_count += 1
            r = json.loads(fp.read_text(encoding="utf-8"))
            res = r.get("result")
            res_id = res.get("id") or res.get("item") if isinstance(res, dict) else (res if isinstance(res, str) else None)
            if res_id and res_id not in ref_items:
                recipe_extra.append((fp.name, res_id))

    renamed_present = []
    for old_id, new_id in KNOWN_RENAMES_1201_TO_263.items():
        if new_id in creative_items or new_id in cmd_extra_items:
            renamed_present.append((old_id, new_id))

    print(f"\nCreative Inventory (all tabs):")
    print(f"  unique items = {len(creative_items)}")
    print(f"  extra items not in 1.20.1 = {len(extra_creative_items)}")
    print(f"  1.21+ potion/enchant SNBT entries = {bad_snbt_count}")
    print(f"  Combat tab items = {len(combat_items)}, extra = {len(combat_extra)}")
    print(f"  nautilus_shell tabs = {nautilus_shell_tabs}")
    print(f"\nCommand Registries:")
    print(f"  extra items = {len(cmd_extra_items)}, extra blocks = {len(cmd_extra_blocks)}")
    print(f"\nRecipes ({rec_dir}):")
    print(f"  total = {recipe_count}, extra results not in 1.20.1 = {len(recipe_extra)}")
    print(f"\nRenamed 26.3 IDs remaining in creative/commands: {len(renamed_present)}")
    for old_id, new_id in renamed_present:
        print(f"  - {old_id} -> {new_id}")

    # Check all 1.20.1 items (except air) have item definitions, models, and textures
    assets_mc = res_dir / "assets" / "minecraft"
    missing_defs = []
    broken_assets = []
    model_cache: dict[str, tuple[list[str], bool]] = {}

    def resolve_textures(model_id: str, seen: set[str] | None = None) -> tuple[list[str], bool]:
        if model_id in model_cache:
            return model_cache[model_id]
        bare = model_id.removeprefix("minecraft:")
        if bare.startswith("builtin/"):
            return [], True
        if seen is None:
            seen = set()
        if model_id in seen:
            return [], True
        seen.add(model_id)
        fp = assets_mc / "models" / f"{bare}.json"
        if not fp.is_file():
            return [f"MISSING_MODEL:{model_id}"], False
        m = json.loads(fp.read_text(encoding="utf-8"))
        miss: list[str] = []
        ok = True
        if "parent" in m:
            pm, pok = resolve_textures(m["parent"], seen)
            miss.extend(pm)
            ok = ok and pok
        for v in m.get("textures", {}).values():
            if v.startswith("#"):
                continue
            t_rel = v.removeprefix("minecraft:")
            # Armor trim paletted permutations are generated at runtime from atlases/items.json
            if t_rel.startswith("trims/items/"):
                continue
            tp = assets_mc / "textures" / f"{t_rel}.png"
            if not tp.is_file():
                miss.append(f"MISSING_TEX:{v}")
                ok = False
        res = (miss, ok)
        model_cache[model_id] = res
        return res

    def collect_model_refs(node: object) -> list[str]:
        refs: list[str] = []
        if isinstance(node, dict):
            if node.get("type") in ("minecraft:model", "model") and "model" in node:
                refs.append(node["model"])
            for val in node.values():
                refs.extend(collect_model_refs(val))
        elif isinstance(node, list):
            for val in node:
                refs.extend(collect_model_refs(val))
        return refs

    if (assets_mc / "items").is_dir():
        for item_id in sorted(ref_items):
            if item_id == "minecraft:air":
                continue
            name = item_id.removeprefix("minecraft:")
            def_p = assets_mc / "items" / f"{name}.json"
            if not def_p.is_file():
                missing_defs.append(item_id)
                continue
            d = json.loads(def_p.read_text(encoding="utf-8"))
            for r in collect_model_refs(d):
                miss, ok = resolve_textures(r)
                if not ok:
                    broken_assets.append((item_id, r, miss))

    missing_bundle_lang = []
    lang_p = assets_mc / "lang" / "en_us.json"
    if lang_p.is_file():
        lang_d = json.loads(lang_p.read_text(encoding="utf-8"))
        for k in ("item.minecraft.bundle.full", "item.minecraft.bundle.empty"):
            if k not in lang_d:
                missing_bundle_lang.append(k)

    print(f"\nItem Assets (1,254 non-air 1.20.1 items):")
    print(f"  missing items/*.json = {len(missing_defs)}, broken models/textures = {len(broken_assets)}")
    for b in broken_assets[:20]:
        print(f"  - {b}")
    print(f"  missing bundle lang keys = {missing_bundle_lang}")

    has_issues = bool(
        extra_creative_items
        or bad_snbt_count
        or combat_extra
        or cmd_extra_items
        or cmd_extra_blocks
        or recipe_extra
        or renamed_present
        or missing_defs
        or broken_assets
        or missing_bundle_lang
    )
    if args.strict and has_issues:
        return 1
    return 0

if __name__ == "__main__":
    raise SystemExit(main())
