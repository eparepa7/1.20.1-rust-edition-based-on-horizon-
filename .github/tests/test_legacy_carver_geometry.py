from pathlib import Path
import struct
import tempfile
import unittest

from tools.test_legacy_carver_geometry import compare_bytes, geometry_counts


class GeometryTraceTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)

    def test_binary_comparison_checks_every_byte_and_both_lengths(self):
        reference = self.root / "java.bin"
        actual = self.root / "rust.bin"
        data = bytes(range(256)) * 8193
        reference.write_bytes(data)
        for changed in (data[:-1], data + b"x", data[:1048576] + b"x" + data[1048577:]):
            actual.write_bytes(changed)
            with self.subTest(length=len(changed)), self.assertRaisesRegex(ValueError, "differs at byte"):
                compare_bytes(reference, actual)
        actual.write_bytes(data)
        self.assertEqual(compare_bytes(reference,actual),len(data))

    def trace(self, points, selected=1, declared=None, suffix=b""):
        data = b"LCG1" + struct.pack(">IB",0,selected)
        data += b"".join(struct.pack(">i",point) for point in points)
        return data + struct.pack(">iIBq",-1,len(points) if declared is None else declared,selected,17) + suffix

    def test_counts_keep_debug_repeats_and_both_vertical_boundaries(self):
        path = self.root / "trace.bin"
        cases = [dict(carver="test:cave",min=0,height=16,upgrading=False)]
        path.write_bytes(self.trace([2<<8,8<<8,2<<8]))
        result = geometry_counts(path,cases)
        self.assertEqual(result["candidate_visits"],3)
        self.assertEqual(result["debug_revisits"],1)
        self.assertEqual(result["lower_boundary_visits"],2)
        self.assertEqual(result["upper_boundary_visits"],1)

    def test_bad_count_extra_data_bounds_and_unselected_visits_fail(self):
        path = self.root / "trace.bin"
        cases = [dict(carver="test:cave",min=0,height=16,upgrading=False)]
        for data in (self.trace([2<<8,8<<8,2<<8],declared=4),
                     self.trace([2<<8,8<<8,2<<8],suffix=b"extra"),
                     self.trace([1<<8,8<<8,1<<8]),
                     self.trace([2<<8,8<<8,2<<8],selected=0)):
            path.write_bytes(data)
            with self.subTest(data=data), self.assertRaises(ValueError):
                geometry_counts(path,cases)


if __name__=="__main__": unittest.main()
