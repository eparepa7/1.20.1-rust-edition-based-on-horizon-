"""Independent Java SE and Python reference tests for 1.5.20-1.5.21 1.20.1 aquifer router & chunk carving pipeline."""

from __future__ import annotations

from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

REPO = Path(__file__).resolve().parents[1]
ROUTER_RS = REPO / "crates" / "horizon-world" / "src" / "legacy_1_20_1_aquifer_router.rs"
CHUNK_CARVING_RS = REPO / "crates" / "horizon-world" / "src" / "legacy_1_20_1_chunk_carving.rs"
LIB_RS = REPO / "crates" / "horizon-world" / "src" / "lib.rs"

LCG_MULT = 0x5DEECE66D
LCG_ADD = 0xB
LCG_MASK = (1 << 48) - 1


def s64(value: int) -> int:
    value &= 0xFFFFFFFFFFFFFFFF
    return value - (1 << 64) if value & (1 << 63) else value


def s32(value: int) -> int:
    value &= 0xFFFFFFFF
    return value - (1 << 32) if value & (1 << 31) else value


class PyLegacyRandom:
    def __init__(self, seed: int) -> None:
        self.set_seed(seed)

    def set_seed(self, seed: int) -> None:
        self.state = ((seed & 0xFFFFFFFFFFFFFFFF) ^ LCG_MULT) & LCG_MASK

    def bits(self, count: int) -> int:
        self.state = (self.state * LCG_MULT + LCG_ADD) & LCG_MASK
        return s32(self.state >> (48 - count))

    def next_long(self) -> int:
        hi = self.bits(32) << 32
        lo = self.bits(32)
        return s64(hi + lo)


def py_set_large_feature_seed(rng: PyLegacyRandom, base_seed: int, chunk_x: int, chunk_z: int) -> tuple[int, int]:
    rng.set_seed(base_seed)
    a = rng.next_long()
    b = rng.next_long()
    mixed = s64(s64(chunk_x * a) ^ s64(chunk_z * b) ^ s64(base_seed))
    rng.set_seed(mixed)
    return (mixed, rng.next_long())


def f32_to_f64(value: float) -> float:
    return struct.unpack(">f", struct.pack(">f", value))[0]


DEEP_DARK_EROSION_MAX = f32_to_f64(-0.225)
DEEP_DARK_DEPTH_MIN = f32_to_f64(0.9)
PRELIMINARY_SURFACE_THRESHOLD = 0.390625


def py_quart_block_column(x: int, z: int) -> tuple[int, int]:
    return ((x >> 2) << 2, (z >> 2) << 2)


def py_is_deep_dark_region(erosion: float, depth: float) -> tuple[bool, bool]:
    """Returns (is_deep_dark, sampled_depth)."""
    if not (erosion < DEEP_DARK_EROSION_MAX):
        return (False, False)
    return (depth > DEEP_DARK_DEPTH_MIN, True)


def py_compute_preliminary_surface(
    x: int,
    z: int,
    min_y: int,
    height: int,
    cell_height: int,
    seed: int,
) -> tuple[int, int, int, int]:
    """Returns (qx, qz, surface_y, samples_taken)."""
    qx, qz = py_quart_block_column(x, z)
    y = min_y + height
    samples = 0
    while y >= min_y:
        samples += 1
        h = ((qx * 3129871) ^ (qz * 116129781) ^ (y * 42317861) ^ seed) & 0xFFFFFFFF
        base = ((h & 0xFFFF) / 65535.0) * 0.8 - 0.2
        slope = (64.0 - float(y)) / 160.0
        density = base + slope
        if density > PRELIMINARY_SURFACE_THRESHOLD:
            return (qx, qz, y, samples)
        y -= cell_height
    return (qx, qz, 2147483647, samples)


