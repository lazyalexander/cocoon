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


class SnapshotAcceptanceTests(unittest.TestCase):
    def valid_reports(self):
        token = "a" * 32
        save = {"exit_code": 0, "snapshot_saved": True, "termination_confirmed": True,
                "guest_ready": True, "serial": f"COCOON_SNAPSHOT_READY:{token}:7\n"}
        restore = {"exit_code": 0, "termination_confirmed": True, "guest_ready": False,
                   "serial": f"COCOON_RESUMED:{token}:8\nCOCOON_DONE:{token}\n"}
        return save, restore

    def test_accepts_only_exact_continuous_restore(self):
        from experiments.microvm.probe import verify_snapshot
        save, restore = self.valid_reports()
        self.assertEqual(verify_snapshot(save, restore, 4096, "disk-a", "disk-a"), [])

    def test_rejects_missing_or_unconfirmed_evidence(self):
        from experiments.microvm.probe import verify_snapshot
        for field, value in [("snapshot_saved", False), ("termination_confirmed", False),
                             ("exit_code", 2), ("guest_ready", False)]:
            with self.subTest(save_field=field):
                save, restore = self.valid_reports()
                save[field] = value
                self.assertTrue(verify_snapshot(save, restore, 4096, "a", "a"))
        for field, value in [("termination_confirmed", False), ("exit_code", 2)]:
            with self.subTest(restore_field=field):
                save, restore = self.valid_reports()
                restore[field] = value
                self.assertTrue(verify_snapshot(save, restore, 4096, "a", "a"))
        save, restore = self.valid_reports()
        self.assertTrue(verify_snapshot(save, restore, 0, "a", "a"))
        self.assertTrue(verify_snapshot(save, restore, 4096, "a", "b"))

    def test_rejects_cold_boot_and_counter_reset(self):
        from experiments.microvm.probe import verify_snapshot
        for serial in ["COCOON_READY:" + "a" * 32 + "\n",
                       "COCOON_RESUMED:" + "a" * 32 + ":7\n",
                       "COCOON_RESUMED:" + "b" * 32 + ":8\n"]:
            with self.subTest(serial=serial):
                save, restore = self.valid_reports()
                restore["serial"] = serial
                self.assertTrue(verify_snapshot(save, restore, 4096, "a", "a"))


if __name__ == "__main__":
    unittest.main(verbosity=2)
