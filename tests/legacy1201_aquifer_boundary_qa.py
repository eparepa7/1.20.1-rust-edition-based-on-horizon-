#!/usr/bin/env python3
"""Independent model of 1.20.1 aquifer site/cache/boundary planning.

Python results are NOT Rust compilation or Java-server differential results.
Run from any directory with `python3 tests/legacy1201_aquifer_boundary_qa.py`.
Uses standard library only, and never downloads Minecraft data.
"""
from __future__ import annotations
from pathlib import Path
from collections import defaultdict

ROOT = Path(__file__).resolve().parents[1]
RUST = ROOT / 'crates/horizon-world/src/legacy_1_20_1_aquifer_boundary.rs'
AQUIFER = ROOT / 'crates/horizon-world/src/legacy_1_20_1_aquifer.rs'
LIB = ROOT / 'crates/horizon-world/src/lib.rs'

# Independent Java 48-bit LCG / BlockPos coordinate seed (not a live JVM oracle).
MASK48 = (1 << 48) - 1

def signed(n: int, width: int) -> int:
    return (n + (1 << (width - 1))) % (1 << width) - (1 << (width - 1))

def make_site(seed: int, cell: tuple[int, int, int]) -> tuple[int, int, int]:
    x, y, z = cell
    mixed = signed(signed(x * 3129871, 32) ^ signed(z * 116129781, 64) ^ y, 64)
    pos_seed = signed(signed(signed(mixed * mixed, 64) * 42317861, 64) + signed(mixed * 11, 64), 64) >> 16
    state = ((seed ^ 0x5DEECE66D) & MASK48) ^ (pos_seed & MASK48)

    def draw(bound: int) -> int:
        nonlocal state
        while True:
            state = (state * 0x5DEECE66D + 11) & MASK48
            bits = state >> 17
            val = bits % bound
            if signed(bits - val + bound - 1, 32) >= 0:
                return val

    return x * 16 + draw(10), y * 12 + draw(9), z * 16 + draw(10)


def original_nearest(seed: int, pos: tuple[int, int, int]):
    x, y, z = pos
    ax, ay, az = (x - 5) // 16, (y + 1) // 12, (z - 5) // 16
    ordered = []
    for dx in (0, 1):
        for dy in (-1, 0, 1):
            for dz in (0, 1):
                cell = ax + dx, ay + dy, az + dz
                site = make_site(seed, cell)
                dist = sum((site[i] - pos[i]) ** 2 for i in range(3))
                ordered.append((dist, cell, site))
    # For ties, later enumeration wins, exactly the <= insertion in Rust.
    ranked = sorted(enumerate(ordered), key=lambda p: (p[1][0], -p[0]))
    return [pair for _, pair in ranked[:3]]

class Cache:
    def __init__(self, seed):
        self.seed = seed
        self.sites = {}
        self.statuses = {}
        self.samples = defaultdict(int)

    def site(self, cell):
        if cell not in self.sites:
            self.sites[cell] = make_site(self.seed, cell)
        return self.sites[cell]

    def nearest(self, pos):
        x, y, z = pos
        ax, ay, az = (x - 5) // 16, (y + 1) // 12, (z - 5) // 16
        best = []
        for dx in (0, 1):
            for dy in (-1, 0, 1):
                for dz in (0, 1):
                    cell = (ax + dx, ay + dy, az + dz)
                    site = self.site(cell)
                    dist = sum((site[i] - pos[i]) ** 2 for i in range(3))
                    best.append((dist, cell, site))
        return [v for _, v in sorted(enumerate(best), key=lambda p: (p[1][0], -p[0]))[:3]]

    def status_at(self, candidate):
        _, cell, site = candidate
        assert self.site(cell) == site
        if cell not in self.statuses:
            self.statuses[cell] = (site[1] // 3, site[0] % 9)
            self.samples[cell] += 1
        return self.statuses[cell]


def weight(distances):
    a, b, c = distances
    sim = lambda x, y: 1.0 - abs(y - x) / 25.0
    s12 = sim(a, b)
    if s12 <= 0:
        return 0.0, 0.0, 0.0
    s13, s23 = sim(a, c), sim(b, c)
    return s12, s12 * s13 if s13 > 0 else 0.0, s12 * s23 if s23 > 0 else 0.0


def run():
    source = RUST.read_text(encoding='utf-8')
    baseline = AQUIFER.read_text(encoding='utf-8')
    lib = LIB.read_text(encoding='utf-8')
    structural = {
        'module enabled once': lib.count('pub mod legacy_1_20_1_aquifer_boundary;') == 1,
        'parent enabled once': lib.count('pub mod legacy_1_20_1_aquifer;') == 1,
        'base exposes one insertion helper': baseline.count('pub(crate) fn insert_nearest(') == 1,
        'cached and uncached site primitives shared': 'legacy1201_aquifer_site(self.random, cell)' in source,
        'no process-global site cache': 'static mut' not in source and 'OnceLock' not in source,
        'failed status not cached': source.index('let computed = sampler(site.position)?;') < source.index('self.statuses.insert(key, computed);'),
        'site/cell validation': 'actual != site.position' in source,
        'pressure plan not disguised as full algorithm': 'not a substitute for the pressure/noise calculation' in source.lower(),
        '26.3 terrain not imported': 'worldgen::aquifer' not in source and 'TerrainAquifer' not in source,
        'Rust unit tests present': source.count('#[test]') == 11,
    }
    assert all(structural.values()), {k:v for k,v in structural.items() if not v}
    print(f'Static module/integration checks: {len(structural)}/{len(structural)}')

    vectors = [
        (0, (0,0,0), (0,7,9)),
        (7, (-1,-2,3), (-10,-23,54)),
        (-123, (12,5,-8), (193,62,-125)),
        (1_234_567, (3,-3,-3), (55,-30,-46)),
    ]
    for seed, cell, expected in vectors:
        assert make_site(seed, cell) == expected
    print(f'Independent Java LCG site vectors: {len(vectors)}/{len(vectors)}')

    queries = 0
    seed_values = [0, -1, 27, 1_234_567, -(1 << 63)]
    samples_repeated = 0
    for seed in seed_values:
        cache = Cache(seed)
        for x in range(-63, 65, 8):
            for y in (-64, -53, -1, 0, 63, 128, 319):
                for z in (-47, 49, 8):
                    pos = x,y,z
                    expected = original_nearest(seed, pos)
                    got = cache.nearest(pos)
                    assert got == expected, (seed,pos,got,expected)
                    assert got == cache.nearest(pos)
                    statuses = [cache.status_at(v) for v in got]
                    assert statuses == [cache.status_at(v) for v in got]
                    assert weight([v[0] for v in got]) == weight([v[0] for v in expected])
                    assert all(0 <= v[0] for v in got)
                    assert all(cache.samples[v[1]] == 1 for v in got)
                    samples_repeated += 3
                    queries += 1
    print(f'Model cached vs uncached nearest queries: {queries}/{queries}')
    print(f'Model repeated fluid statuses reused: {samples_repeated}/{samples_repeated}')

    weights = [
        ([10,35,59], (0,0,0)),
        ([10,20,32], (.6, .072, .312)),
        ([15,15,15], (1,1,1)),
        ([0,5,30], (.8,0,0)),
        ([0,24,24], (.04,.0016,.04)),
    ]
    for xs, ref in weights:
        got = weight(xs)
        assert all(abs(a-b)<1e-12 for a,b in zip(got,ref)), (xs,got,ref)
    print(f'Pressure-plan coefficient cases: {len(weights)}/{len(weights)}')

if __name__ == '__main__':
    run()