JAVA_ROUTER_MODEL = r"""
public final class AquiferRouterRef {
    static final double DEEP_DARK_EROSION_MAX = (double) -0.225f;
    static final double DEEP_DARK_DEPTH_MIN = (double) 0.9f;
    static final double PRELIMINARY_THRESHOLD = 0.390625d;
    static final long MULT = 0x5DEECE66DL;
    static final long ADD = 0xBL;
    static final long MASK = (1L << 48) - 1L;

    static final class LegacyRng {
        long state;
        void setSeed(long s) {
            state = (s ^ MULT) & MASK;
        }
        int next(int bits) {
            state = (state * MULT + ADD) & MASK;
            return (int)(state >>> (48 - bits));
        }
        long nextLong() {
            return ((long)next(32) << 32) + next(32);
        }
    }

    static String setLargeFeatureSeed(long baseSeed, int cx, int cz) {
        LegacyRng rng = new LegacyRng();
        rng.setSeed(baseSeed);
        long a = rng.nextLong();
        long b = rng.nextLong();
        long mixed = ((long)cx * a) ^ ((long)cz * b) ^ baseSeed;
        rng.setSeed(mixed);
        return mixed + "," + rng.nextLong();
    }

    static int quartBlock(int c) {
        return (c >> 2) << 2;
    }

    static String checkDeepDark(double erosion, double depth) {
        boolean sampledDepth = false;
        boolean result = false;
        if (erosion < DEEP_DARK_EROSION_MAX) {
            sampledDepth = true;
            if (depth > DEEP_DARK_DEPTH_MIN) {
                result = true;
            }
        }
        return (result ? 1 : 0) + "," + (sampledDepth ? 1 : 0);
    }

    static String computeSurface(int x, int z, int minY, int height, int cellHeight, int seed) {
        int qx = quartBlock(x);
        int qz = quartBlock(z);
        int y = minY + height;
        int samples = 0;
        while (y >= minY) {
            samples++;
            long h = ((long)(qx * 3129871) ^ (long)(qz * 116129781) ^ (long)(y * 42317861) ^ (long)seed) & 0xFFFFFFFFL;
            double base = ((double)(h & 0xFFFFL) / 65535.0d) * 0.8d - 0.2d;
            double slope = (64.0d - (double)y) / 160.0d;
            double density = base + slope;
            if (density > PRELIMINARY_THRESHOLD) {
                return qx + "," + qz + "," + y + "," + samples;
            }
            y -= cellHeight;
        }
        return qx + "," + qz + "," + Integer.MAX_VALUE + "," + samples;
    }

    public static void main(String[] args) {
        double[] erosions = {
            -1.0, -0.5, DEEP_DARK_EROSION_MAX - 1e-12, DEEP_DARK_EROSION_MAX,
            DEEP_DARK_EROSION_MAX + 1e-12, -0.225d, 0.0, 0.5
        };
        double[] depths = {
            -0.5, 0.0, 0.8999999, DEEP_DARK_DEPTH_MIN - 1e-12,
            DEEP_DARK_DEPTH_MIN, DEEP_DARK_DEPTH_MIN + 1e-12, 0.9d, 1.25
        };
        for (double e : erosions) {
            for (double d : depths) {
                System.out.println("DD:" + Double.doubleToRawLongBits(e) + "," + Double.doubleToRawLongBits(d) + ":" + checkDeepDark(e, d));
            }
        }
        int[][] configs = {
            {-64, 384, 8},
            {-64, 384, 4},
            {0, 256, 8},
            {-32, 128, 16}
        };
        for (int c = 0; c < configs.length; c++) {
            int minY = configs[c][0];
            int height = configs[c][1];
            int cellHeight = configs[c][2];
            for (int x = -35; x <= 35; x += 3) {
                for (int z = -35; z <= 35; z += 5) {
                    System.out.println("PS:" + x + "," + z + "," + c + ":" + computeSurface(x, z, minY, height, cellHeight, c * 97 + 13));
                }
            }
        }
        long[] seeds = {0L, 1L, -1L, 123456789L, -987654321012345L};
        for (long seed : seeds) {
            for (int dx = -8; dx <= 8; dx += 4) {
                for (int dz = -8; dz <= 8; dz += 4) {
                    System.out.println("LF:" + seed + "," + dx + "," + dz + ":" + setLargeFeatureSeed(seed, dx, dz));
                }
            }
        }
    }
}
"""


