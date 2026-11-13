import unittest
from tools.test_legacy_scene_movement import compare_rows


class MovementComparisonTests(unittest.TestCase):
    def test_exact_rows_and_mismatches(self):
        expected = ["0 0 0000", "0 1 0001"]
        self.assertEqual(compare_rows(expected, expected[:], count=2), 2)
        with self.assertRaisesRegex(ValueError, "row 2"):
            compare_rows(expected, ["0 0 0000", "0 1 0000"], count=2)
        with self.assertRaisesRegex(ValueError, "Expected 2"):
            compare_rows(expected, expected[:1], count=2)
