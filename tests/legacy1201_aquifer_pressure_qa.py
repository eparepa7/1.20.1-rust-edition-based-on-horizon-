"""Numerical (not vanilla-server) oracle for 1.20.1 aquifer pressure math.

Run: python3 tests/legacy1201_aquifer_pressure_qa.py
JDK required for bitwise cross-language vectors. This file never fetches,
unpacks or redistributes Mojang files. Rust tests still require cargo.
"""
from __future__ import annotations

import random
import shutil
import struct
import subprocess
import tempfile
from pathlib import Path

# Independent Java SE numeric model; not decompiled Minecraft code.
JAVA = r"""
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.util.StringTokenizer;
class PressureArithmeticReference {
  static double pressure(int y, int levelA, int levelB, int fluidA, int fluidB, double noise) {
    if (((fluidA == 1 && fluidB == 2) || (fluidA == 2 && fluidB == 1))
        && y < levelA && y < levelB) return 2.0;
    long delta = Math.abs((long)levelA - (long)levelB);
    if (delta == 0) return 0.0;
    double midpoint = ((double)levelA + (double)levelB) * 0.5;
    double dy = ((double)y + 0.5) - midpoint;
    double gap = (double)delta * 0.5 - Math.abs(dy);
    double slope;
    if (dy > 0.0) slope = gap > 0.0 ? gap / 1.5 : gap / 2.5;
    else {
      double offset = gap + 3.0;
      slope = offset > 0.0 ? offset / 3.0 : offset / 10.0;
    }
    double barrier = slope >= -2.0 && slope <= 2.0 ? noise : 0.0;
    return 2.0 * (slope + barrier);
  }
  public static void main(String[] args) throws Exception {
    BufferedReader reader = new BufferedReader(new InputStreamReader(System.in));
    String line;
    StringBuilder output = new StringBuilder();
    while ((line = reader.readLine()) != null) {
      StringTokenizer parts = new StringTokenizer(line);
      int y = Integer.parseInt(parts.nextToken());
      int a = Integer.parseInt(parts.nextToken());
      int b = Integer.parseInt(parts.nextToken());
      int fa = Integer.parseInt(parts.nextToken());
      int fb = Integer.parseInt(parts.nextToken());
      double noise = Double.parseDouble(parts.nextToken());
      output.append(Long.toUnsignedString(Double.doubleToRawLongBits(
          pressure(y, a, b, fa, fb, noise)))).append('\n');
    }
    System.out.print(output);
  }
}
"""


def model(y: int, a: int, b: int, fa: int, fb: int, noise: float) -> float:
    if {fa, fb} == {1, 2} and y < min(a, b):
        return 2.0
    difference = abs(a - b)
    if difference == 0:
        return 0.0
    centre = 0.5 * (a + b)
    relative_y = (y + 0.5) - centre
    remaining = difference * 0.5 - abs(relative_y)
    if relative_y > 0:
        gradient = remaining / (1.5 if remaining > 0 else 2.5)
    else:
        below = remaining + 3.0
        gradient = below / (3.0 if below > 0 else 10.0)
    return 2.0 * (gradient + (noise if -2 <= gradient <= 2 else 0.0))


def bits(number: float) -> int:
    return int.from_bytes(struct.pack(">d", number), "big")


def vectors() -> list[tuple[int, int, int, int, int, float]]:
    fixed = [
        (5, 20, 20, 1, 2, 0.125),
        (5, 20, 20, 1, 1, 0.125),
        (20, 20, 20, 1, 2, 0.125),
        (-54, -54, 63, 1, 2, -0.25),
        (-55, -54, 63, 1, 2, -0.25),
        (30, 4, 14, 1, 1, 0.25),
        (-200, 4, 14, 1, 1, 0.25),
        (14, 12, 20, 1, 1, 0.25),
        (14, 20, 12, 1, 1, 0.25),
        (319, -64, 320, 1, 2, -0.125),
    ]
    rng = random.Random(1201)
    for _ in range(3_000):
        a, b = rng.randint(-65, 321), rng.randint(-65, 321)
        fixed.append((rng.randint(-65, 321), a, b, rng.choice([0, 1, 2]),
                      rng.choice([0, 1, 2]), rng.choice([0.0, 0.1, -0.1, 0.25, -0.375, 0.9])))
    return fixed


def main() -> None:
    cases = vectors()
    if not shutil.which("javac") or not shutil.which("java"):
        print("Java SE numeric oracle: SKIPPED (JDK unavailable)")
        print(f"Python mathematical reference cases: {len(cases)} (no Rust binary executed)")
        return

    with tempfile.TemporaryDirectory(prefix="horizon-1201-pressure-") as tmp:
        path = Path(tmp) / "PressureArithmeticReference.java"
        path.write_text(JAVA, encoding="utf-8")
        subprocess.run(["javac", str(path)], check=True, capture_output=True, text=True)
        payload = "".join(" ".join(map(str, case)) + "\n" for case in cases)
        run = subprocess.run(["java", "-cp", tmp, "PressureArithmeticReference"],
                             input=payload, check=True, capture_output=True, text=True)
        got = [int(x) for x in run.stdout.splitlines()]
        assert len(got) == len(cases), (len(got), len(cases))
        for i, (case, java_bits) in enumerate(zip(cases, got)):
            expected_bits = bits(model(*case))
            assert java_bits == expected_bits, (i, case, hex(expected_bits), hex(java_bits))
    print(f"Java SE vs Python pressure arithmetic (bitwise): {len(cases)}/{len(cases)}")
    print("IMPORTANT: not a Rust execution and not Minecraft Java 1.20.1 server parity")


if __name__ == "__main__":
    main()
