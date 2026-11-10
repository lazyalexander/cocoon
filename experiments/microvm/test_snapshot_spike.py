#!/usr/bin/env python3
import argparse
import io
import json
import pathlib
import subprocess
import tempfile
import unittest
from unittest import mock

from experiments.microvm import probe

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

    def test_compile_error_redacts_workspace_and_home_paths(self):
        diagnostic = f"{ROOT}/snapshot_api_probe.swift: error near {pathlib.Path.home()}/private"
        completed = subprocess.CompletedProcess(["swiftc"], 1, "", diagnostic)
        output = io.StringIO()
        with mock.patch.object(probe.subprocess, "run", return_value=completed), \
                mock.patch("sys.stdout", output):
            self.assertEqual(probe.snapshot_api(), 2)
        report = json.loads(output.getvalue())
        self.assertNotIn(str(ROOT), report["compile_error"])
        self.assertNotIn(str(pathlib.Path.home()), report["compile_error"])
        self.assertIn("error", report["compile_error"])


class SnapshotAcceptanceTests(unittest.TestCase):
    def valid_reports(self):
        token = "a" * 32
        nonce = "0123456789abcdef0123456789abcdef"
        save = {"exit_code": 0, "snapshot_saved": True, "termination_confirmed": True,
                "guest_ready": True,
                "serial": f"COCOON_SNAPSHOT_READY:{token}:7:{nonce}\n"}
        restore = {"exit_code": 0, "snapshot_restored": True, "termination_confirmed": True,
                   "guest_ready": False,
                   "serial": f"COCOON_RESUMED:{token}:8:{nonce}\nCOCOON_DONE:{token}\n"}
        return save, restore

    def test_accepts_only_exact_continuous_restore(self):
        save, restore = self.valid_reports()
        self.assertEqual(probe.verify_snapshot(save, restore, 4096, "disk-a", "disk-a"), [])

    def test_rejects_missing_or_unconfirmed_evidence(self):
        for field, value in [("snapshot_saved", False), ("termination_confirmed", False),
                             ("exit_code", 2), ("guest_ready", False)]:
            with self.subTest(save_field=field):
                save, restore = self.valid_reports()
                save[field] = value
                self.assertTrue(probe.verify_snapshot(save, restore, 4096, "a", "a"))
        for field, value in [("termination_confirmed", False), ("exit_code", 2),
                             ("snapshot_restored", False)]:
            with self.subTest(restore_field=field):
                save, restore = self.valid_reports()
                restore[field] = value
                self.assertTrue(probe.verify_snapshot(save, restore, 4096, "a", "a"))
        save, restore = self.valid_reports()
        del restore["snapshot_restored"]
        self.assertTrue(probe.verify_snapshot(save, restore, 4096, "a", "a"))
        self.assertTrue(probe.verify_snapshot(save, restore, 0, "a", "a"))
        self.assertTrue(probe.verify_snapshot(save, restore, 4096, "a", "b"))

    def test_rejects_cold_boot_counter_reset_and_nonce_change(self):
        token = "a" * 32
        nonce = "0123456789abcdef0123456789abcdef"
        cases = [
            f"COCOON_READY:{token}\nCOCOON_SNAPSHOT_READY:{token}:7:{nonce}\n"
            f"COCOON_RESUMED:{token}:8:{nonce}\n",
            f"COCOON_RESUMED:{token}:7:{nonce}\n",
            f"COCOON_RESUMED:{token}:8:fedcba9876543210fedcba9876543210\n",
            f"COCOON_RESUMED:{'b' * 32}:8:{nonce}\n",
        ]
        for serial in cases:
            with self.subTest(serial=serial):
                save, restore = self.valid_reports()
                restore["serial"] = serial
                self.assertTrue(probe.verify_snapshot(save, restore, 4096, "a", "a"))

    def test_rejects_malformed_or_duplicate_nonce_evidence(self):
        save, restore = self.valid_reports()
        save["serial"] = save["serial"].replace("0123456789abcdef0123456789abcdef", "same")
        restore["serial"] = restore["serial"].replace("0123456789abcdef0123456789abcdef", "same")
        self.assertTrue(probe.verify_snapshot(save, restore, 4096, "a", "a"))
        save, restore = self.valid_reports()
        restore["serial"] = restore["serial"] + restore["serial"]
        self.assertTrue(probe.verify_snapshot(save, restore, 4096, "a", "a"))

    def test_timeout_preserves_unknown_outcome_without_restore(self):
        with tempfile.TemporaryDirectory(prefix="cocoon-snapshot-test-") as directory:
            root = pathlib.Path(directory)
            fixture = root / "fixture"
            fixture.mkdir()
            (fixture / "root.raw").write_bytes(b"disk")
            (fixture / "initrd.cpio.gz").write_bytes(b"initrd")
            kernel = root / "linux"
            kernel.write_bytes(b"kernel")
            args = argparse.Namespace(fixture=fixture, kernel=kernel, samples=1)
            with mock.patch.object(probe, "ROOT", root), mock.patch.object(
                    probe, "run_guest", side_effect=subprocess.TimeoutExpired("runner", 75)) as run:
                self.assertEqual(probe.snapshot(args), 2)
            self.assertEqual(run.call_count, 1)
            summaries = list((root / "runs").glob("snapshot-*/summary.json"))
            self.assertEqual(len(summaries), 1)
            summary = json.loads(summaries[0].read_text())
            self.assertFalse(summary["passed"])
            sample_dir = summaries[0].parent / "sample-1"
            report = json.loads((sample_dir / "result.json").read_text())
            self.assertEqual(report["save"]["status"], "runner_timeout")
            self.assertFalse(report["save"]["termination_confirmed"])
            self.assertIsNone(report["restore"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
