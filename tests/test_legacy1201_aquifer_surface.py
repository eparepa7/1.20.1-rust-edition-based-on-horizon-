"""1.20.1 surface-survey and cell-status arithmetic checks (Java SE vs Python).

This tests independent models, NOT the compiled Rust implementation or the
vanilla Minecraft server. It uses no Minecraft-owned JAR, mappings or data.
"""

import math
import random
import shutil
import subprocess
import tempfile
import unittest
from pathlib import Path

OFFSETS = (
    (0, 0), (-2, -1), (-1, -1), (0, -1), (1, -1),
    (-3, 0), (-2, 0), (-1, 0), (1, 0),
    (-2, 1), (-1, 1), (0, 1), (1, 1),
)

JAVA = r"""
import java.io.*;
import java.util.*;
public class AquiferSurfaceReference {
    static final int[][] OFFSETS = {
      {0,0}, {-2,-1}, {-1,-1}, {0,-1}, {1,-1},
      {-3,0}, {-2,0}, {-1,0}, {1,0},
      {-2,1}, {-1,1}, {0,1}, {1,1}
    };
    static int[] global(int y) { return y < -54 ? new int[]{-54, 2} : new int[]{63, 1}; }
    static int prelim(int mode, int x, int z, int index) {
        switch (mode) {
            case 0: return 100;
            case 1: return -40;
            case 2: return 30;
            case 3: return index == 0 ? 30 : -20;
            case 4: return 35 + Math.floorMod(Math.floorDiv(x,16)*13 + Math.floorDiv(z,16)*7, 31);
            case 5: return 50;
            default: return 8;
        }
    }
    static int[] compute(int x, int y, int z, int mode, boolean dark,
                         double flooded, double spread, double lavaNoise) {
        int low = Integer.MAX_VALUE;
        boolean under = false;
        int calls = 0;
        for (int i=0; i<13; i++) {
            int sx=x+OFFSETS[i][0]*16, sz=z+OFFSETS[i][1]*16;
            int surface=prelim(mode,sx,sz,i);
            calls++;
            int adjusted=surface+8;
            boolean center=i==0;
            if (center && y-12>adjusted) {
                int[] g=global(y); return new int[]{g[0],g[1],calls};
            }
            boolean near=y+12>adjusted;
            if (center || near) {
                int[] g=global(adjusted);
                if (adjusted<g[0]) {
                    if (center) under=true;
                    if (near) return new int[]{g[0],g[1],calls};
                }
            }
            low=Math.min(low,surface);
        }
        int[] g=global(y);
        double distance=(double)((long)low+8L-(long)y);
        double influence=under?1.0-Math.max(0.0,Math.min(1.0,distance/64.0)):0.0;
        double t=1.0-influence;
        double full=-0.3+t*(0.8-(-0.3));
        double partial=-0.8+t*(0.4-(-0.8));
        double noise=Math.max(-1.0,Math.min(1.0,flooded));
        int modeFlood=dark?0:noise>full?2:noise>partial?1:0;
        int fluidLevel;
        if (modeFlood==2) fluidLevel=g[0];
        else if (modeFlood==0) fluidLevel=-32512;
        else fluidLevel=Math.min(low,Math.floorDiv(y,40)*40+20+(int)Math.floor((spread*10.0)/3.0)*3);
        int state=(fluidLevel>-10 || fluidLevel==-32512 || g[1]==2) ? g[1]
                : Math.abs(lavaNoise)>0.3?2:g[1];
        return new int[]{fluidLevel,state,calls};
    }
    public static void main(String[] args) throws Exception {
        BufferedReader in=new BufferedReader(new InputStreamReader(System.in));
        StringBuilder out=new StringBuilder();
        String line;
        while ((line=in.readLine())!=null) {
            StringTokenizer t=new StringTokenizer(line);
            int x=Integer.parseInt(t.nextToken());
            int y=Integer.parseInt(t.nextToken());
            int z=Integer.parseInt(t.nextToken());
            int mode=Integer.parseInt(t.nextToken());
            boolean dark=t.nextToken().equals("1");
            double flood=Double.parseDouble(t.nextToken());
            double spread=Double.parseDouble(t.nextToken());
            double lava=Double.parseDouble(t.nextToken());
            int[] value=compute(x,y,z,mode,dark,flood,spread,lava);
            out.append(value[0]).append(' ').append(value[1]).append(' ').append(value[2]).append('\n');
        }
        System.out.print(out);
    }
}
"""


