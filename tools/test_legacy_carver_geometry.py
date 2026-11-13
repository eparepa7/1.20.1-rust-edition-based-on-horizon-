#!/usr/bin/env python3
"""Compare 1.20.1 trig and ordered candidate coordinates, not terrain replacement."""
import argparse
import copy
import json
import os
from pathlib import Path
import struct
import subprocess
import tempfile
import zipfile

try:
    from tools import lab, version_port as port
except ModuleNotFoundError:
    import lab
    import version_port as port


def random_words(count):
    state = 0x123456789abcdef0
    for _ in range(count):
        state = (state * 6364136223846793005 + 1442695040888963407) & ((1 << 64)-1)
        yield state


def angles():
    result = [0, 0x80000000, 0x7f800000, 0xff800000, 0x7fc00000, 0x7f800001,
              0xffc00001, 1, 0x80000001, 0x7f7fffff, 0xff7fffff]
    scale = struct.unpack(">f", struct.pack(">f", 10430.378))[0]
    for index in range(-65537, 65538, 31):
        bits = struct.unpack(">I", struct.pack(">f", index / scale))[0]
        result.extend([(bits-1) & 0xffffffff, bits, (bits+1) & 0xffffffff])
    result.extend(word & 0xffffffff for word in random_words(16384))
    return result


def fixtures(names):
    seeds = [0,1,-1,-(1<<63),(1<<63)-1,0x12345678]
    seeds += [word if word < 1<<63 else word-(1<<64) for word in random_words(18)]
    scenes = [([0,0],[0,0],-64,384,False), ([-3,5],[-2,5],0,128,False),
              ([7,-11],[7,-12],-64,384,True), ([0,0],[8,-8],-64,384,False),
              ([1874999,-1874999],[1874998,-1874999],-64,384,False),
              ([-11,7],[-11,7],0,16,False), ([-11,7],[-11,7],0,16,True)]
    result = []
    for name in sorted(names):
        for seed in seeds:
            for target, origin, minimum, height, upgrading in scenes:
                result.append(dict(carver=name,seed=seed,target=target,origin=origin,min=minimum,
                                   height=height,upgrading=upgrading,select_start=False))
        for word in random_words(64):
            seed = word if word < 1<<63 else word-(1<<64)
            result.append(dict(carver=name,seed=seed,target=[0,0],origin=[0,0],min=-64,
                               height=384,upgrading=False,select_start=True))
    return result


def compare_bytes(reference, actual):
    with reference.open("rb") as left, actual.open("rb") as right:
        offset = 0
        while True:
            a, b = left.read(1<<20), right.read(1<<20)
            if a != b:
                common = min(len(a),len(b))
                first = next((i for i in range(common) if a[i]!=b[i]),common)
                raise ValueError(f"Java/Rust binary trace differs at byte {offset+first}: Java size={reference.stat().st_size}, Rust size={actual.stat().st_size}")
            if not a:
                return offset
            offset += len(a)


def geometry_counts(path, cases):
    data = path.read_bytes()
    if data[:4] != b"LCG1":
        raise ValueError("Missing geometry trace header")
    offset = 4
    total = repeats = active = nonempty = 0
    by_carver = {fixture["carver"]: 0 for fixture in cases}
    lower = upper = 0
    for case_id, fixture in enumerate(cases):
        actual_id, selected = struct.unpack_from(">IB",data,offset); offset += 5
        if actual_id != case_id or selected not in (0,1):
            raise ValueError("Invalid fixture frame")
        active += selected
        visits = 0; seen = set()
        while True:
            point, = struct.unpack_from(">i",data,offset); offset += 4
            if point == -1: break
            y = (point >> 8) + fixture["min"]
            top = fixture["min"] + fixture["height"] - 1 - (0 if fixture["upgrading"] else 7)
            if point < 0 or not fixture["min"]+1 < y <= top:
                raise ValueError("Out-of-range geometry point")
            lower += y == fixture["min"]+2
            upper += y == top
            visits += 1; seen.add(point)
        count, result, tail = struct.unpack_from(">IBq",data,offset); offset += 13
        if count != visits or result != selected or (not selected and visits):
            raise ValueError("Invalid fixture count/result")
        total += visits; repeats += visits-len(seen); nonempty += bool(visits)
        by_carver[fixture["carver"]] += visits
    if offset != len(data) or not total or not repeats or not lower or not upper or not all(by_carver.values()):
        raise ValueError("Incomplete geometry/boundary/debug-repeat coverage")
    return dict(cases=len(cases),selected=active,nonempty=nonempty,candidate_visits=total,
                debug_revisits=repeats,lower_boundary_visits=lower,upper_boundary_visits=upper,
                visits_by_carver=by_carver)


