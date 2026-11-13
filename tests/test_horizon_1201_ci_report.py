"""Offline, game-data-free QA for the GitHub Actions diagnostic helper."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tools"))
from horizon_1201_ci_report import (  # noqa: E402
    error_excerpts, failure_tail, md, parse_log_argument, public_tree_section,
    rejected_paths, rust_section,
)


class CIReportTests(unittest.TestCase):
    def test_no_weakening_of_public_tree_predicate(self):
        calls = []
        def predicate(path):
            calls.append(path)
            return path.endswith(".jar") or path == "assets/texture.png"
        self.assertEqual(rejected_paths(["keep.rs", "assets/texture.png", "game.jar", "game.jar", ""], predicate),
                         ["assets/texture.png", "game.jar"])
        self.assertEqual(len(calls), 4)

    def test_rejections_are_visible(self):
        summary = public_tree_section(["assets/texture.png", "game.jar"])
        self.assertIn("**2**", summary)
        self.assertIn("`game.jar`", summary)
        self.assertIn("mandatory", summary)

    def test_zero_rejections_visible(self):
        self.assertIn("**0**", public_tree_section([]))

    def test_rust_compiler_and_dependency_errors_extracted(self):
        log = """Compiling world
error[E0308]: mismatched types
 --> src/lib.rs:5:2
  | expected a bool
Caused by: blocked mirror
error: could not compile `horizon-world`
"""
        result = error_excerpts(log)
        self.assertEqual(len(result), 3)
        self.assertIn("E0308", result[0])
        self.assertIn("Caused by", result[1])

    def test_ansi_coloured_rust_e0502_is_detected(self):
        # GitHub Actions streams contain colour escapes around error[E0502].
        log = ("\x1b[1m\x1b[91merror[E0502]\x1b[0m\x1b[1m: "
               "cannot borrow `calls` as immutable\x1b[0m\n"
               "\x1b[1m\x1b[91merror\x1b[0m: could not compile\n")
        excerpts = error_excerpts(log)
        self.assertEqual(len(excerpts), 2)
        self.assertIn("E0502", excerpts[0])
        self.assertNotIn("\x1b", excerpts[0])
        summary = rust_section("1.20.1", log, "1")
        self.assertIn("E0502", summary)
        self.assertNotIn("No recognised compiler/test error marker", summary)
        self.assertNotIn("\x1b", summary)

    def test_ansi_log_tail_is_sanitized(self):
        self.assertEqual(failure_tail("\x1b[33mwarning\x1b[0m\n"), ["warning"])

    def test_error_excerpt_is_capped(self):
        log = "\n".join(f"error[E0001]: issue {i}" for i in range(100))
        self.assertEqual(len(error_excerpts(log)), 12)

    def test_no_log_does_not_look_like_success(self):
        result = rust_section("1.20.1", None, "")
        self.assertIn("unknown", result)
        self.assertIn("No compiler log", result)

    def test_failure_not_reported_as_success(self):
        self.assertIn("failed", rust_section("regression", "error: failure", "101"))
        self.assertIn("passed", rust_section("regression", "ok", "0"))

    def test_test_panics_and_failures_are_detected(self):
        log = """running 5 tests
thread 'tests::bad' panicked at src/world.rs:23:5:
test tests::bad ... FAILED
test result: FAILED. 4 passed; 1 failed
"""
        errors = error_excerpts(log)
        self.assertEqual(len(errors), 3)
        self.assertIn("panicked", errors[0])

    def test_noncompiler_failure_shows_bounded_log_tail(self):
        log = """Compiling horizon-world
Finished test profile
permission denied by test harness
killed by signal 9
"""
        summary = rust_section("1.20.1", log, "1")
        self.assertIn("failed", summary)
        self.assertIn("Last non-empty log lines", summary)
        self.assertIn("killed by signal 9", summary)

    def test_tail_does_not_overflow_summary(self):
        log = "\n".join("line %d" % i for i in range(100))
        tail = failure_tail(log)
        self.assertEqual(len(tail), 24)
        self.assertEqual(tail[0], "line 76")
        self.assertEqual(tail[-1], "line 99")

    def test_success_does_not_emphasize_log_tail(self):
        self.assertNotIn("Last non-empty log lines", rust_section("world-regression", "done", "0"))

    def test_prevent_markdown_markup_in_report(self):
        self.assertNotIn("`", md("weird`path\nInjected"))
        self.assertNotIn("\n", md("weird`path\nInjected"))

    def test_rust_log_argument_parses_colon_path(self):
        label, path = parse_log_argument("world:/tmp/build:log.txt")
        self.assertEqual(label, "world")
        self.assertEqual(path, Path("/tmp/build:log.txt"))


if __name__ == "__main__":
    unittest.main()
