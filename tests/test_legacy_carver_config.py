import unittest
from tools.test_legacy_carver_config import compare_rows


class StrictCarverComparisonTests(unittest.TestCase):
    def test_rejects_missing_or_extra_reference_rows(self):
        for expected, actual in [(["B 0 1"], []), ([], ["B 0 1"]), (["C 1", "B 0 1"], ["C 1"])]:
            with self.subTest(expected=expected, actual=actual), self.assertRaisesRegex(ValueError, "Row count"):
                compare_rows(expected, actual)

    def test_single_bit_or_rng_tail_difference_is_an_error(self):
        for actual in ["R case 0 1065353217 9", "R case 0 1065353216 10"]:
            with self.subTest(actual=actual), self.assertRaisesRegex(ValueError, "Row 0 differs"):
                compare_rows(["R case 0 1065353216 9"], [actual])

    def test_identical_rows_are_accepted_in_order(self):
        rows = ["C case 1", "B case 0 1", "B case 1 0"]
        compare_rows(rows, list(rows))
        with self.assertRaisesRegex(ValueError, "Row 1 differs"):
            compare_rows(rows, [rows[0], rows[2], rows[1]])


if __name__ == "__main__":
    unittest.main()
