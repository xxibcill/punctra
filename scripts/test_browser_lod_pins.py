"""Local pin-closure commands distinguish a clean result from execution failure."""

import importlib.util
from pathlib import Path
import sys
import unittest


SPEC = importlib.util.spec_from_file_location(
    "punctra_browser_server", Path(__file__).with_name("serve-browser-demo.py")
)
SERVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(SERVER)


class ClosureCommandTests(unittest.TestCase):
    def test_clean_empty_status_remains_a_successful_empty_string(self):
        self.assertEqual(SERVER.command_text(sys.executable, "-c", "pass", allow_empty=True), "")
        self.assertIsNone(SERVER.command_text(sys.executable, "-c", "pass"))

    def test_failed_command_cannot_look_like_a_clean_or_qualified_result(self):
        self.assertIsNone(SERVER.command_text(sys.executable, "-c", "raise SystemExit(1)", allow_empty=True))
        self.assertIsNone(SERVER.command_text(sys.executable, "-c", "print('qualified'); raise SystemExit(1)"))


if __name__ == "__main__":
    unittest.main()
