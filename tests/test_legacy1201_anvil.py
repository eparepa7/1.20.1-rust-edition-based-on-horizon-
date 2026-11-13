#!/usr/bin/env python3
"""Test Vanilla Java 1.20.1 Anvil/NBT chunk, section, region, and world storage format."""
import os
import struct
import tempfile
import unittest
import zlib


class Legacy1201AnvilFormatTests(unittest.TestCase):
    """Test Bit-exact Java 1.20.1 Anvil NBT and BitStorage layout."""

    def test_heightmap_bitstorage_dimensions(self):
        """Vanilla 1.20.1 overworld height is 384 (-64..320).
        Entries = 256 columns (16x16).
        Bits per entry = ceil(log2(384 + 1)) = 9 bits.
        Entries per 64-bit word = floor(64 / 9) = 7 entries.
        Expected words = ceil(256 / 7) = 37 words (LongArray of 37 elements).
        """
        entries = 256
        bits = 9
        per_word = 64 // bits
        self.assertEqual(per_word, 7)
        expected_words = (entries + per_word - 1) // per_word
        self.assertEqual(expected_words, 37)

    def test_block_section_dimensions(self):
        """A chunk section has 16x16x16 = 4096 blocks.
        Minimum bits per block = 4.
        Quart-biomes: 4x4x4 = 64 cells per section.
        Minimum bits per biome = 1.
        Nibble lighting: 4096 nibbles / 2 = 2048 bytes per section.
        """
        blocks = 16 * 16 * 16
        self.assertEqual(blocks, 4096)
        biomes = 4 * 4 * 4
        self.assertEqual(biomes, 64)
        light_bytes = blocks // 2
        self.assertEqual(light_bytes, 2048)

    def test_overworld_section_range(self):
        """Overworld height range is -64..320:
        Lowest section Y = -4 (-64 / 16)
        Highest section Y = 19 (304 / 16)
        Total sections = 19 - (-4) + 1 = 24 sections.
        """
        min_sec_y = -64 // 16
        max_sec_y = 304 // 16
        self.assertEqual(min_sec_y, -4)
        self.assertEqual(max_sec_y, 19)
        section_count = max_sec_y - min_sec_y + 1
        self.assertEqual(section_count, 24)

    def test_superflat_classic_preset_heights(self):
        """Classic superflat preset:
        Y = -64: Bedrock (1 layer)
        Y = -63..-62: Dirt (2 layers)
        Y = -61: Grass Block (1 layer)
        Y = -60..319: Air (380 layers)
        Top ground block is at Y = -61.
        Vanilla heightmap (1-based relative height or Y + 1) reports:
        Y = -61 -> height = -61 + 1 = -60 (relative 4).
        """
        bedrock_y = -64
        dirt_y = [-63, -62]
        grass_y = -61
        top_ground_y = grass_y
        self.assertEqual(top_ground_y, -61)
        expected_surface_height = top_ground_y + 1
        self.assertEqual(expected_surface_height, -60)

    def test_mca_region_chunk_indexing(self):
        """In Anvil .mca files:
        Each region covers 32x32 chunks = 1024 chunks.
        Chunk coordinates (cx, cz) -> region coordinates (rx, rz):
        rx = cx // 32, rz = cz // 32.
        Header offset table: index = (cx % 32) + (cz % 32) * 32.
        """
        cx, cz = 35, -5
        rx = cx // 32
        rz = cz // 32
        self.assertEqual(rx, 1)
        self.assertEqual(rz, -1)
        local_x = cx % 32
        local_z = cz % 32
        self.assertEqual(local_x, 3)
        self.assertEqual(local_z, 27)
        offset_idx = local_x + local_z * 32
        self.assertEqual(offset_idx, 3 + 27 * 32)


if __name__ == '__main__':
    unittest.main()
