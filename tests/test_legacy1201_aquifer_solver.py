"""1.20.1 NoiseBasedAquifer block-substance and fluid-schedule reference tests.

Checks the complete 1.20.1 three-centre computeSubstance + shouldScheduleFluidUpdate
branching and lazy-sampling order between Python and an independent Java SE
reference model, plus structural checks on `legacy_1_20_1_aquifer_solver.rs`.
Uses no Mojang JARs, mappings or bundled game data.
"""

from __future__ import annotations

import math
import random
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
SOLVER_RS = ROOT / "crates/horizon-world/src/legacy_1_20_1_aquifer_solver.rs"
LIB_RS = ROOT / "crates/horizon-world/src/lib.rs"

FLOWING_UPDATE_SIMILARITY = 1.0 - (144.0 - 100.0) / 25.0

JAVA = r"""
import java.io.*;
import java.util.*;

public class AquiferSolverReference {
    static final double FLOWING_UPDATE_SIMILARITY = 1.0 - (144.0 - 100.0) / 25.0;

    static double similarity(int a, int b) {
        return 1.0 - Math.abs(b - a) / 25.0;
    }

    static int globalFluidAt(int seaLevel, int defaultFluid, int y) {
        if (y < Math.min(-54, seaLevel)) {
            return y < -54 ? 2 : 0;
        }
        return y < seaLevel ? defaultFluid : 0;
    }

    static int statusAt(int level, int state, int y) {
        return y < level ? state : 0;
    }

    static double pairPressure(
        int y,
        int levelA, int stateA,
        int levelB, int stateB,
        double barrierNoise,
        int[] barrierCalls
    ) {
        int a = statusAt(levelA, stateA, y);
        int b = statusAt(levelB, stateB, y);
        if ((a == 1 && b == 2) || (a == 2 && b == 1)) {
            return 2.0;
        }
        int diff = Math.abs(levelA - levelB);
        if (diff == 0) {
            return 0.0;
        }
        double centre = 0.5 * (double)(levelA + levelB);
        double relY = (double)y + 0.5 - centre;
        double rem = (double)diff / 2.0 - Math.abs(relY);
        double grad;
        if (relY > 0.0) {
            double n = 0.0 + rem;
            grad = n > 0.0 ? n / 1.5 : n / 2.5;
        } else {
            double n = 3.0 + rem;
            grad = n > 0.0 ? n / 3.0 : n / 10.0;
        }
        double noise;
        if (grad < -2.0 || grad > 2.0) {
            noise = 0.0;
        } else {
            if (barrierCalls[0] == 0) {
                barrierCalls[0] = 1;
            }
            noise = barrierNoise;
        }
        return 2.0 * (noise + grad);
    }

    static int[] solve(
        int y, double density, int seaLevel, int defaultFluid,
        int d0, int d1, int d2,
        int l0, int s0, int l1, int s1, int l2, int s2,
        double barrierNoise
    ) {
        int statusCalls = 0;
        int[] barrierCalls = new int[]{0};
        if (density > 0.0) {
            return new int[]{-1, 0, statusCalls, barrierCalls[0]};
        }
        if (globalFluidAt(seaLevel, defaultFluid, y) == 2) {
            return new int[]{2, 0, statusCalls, barrierCalls[0]};
        }
        statusCalls = 1;
        double sim12 = similarity(d0, d1);
        int candidate = statusAt(l0, s0, y);
        if (sim12 <= 0.0) {
            int sched = sim12 >= FLOWING_UPDATE_SIMILARITY ? 1 : 0;
            return new int[]{candidate, sched, statusCalls, barrierCalls[0]};
        }
        if (candidate == 1 && globalFluidAt(seaLevel, defaultFluid, y - 1) == 2) {
            return new int[]{candidate, 1, statusCalls, barrierCalls[0]};
        }
        statusCalls = 2;
        double p12 = sim12 * pairPressure(y, l0, s0, l1, s1, barrierNoise, barrierCalls);
        if (density + p12 > 0.0) {
            return new int[]{-1, 0, statusCalls, barrierCalls[0]};
        }
        statusCalls = 3;
        double sim13 = similarity(d0, d2);
        if (sim13 > 0.0) {
            double p13 = sim12 * sim13 * pairPressure(y, l0, s0, l2, s2, barrierNoise, barrierCalls);
            if (density + p13 > 0.0) {
                return new int[]{-1, 0, statusCalls, barrierCalls[0]};
            }
        }
        double sim23 = similarity(d1, d2);
        if (sim23 > 0.0) {
            double p23 = sim12 * sim23 * pairPressure(y, l1, s1, l2, s2, barrierNoise, barrierCalls);
            if (density + p23 > 0.0) {
                return new int[]{-1, 0, statusCalls, barrierCalls[0]};
            }
        }
        return new int[]{candidate, 1, statusCalls, barrierCalls[0]};
    }

    public static void main(String[] args) throws Exception {
        BufferedReader in = new BufferedReader(new InputStreamReader(System.in));
        StringBuilder out = new StringBuilder();
        String line;
        while ((line = in.readLine()) != null) {
            StringTokenizer t = new StringTokenizer(line);
            int y = Integer.parseInt(t.nextToken());
            double density = Double.parseDouble(t.nextToken());
            int seaLevel = Integer.parseInt(t.nextToken());
            int defaultFluid = Integer.parseInt(t.nextToken());
            int d0 = Integer.parseInt(t.nextToken());
            int d1 = Integer.parseInt(t.nextToken());
            int d2 = Integer.parseInt(t.nextToken());
            int l0 = Integer.parseInt(t.nextToken());
            int s0 = Integer.parseInt(t.nextToken());
            int l1 = Integer.parseInt(t.nextToken());
            int s1 = Integer.parseInt(t.nextToken());
            int l2 = Integer.parseInt(t.nextToken());
            int s2 = Integer.parseInt(t.nextToken());
            double barrier = Double.parseDouble(t.nextToken());
            int[] res = solve(y, density, seaLevel, defaultFluid, d0, d1, d2, l0, s0, l1, s1, l2, s2, barrier);
            out.append(res[0]).append(' ')
               .append(res[1]).append(' ')
               .append(res[2]).append(' ')
               .append(res[3]).append('\n');
        }
        System.out.print(out);
    }
}
"""


