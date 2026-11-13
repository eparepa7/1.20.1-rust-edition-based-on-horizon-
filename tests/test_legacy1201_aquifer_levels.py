"""Independent Java SE/Python arithmetic checks for 1.20.1 aquifer levels.

This is NOT a test of compiled Rust or the Minecraft server. Run:
python3 -m unittest discover -s tests -p test_legacy1201_aquifer_levels.py
"""
import math
import random
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

JAVA = r"""
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.util.StringTokenizer;
public class AquiferLevelReference {
    static int floodedness(int y, int lowest, boolean underGlobal, boolean deepDark, double noise) {
        if (deepDark) return 0;
        double distance = (double)((long)lowest + 8L - (long)y);
        double influence = underGlobal ? 1.0 - Math.max(0.0, Math.min(1.0, distance / 64.0)) : 0.0;
        double t = 1.0 - influence;
        double full = -0.3 + t * (0.8 - (-0.3));
        double partial = -0.8 + t * (0.4 - (-0.8));
        double value = Math.max(-1.0, Math.min(1.0, noise));
        return value > full ? 2 : (value > partial ? 1 : 0);
    }
    static int randomized(int y, int lowest, double spread) {
        int cy = Math.floorDiv(y, 40);
        int quantized = (int)Math.floor((spread * 10.0) / 3.0) * 3;
        return Math.min(lowest, cy * 40 + 20 + quantized);
    }
    static int lava(int x, int y, int z, int fluidLevel, int globalFluid, double noise) {
        if (fluidLevel > -10 || fluidLevel == -32512 || globalFluid == 2) return globalFluid;
        return Math.abs(noise) > 0.3 ? 2 : globalFluid;
    }
    public static void main(String[] args) throws Exception {
        BufferedReader reader = new BufferedReader(new InputStreamReader(System.in));
        String line;
        StringBuilder result = new StringBuilder();
        while ((line = reader.readLine()) != null) {
            StringTokenizer t = new StringTokenizer(line);
            int x = Integer.parseInt(t.nextToken());
            int y = Integer.parseInt(t.nextToken());
            int z = Integer.parseInt(t.nextToken());
            int low = Integer.parseInt(t.nextToken());
            boolean underGlobal = t.nextToken().equals("1");
            boolean deepDark = t.nextToken().equals("1");
            double floodednessNoise = Double.parseDouble(t.nextToken());
            double spreadNoise = Double.parseDouble(t.nextToken());
            int selectedLevel = Integer.parseInt(t.nextToken());
            int globalFluid = Integer.parseInt(t.nextToken());
            double lavaNoise = Double.parseDouble(t.nextToken());
            result.append(floodedness(y, low, underGlobal, deepDark, floodednessNoise))
                .append(' ').append(randomized(y, low, spreadNoise))
                .append(' ').append(lava(x, y, z, selectedLevel, globalFluid, lavaNoise))
                .append('\n');
        }
        System.out.print(result.toString());
    }
}
"""


def model(case):
    x, y, z, low, under, dark, flood, spread, level, global_fluid, lava_noise = case
    if dark:
        flooded = 0
    else:
        distance = float(low + 8 - y)
        influence = 1.0 - max(0.0, min(1.0, distance / 64.0)) if under else 0.0
        t = 1.0 - influence
        full = -0.3 + t * (0.8 - (-0.3))
        partial = -0.8 + t * (0.4 - (-0.8))
        flood = max(-1.0, min(1.0, flood))
        flooded = 2 if flood > full else (1 if flood > partial else 0)
    fluid_surface = min(low, (y // 40) * 40 + 20 + math.floor(spread * 10.0 / 3.0) * 3)
    lava_type = (global_fluid if level > -10 or level == -32512 or global_fluid == 2
                 else (2 if abs(lava_noise) > 0.3 else global_fluid))
    return flooded, fluid_surface, lava_type


class AquiferLevelArithmeticTest(unittest.TestCase):
    @staticmethod
    def cases():
        rng = random.Random(120116)
        cases = []
        for x in [-65, -64, -17, -16, -1, 0, 15, 16, 17, 63, 64]:
            for y in [-65, -64, -41, -40, -1, 0, 39, 40, 63, 319]:
                cases.append((x, y, -x - 1, 56, 1, 0, 0.8, -0.01, -10, 1, -0.301))
        for _ in range(3000):
            x, y, z = (rng.randint(-100000, 100000), rng.randint(-64, 319), rng.randint(-100000, 100000))
            cases.append((x, y, z, rng.randint(-64, 319), rng.randint(0, 1), rng.randint(0, 1),
                          rng.choice([-1., -0.8, -0.799, -0.3, -0.299, 0., 0.3, 0.4, 0.8, 0.801, 1.]),
                          rng.uniform(-1, 1), rng.choice([-32512, -54, -11, -10, -9, 63]),
                          rng.choice([1, 2]), rng.choice([-1., -0.301, -0.3, 0., 0.3, 0.301, 1.])))
        return cases

    def test_fixed_boundaries(self):
        self.assertEqual(model((0, -1, 0, 30, 0, 0, -1, -0.01, -10, 1, -0.301)), (0, -23, 2))
        self.assertEqual(model((0, 0, 0, 56, 1, 0, -0.8, 0, -32512, 1, 1.0)), (0, 20, 1))
        self.assertEqual(model((0, 64, 0, 56, 1, 0, -0.3, 0, -9, 1, 1.0))[0], 1)

    def test_python_vs_java_se_model(self):
        if not shutil.which('javac') or not shutil.which('java'):
            self.skipTest('JDK missing: cannot claim Java arithmetic parity')
        cases = self.cases()
        with tempfile.TemporaryDirectory(prefix='horizon-1201-aquifer-level-') as tmp:
            source = Path(tmp) / 'AquiferLevelReference.java'
            source.write_text(JAVA, encoding='utf-8')
            subprocess.run(['javac', str(source)], capture_output=True, text=True, check=True)
            payload = ''.join(' '.join(map(str, case)) + '\n' for case in cases)
            reference = subprocess.run(['java', '-cp', tmp, 'AquiferLevelReference'],
                                       input=payload, capture_output=True, text=True, check=True)
            lines = reference.stdout.splitlines()
            self.assertEqual(len(lines), len(cases))
            for index, (case, line) in enumerate(zip(cases, lines)):
                self.assertEqual(tuple(map(int, line.split())), model(case), f'case {index}: {case}')
            print(f'Java SE/Python level arithmetic: {len(cases)}/{len(cases)}')


if __name__ == '__main__':
    unittest.main()