def run(args):
    _, version = port.verified_local_server("1.20.1",args.server,args.metadata)
    if port.jar_facts(args.named_server) != version: raise ValueError("Named server version mismatch")
    with args.facts.open(encoding="utf-8") as stream:
        states, _ = port.validate_legacy_block_facts(json.load(stream), json.loads(args.blocks.read_text(encoding="utf-8")))
    java = port.selected_java(args.java,17)
    with tempfile.TemporaryDirectory(prefix="horizon-1201-geometry-") as directory:
        scratch = Path(directory)
        game, libraries = lab.unpack_server(args.server.resolve(),scratch,"1.20.1")
        bundles = scratch / "worldgen"; bundles.mkdir()
        names = None
        with zipfile.ZipFile(game) as archive:
            for name,resource in [("carvers","worldgen/configured_carver"),("block_tags","tags/blocks")]:
                prefix = f"data/minecraft/{resource}/"
                entries = {"minecraft:"+path[len(prefix):-5]:archive.read(path).decode("utf-8") for path in archive.namelist() if path.startswith(prefix) and path.endswith(".json")}
                if name == "carvers":
                    if len(entries)!=4: raise ValueError("Expected four vanilla carvers")
                    for original in ("minecraft:cave","minecraft:canyon"):
                        variant = copy.deepcopy(json.loads(entries[original]))
                        variant["config"]["debug_settings"]["debug_mode"] = True
                        entries["test:"+original.split(":")[1]+"_debug"] = json.dumps(variant)
                    names = entries.keys()
                (bundles/(name+".json")).write_text(json.dumps(entries),encoding="utf-8")
        angle_bits = angles(); cases = fixtures(names)
        angle_path = scratch/"angles.json"; angle_path.write_text(json.dumps(angle_bits),encoding="utf-8")
        case_path = scratch/"cases.json"; case_path.write_text(json.dumps(cases),encoding="utf-8")
        classpath = os.pathsep.join(map(str,[args.named_server.resolve(),*libraries]))
        sources = [str(lab.ROOT/"tools"/(name+".java")) for name in ("Legacy1201CarverConfigOracle","Legacy1201CarverGeometryOracle")]
        subprocess.run([java,"--module","jdk.compiler/com.sun.tools.javac.Main","--release","17","-cp",classpath,"-d",str(scratch),*sources],cwd=scratch,check=True,capture_output=True,text=True)
        oracle_cp = os.pathsep.join([str(scratch),classpath])
        for mode, inputs in [("trig",[str(angle_path)]),("geometry",[str(bundles),str(case_path)])]:
            reference = scratch/(mode+"-java.bin"); actual = scratch/(mode+"-rust.bin")
            subprocess.run([java,"-cp",oracle_cp,"Legacy1201CarverGeometryOracle",mode,*inputs,str(reference)],cwd=scratch,check=True,capture_output=True,text=True)
            rust_inputs = [str(args.facts.resolve()),*inputs] if mode=="geometry" else inputs
            subprocess.run([args.cargo,"run","--quiet","-p","horizon-world","--example","legacy_carver_geometry_probe","--",mode,*rust_inputs,str(actual)],cwd=lab.ROOT,check=True,capture_output=True,text=True)
            byte_count = compare_bytes(reference,actual)
            if mode=="trig":
                if byte_count != 4+65536*4+len(angle_bits)*12: raise ValueError("Incomplete trig trace")
                trig_result = dict(table_entries=65536,angle_cases=len(angle_bits),matched_bytes=byte_count)
            else:
                geometry_result = geometry_counts(reference,cases)
                geometry_result["matched_bytes"] = byte_count
        return dict(version="1.20.1",states=states,trig=trig_result,geometry=geometry_result,
                    scope="exact candidate sequence and outer RNG tails; block replacement, aquifers and terrain generation are not exercised")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("server","metadata","named-server","facts","blocks"):
        parser.add_argument("--"+name,required=True,type=Path)
    parser.add_argument("--java"); parser.add_argument("--cargo",default="cargo")
    try:
        result = run(parser.parse_args())
    except subprocess.CalledProcessError as error:
        raise SystemExit("Geometry harness subprocess failed:\n"+(error.stderr+error.stdout)[-4000:]) from None
    print(json.dumps(result,indent=2))


if __name__=="__main__": main()
