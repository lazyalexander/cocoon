"""Acceptance checks must not disappear with Python optimization."""
import pathlib
import subprocess
import sys
import unittest
from persistence import require


class PersistenceChecks(unittest.TestCase):
    def test_failed_condition_is_an_error(self):
        with self.assertRaisesRegex(ValueError, 'not accepted'):
            require(False, 'not accepted')

    def test_optimized_python_still_refuses_failure(self):
        result = subprocess.run(
            [sys.executable, '-O', '-c', 'from persistence import require; require(False)'],
            cwd=pathlib.Path(__file__).parent, capture_output=True, timeout=10)
        self.assertNotEqual(result.returncode, 0)
        self.assertIn(b'ValueError', result.stderr)
