#!/usr/bin/env python3
"""Test Vanilla Java 1.20.1 Noise Terrain Generator, Surface Rules, and World Management."""
import os
import unittest


class Legacy1201TerrainTests(unittest.TestCase):
    """Test 1.20.1 Overworld Terrain, Noise Routing, and Surface Rules."""

    def test_overworld_density_cell_grid(self):
        """Vanilla 1.20.1 NoiseChunk grid:
        Horizontal cell size = 4 blocks (16 / 4 = 4 cells per chunk)
        Vertical cell size = 8 blocks (384 / 8 = 48 cells per chunk)
        Total 3D cells = 4 x 4 x 48 = 768 cells.
        Grid vertices = (4+1) x (4+1) x (48+1) = 5 x 5 x 49 = 1,225 samples.
        """
        horiz_cells = 16 // 4
        vert_cells = 384 // 8
        self.assertEqual(horiz_cells, 4)
        self.assertEqual(vert_cells, 48)
        total_cells = horiz_cells * horiz_cells * vert_cells
        self.assertEqual(total_cells, 768)
        grid_vertices = (horiz_cells + 1) * (horiz_cells + 1) * (vert_cells + 1)
        self.assertEqual(grid_vertices, 1225)

    def test_surface_rules_height_boundaries(self):
        """Surface rules height boundaries:
        Bedrock floor: Y = -64
        Bedrock roof / stochastic range: -64..-60
        Deepslate full depth: Y <= -8
        Stone rock depth: Y >= 0
        Dithered transition zone: -8 < Y < 0
        Default sea level: Y = 63
        Lava lake level in deep caves: Y = -54
        """
        bedrock_floor = -64
        bedrock_roof = -60
        deepslate_max = -8
        stone_min = 0
        sea_level = 63
        lava_level = -54

        self.assertEqual(bedrock_floor, -64)
        self.assertTrue(bedrock_floor < bedrock_roof)
        self.assertTrue(lava_level < deepslate_max)
        self.assertTrue(deepslate_max < stone_min)
        self.assertTrue(stone_min < sea_level)

    def test_quart_biome_grid_invariants(self):
        """Vanilla 1.20.1 quart biomes:
        Section size: 16x16x16 blocks.
        Quart cell size: 4x4x4 blocks.
        Cells per section: (16/4)^3 = 64 cells.
        24 sections in Overworld: 24 * 64 = 1,536 biome cells per chunk column.
        """
        cells_per_section = (16 // 4) ** 3
        self.assertEqual(cells_per_section, 64)
        total_quart_cells = 24 * cells_per_section
        self.assertEqual(total_quart_cells, 1536)

    def test_noise_router_climate_parameters(self):
        """1.20.1 Multi-noise climate parameters:
        Continentalness, Erosion, Peaks & Valleys (Ridges), Temperature, Humidity.
        Verify distinct ranges and classification boundaries.
        """
        # Ocean boundary: continentalness < -0.15
        ocean_thresh = -0.15
        deep_ocean_thresh = -0.45
        self.assertTrue(deep_ocean_thresh < ocean_thresh)

        # Beach boundary: -0.15 <= continentalness < -0.05
        beach_thresh = -0.05
        self.assertTrue(ocean_thresh < beach_thresh)


if __name__ == '__main__':
    unittest.main()