def global_fluid_at(sea_level: int, default_fluid: int, y: int) -> int:
    if y < min(-54, sea_level):
        return 2 if y < -54 else 0
    return default_fluid if y < sea_level else 0


def status_at(level: int, state: int, y: int) -> int:
    return state if y < level else 0


def pair_pressure(
    y: int,
    level_a: int,
    state_a: int,
    level_b: int,
    state_b: int,
    barrier_noise: float,
    barrier_calls: list[int],
) -> float:
    a = status_at(level_a, state_a, y)
    b = status_at(level_b, state_b, y)
    if (a == 1 and b == 2) or (a == 2 and b == 1):
        return 2.0
    diff = abs(level_a - level_b)
    if diff == 0:
        return 0.0
    centre = 0.5 * float(level_a + level_b)
    rel_y = float(y) + 0.5 - centre
    rem = float(diff) / 2.0 - abs(rel_y)
    if rel_y > 0.0:
        grad = rem / 1.5 if rem > 0.0 else rem / 2.5
    else:
        below = 3.0 + rem
        grad = below / 3.0 if below > 0.0 else below / 10.0
    if -2.0 <= grad <= 2.0:
        if barrier_calls[0] == 0:
            barrier_calls[0] = 1
        noise = barrier_noise
    else:
        noise = 0.0
    return 2.0 * (noise + grad)


def model(case: tuple) -> tuple[int, int, int, int]:
    y, density, sea_level, default_fluid, d0, d1, d2, l0, s0, l1, s1, l2, s2, barrier = case
    if density > 0.0:
        return (-1, 0, 0, 0)
    if global_fluid_at(sea_level, default_fluid, y) == 2:
        return (2, 0, 0, 0)
    sim12 = 1.0 - abs(d1 - d0) / 25.0
    candidate = status_at(l0, s0, y)
    if sim12 <= 0.0:
        sched = 1 if sim12 >= FLOWING_UPDATE_SIMILARITY else 0
        return (candidate, sched, 1, 0)
    if candidate == 1 and global_fluid_at(sea_level, default_fluid, y - 1) == 2:
        return (candidate, 1, 1, 0)

    barrier_calls = [0]
    p12 = sim12 * pair_pressure(y, l0, s0, l1, s1, barrier, barrier_calls)
    if density + p12 > 0.0:
        return (-1, 0, 2, barrier_calls[0])

    sim13 = 1.0 - abs(d2 - d0) / 25.0
    if sim13 > 0.0:
        p13 = sim12 * sim13 * pair_pressure(y, l0, s0, l2, s2, barrier, barrier_calls)
        if density + p13 > 0.0:
            return (-1, 0, 3, barrier_calls[0])

    sim23 = 1.0 - abs(d2 - d1) / 25.0
    if sim23 > 0.0:
        p23 = sim12 * sim23 * pair_pressure(y, l1, s1, l2, s2, barrier, barrier_calls)
        if density + p23 > 0.0:
            return (-1, 0, 3, barrier_calls[0])

    return (candidate, 1, 3, barrier_calls[0])


