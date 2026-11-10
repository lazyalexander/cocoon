#!/usr/bin/env python3
import json
import pathlib
import subprocess
import unittest

ROOT = pathlib.Path(__file__).resolve().parent


class SnapshotApiTests(unittest.TestCase):
    def test_api_probe_is_explicit(self):
        result = subprocess.run(
            ["python3", str(ROOT / "probe.py"), "snapshot-api"],
            capture_output=True,
            text=True,
            timeout=60,
        )
        self.assertIn(result.returncode, [0, 2], result.stderr)
        report = json.loads(result.stdout)
        self.assertEqual(report["schema"], "cocoon-snapshot-api-probe-v1")
        self.assertIsInstance(report["api_available"], bool)
        self.assertFalse(report["guest_boot_tested"])
        if not report["api_available"]:
            self.assertTrue(report["compile_error"])
            self.assertEqual(result.returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