class AquiferRouterReferenceTest(unittest.TestCase):
    def test_structural_and_wiring_invariants(self) -> None:
        self.assertTrue(ROUTER_RS.is_file(), "missing legacy_1_20_1_aquifer_router.rs")
        self.assertTrue(CHUNK_CARVING_RS.is_file(), "missing legacy_1_20_1_chunk_carving.rs")
        lib_text = LIB_RS.read_text(encoding="utf-8")
        self.assertIn("pub mod legacy_1_20_1_aquifer_router;", lib_text)
        self.assertIn("pub mod legacy_1_20_1_chunk_carving;", lib_text)
        source = ROUTER_RS.read_text(encoding="utf-8")
        for token in (
            "pub const LEGACY1201_DEEP_DARK_EROSION_MAX: f64 = (-0.225_f32) as f64;",
            "pub const LEGACY1201_DEEP_DARK_DEPTH_MIN: f64 = (0.9_f32) as f64;",
            "pub const LEGACY1201_PRELIMINARY_SURFACE_DENSITY_THRESHOLD: f64 = 0.390625;",
            "pub const fn legacy1201_quart_block_column",
            "pub fn legacy1201_is_deep_dark_region",
            "pub fn legacy1201_compute_preliminary_surface_level",
            "pub struct Legacy1201QuartSurfaceCache",
            "pub enum Legacy1201ChunkAquifer",
            "pub fn output_with_legacy1201_chunk_aquifer",
        ):
            self.assertIn(token, source)
        carving_source = CHUNK_CARVING_RS.read_text(encoding="utf-8")
        for token in (
            "pub const LEGACY1201_CARVER_CHUNK_RANGE: i32 = 8;",
            "pub fn legacy1201_set_large_feature_seed",
            "pub struct Legacy1201CarveStepSummary",
            "pub fn legacy1201_apply_chunk_carvers",
        ):
            self.assertIn(token, carving_source)
        for mod_src in (source, carving_source):
            for forbidden in ("TerrainAquifer", "NoiseBasedAquifer", "AquiferSamplers", "apply_carving_mask"):
                fn_body = mod_src.split("#[cfg(test)]", 1)[0]
                self.assertNotIn(
                    forbidden,
                    "\n".join(
                        line
                        for line in fn_body.splitlines()
                        if not line.lstrip().startswith("//")
                    ),
                )

    def test_java_se_python_router_and_carving_seed_equivalence(self) -> None:
        javac = shutil.which("javac")
        java = shutil.which("java")
        if not javac or not java:
            self.skipTest("JDK missing; cannot run Java SE router reference comparison")

        with tempfile.TemporaryDirectory() as tmp:
            java_file = Path(tmp) / "AquiferRouterRef.java"
            java_file.write_text(JAVA_ROUTER_MODEL, encoding="utf-8")
            subprocess.run([javac, str(java_file)], check=True, cwd=tmp)
            proc = subprocess.run(
                [java, "-cp", tmp, "AquiferRouterRef"],
                check=True,
                capture_output=True,
                text=True,
            )

        configs = [
            (-64, 384, 8),
            (-64, 384, 4),
            (0, 256, 8),
            {-32: 128} and (-32, 128, 16),
        ]
        rng = PyLegacyRandom(0)
        compared = 0
        for line in proc.stdout.strip().splitlines():
            kind, header, payload = line.split(":")
            if kind == "DD":
                ebits_s, dbits_s = header.split(",")
                e = struct.unpack(">d", struct.pack(">Q", int(ebits_s) & 0xFFFFFFFFFFFFFFFF))[0]
                d = struct.unpack(">d", struct.pack(">Q", int(dbits_s) & 0xFFFFFFFFFFFFFFFF))[0]
                is_dd, sampled = py_is_deep_dark_region(e, d)
                expected = f"{1 if is_dd else 0},{1 if sampled else 0}"
                self.assertEqual(payload, expected)
                compared += 1
            elif kind == "PS":
                x_s, z_s, c_s = header.split(",")
                x, z, c = int(x_s), int(z_s), int(c_s)
                min_y, height, cell_height = configs[c]
                qx, qz, surf_y, samples = py_compute_preliminary_surface(
                    x, z, min_y, height, cell_height, c * 97 + 13
                )
                expected = f"{qx},{qz},{surf_y},{samples}"
                self.assertEqual(payload, expected)
                compared += 1
            elif kind == "LF":
                seed_s, dx_s, dz_s = header.split(",")
                mixed, next_l = py_set_large_feature_seed(rng, int(seed_s), int(dx_s), int(dz_s))
                expected = f"{mixed},{next_l}"
                self.assertEqual(payload, expected)
                compared += 1
        self.assertEqual(compared, 1629)
        print(f"Java SE/Python aquifer router, preliminary surface, and carver seeds: {compared}/{compared}")


if __name__ == "__main__":
    unittest.main()