class AquiferSolverReferenceTest(unittest.TestCase):
    @staticmethod
    def cases() -> list[tuple]:
        fixed = [
            # Positive density: solid, no site/barrier calls
            (10, 0.05, 63, 1, 10, 15, 20, 63, 1, 63, 1, 63, 1, 0.1),
            # Global lava at y = -55: lava, no site/barrier calls
            (-55, -0.2, 63, 1, 10, 15, 20, 63, 1, 63, 1, 63, 1, 0.1),
            # sim12 = -0.76 (d1 - d0 = 44): schedules fluid update, only 1 status call
            (20, -0.2, 63, 1, 100, 144, 180, 63, 1, 10, 1, 10, 1, 0.1),
            # sim12 = -0.80 (d1 - d0 = 45): does not schedule fluid update, only 1 status call
            (20, -0.2, 63, 1, 100, 145, 180, 63, 1, 10, 1, 10, 1, 0.1),
            # Water at y = -54 directly above global lava at y = -55 with sim12 > 0
            (-54, -2.0, 63, 1, 10, 20, 30, 0, 1, -20, 2, -20, 2, 0.25),
            # First pair makes solid barrier, skips 3rd status
            (10, -0.2, 63, 1, 10, 15, 20, 40, 1, 40, 2, 40, 1, 0.0),
            # Open after all three pairs: schedules fluid update = 1, 3 statuses
            (10, -2.5, 63, 1, 10, 15, 20, 40, 1, 40, 1, 40, 1, 0.0),
        ]
        rng = random.Random(120119)
        for _ in range(3600):
            d0 = rng.randint(0, 120)
            d1 = d0 + rng.choice([0, 5, 12, 24, 25, 30, 44, 45, 60])
            d2 = d1 + rng.choice([0, 4, 10, 24, 25, 40])
            y = rng.choice([-64, -55, -54, -53, -20, 0, 14, 62, 63, 120])
            density = rng.choice([0.2, 0.0, -0.05, -0.25, -0.8, -1.5, -3.0])
            sea = rng.choice([63, -60])
            default_fluid = rng.choice([1, 2])
            l0 = rng.choice([-32512, -54, -20, 0, 14, 20, 63])
            l1 = rng.choice([-32512, -54, -20, 0, 14, 20, 63])
            l2 = rng.choice([-32512, -54, -20, 0, 14, 20, 63])
            s0, s1, s2 = rng.choice([1, 2]), rng.choice([1, 2]), rng.choice([1, 2])
            barrier = rng.choice([-0.5, -0.25, 0.0, 0.125, 0.375, 0.75])
            fixed.append((y, density, sea, default_fluid, d0, d1, d2, l0, s0, l1, s1, l2, s2, barrier))
        return fixed

    def test_structural_and_optimization_invariants(self) -> None:
        source = SOLVER_RS.read_text(encoding="utf-8")
        lib = LIB_RS.read_text(encoding="utf-8")
        checks = {
            "registered in lib.rs once": lib.count("pub mod legacy_1_20_1_aquifer_solver;") == 1,
            "defines flowing update similarity": "LEGACY1201_FLOWING_UPDATE_SIMILARITY" in source,
            "defines stateless cache resolver": "pub fn legacy1201_resolve_aquifer_substance" in source,
            "defines 26.3-optimized chunk solver": "pub struct Legacy1201NoiseAquifer" in source,
            "preserves 12-cell sliding window": "struct Cells" in source and "for to in [11, 10, 9, 8, 5, 4, 3, 2]" in source,
            "uses preallocated flat grid": "try_reserve_exact(count)" in source,
            "does not import 26.3 aquifer": "worldgen::aquifer" not in source and "NoiseBasedAquifer" not in source.split("mod tests")[0].replace("`NoiseBasedAquifer`", "").replace("NoiseBasedAquifer.", ""),
            "unit tests present": source.count("#[test]") >= 8,
        }
        self.assertTrue(all(checks.values()), {k: v for k, v in checks.items() if not v})

    def test_fixed_lazy_branching_cases(self) -> None:
        cases = self.cases()[:7]
        expected = [
            (-1, 0, 0, 0),
            (2, 0, 0, 0),
            (1, 1, 1, 0),
            (1, 0, 1, 0),
            (1, 1, 1, 0),
            (-1, 0, 2, 0),
            (1, 1, 3, 0),
        ]
        for case, want in zip(cases, expected):
            self.assertEqual(model(case), want, f"case={case}")

    def test_java_se_python_solver_equivalence(self) -> None:
        if not (shutil.which("java") and shutil.which("javac")):
            self.skipTest("JDK missing; cannot claim Java SE solver comparison")
        cases = self.cases()
        with tempfile.TemporaryDirectory(prefix="horizon-1201-aquifer-solver-") as tmp:
            source = Path(tmp) / "AquiferSolverReference.java"
            source.write_text(JAVA, encoding="utf-8")
            subprocess.run(["javac", str(source)], check=True, capture_output=True, text=True)
            payload = "".join(" ".join(map(str, case)) + "\n" for case in cases)
            proc = subprocess.run(
                ["java", "-cp", tmp, "AquiferSolverReference"],
                input=payload,
                check=True,
                capture_output=True,
                text=True,
            )
            lines = proc.stdout.splitlines()
            self.assertEqual(len(lines), len(cases))
            for idx, (case, line) in enumerate(zip(cases, lines)):
                got = tuple(map(int, line.split()))
                self.assertEqual(got, model(case), f"idx={idx}: {case}")
            print(f"Java SE/Python 1.20.1 aquifer solver cases: {len(cases)}/{len(cases)}")


if __name__ == "__main__":
    unittest.main()