def prelim(mode, x, z, index):
    if mode == 0:
        return 100
    if mode == 1:
        return -40
    if mode == 2:
        return 30
    if mode == 3:
        return 30 if index == 0 else -20
    if mode == 4:
        return 35 + ((x // 16 * 13 + z // 16 * 7) % 31)
    if mode == 5:
        return 50
    return 8


def global_at(y):
    return (-54, 2) if y < -54 else (63, 1)


def model(case):
    x, y, z, mode, dark, flooded, spread, lava_noise = case
    lowest = 2**31 - 1
    under = False
    calls = 0
    for i, (dx, dz) in enumerate(OFFSETS):
        surface = prelim(mode, x + dx * 16, z + dz * 16, i)
        calls += 1
        adjusted = surface + 8
        center = i == 0
        if center and y - 12 > adjusted:
            return (*global_at(y), calls)
        near = y + 12 > adjusted
        if center or near:
            status = global_at(adjusted)
            if adjusted < status[0]:
                if center:
                    under = True
                if near:
                    return (*status, calls)
        lowest = min(lowest, surface)

    level, state = global_at(y)
    if dark:
        floodedness_mode = 0
    else:
        distance = float(lowest + 8 - y)
        influence = 1 - max(0., min(1., distance / 64.)) if under else 0.
        t = 1 - influence
        full = -0.3 + t * (0.8 + 0.3)
        partial = -0.8 + t * (0.4 + 0.8)
        flood = max(-1., min(1., flooded))
        floodedness_mode = 2 if flood > full else (1 if flood > partial else 0)
    if floodedness_mode == 0:
        level = -32512
    elif floodedness_mode == 1:
        level = min(lowest, (y // 40) * 40 + 20 + math.floor(spread * 10 / 3) * 3)
    if level <= -10 and level != -32512 and state != 2 and abs(lava_noise) > 0.3:
        state = 2
    return level, state, calls


class AquiferSurfaceArithmeticTest(unittest.TestCase):
    @staticmethod
    def cases():
        rng = random.Random(12011718)
        cases = []
        for mode in range(7):
            for y in (-80, -64, -54, -53, -40, -33, -12, -1, 0, 12, 31, 55, 62, 63, 100, 319):
                for x, z in ((0, 0), (-1, -17), (31, -15), (-63, 64)):
                    cases.append((x, y, z, mode, 0, 0.5, 0.0, 0.4))
        for _ in range(2800):
            cases.append((rng.randint(-300000, 300000), rng.randint(-64, 319),
                          rng.randint(-300000, 300000), rng.randrange(7), rng.randrange(2),
                          rng.choice((-1., -.8, -.3, .0, .3, .4, .5, .8, .800001, 1.)),
                          rng.uniform(-1., 1.), rng.choice((-.301, -.3, .0, .3, .301))))
        return cases

    def test_offsets_and_short_circuit_rules(self):
        self.assertEqual(len(OFFSETS), 13)
        self.assertEqual(len(set(OFFSETS)), 13)
        self.assertEqual(OFFSETS[0], (0, 0))
        self.assertEqual(model((0, 32, 0, 1, 0, 0.5, 0., 0.))[2], 1)
        self.assertEqual(model((0, 0, 0, 3, 0, 0.5, 0., 0.))[2], 2)
        self.assertEqual(model((0, -33, 0, 5, 1, 0.5, 0., 0.)), (-32512, 1, 13))

    def test_java_se_python_arithmetic(self):
        if not (shutil.which('java') and shutil.which('javac')):
            self.skipTest('JDK missing; cannot claim Java SE arithmetic comparison')
        cases = self.cases()
        with tempfile.TemporaryDirectory(prefix='horizon-1201-surface-') as tmp:
            source = Path(tmp) / 'AquiferSurfaceReference.java'
            source.write_text(JAVA, encoding='utf-8')
            subprocess.run(['javac', str(source)], check=True, capture_output=True, text=True)
            payload = ''.join(' '.join(map(str, case)) + '\n' for case in cases)
            process = subprocess.run(['java', '-cp', tmp, 'AquiferSurfaceReference'],
                                     input=payload, capture_output=True, text=True, check=True)
            lines = process.stdout.splitlines()
            self.assertEqual(len(cases), len(lines))
            for i, (case, line) in enumerate(zip(cases, lines)):
                self.assertEqual(tuple(map(int, line.split())), model(case), f'{i}: {case}')
            print(f'Java SE/Python surface and status: {len(cases)}/{len(cases)}')


if __name__ == '__main__':
    unittest.main()
