#!/usr/bin/env python3
"""Black-box checks of a built macOS runner. Does not start a guest VM."""
import json
import pathlib
import subprocess
import sys
import tempfile
import unittest

BINARY = None


class RunnerProtocolTests(unittest.TestCase):
    def response(self, raw):
        with tempfile.TemporaryDirectory(prefix="cocoon-request-") as directory:
            path = pathlib.Path(directory) / "request.json"
            path.write_bytes(raw)
            result = subprocess.run([str(BINARY), str(path)], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            return json.loads(result.stdout)

    def request(self, raw):
        report = self.response(raw)
        self.assertEqual(report["status"], "invalid_request")
        self.assertFalse(report["termination_confirmed"])
        self.assertFalse(report["guest_ready"])
        self.assertNotEqual(report.get("snapshot_saved"), True)
        self.assertNotIn("start_requested_ms", report)

    def test_unknown_fields_and_invalid_bounds_refuse_before_vm_start(self):
        base = {"kernel": "/unused-kernel", "initrd": "/unused-initrd",
                "token": "a" * 32, "memoryMiB": 128, "seconds": 1}
        for change in [{"network": True}, {"seconds": 0}, {"seconds": 61},
                       {"memoryMiB": 127}, {"token": "../"}, {"memoryMiB": -1}]:
            with self.subTest(change=change):
                self.request(json.dumps({**base, **change}).encode())

    def test_disk_fields_are_complete_and_bounded(self):
        base = {"kernel":"/unused-kernel", "initrd":"/unused-initrd", "token":"a"*32,
                "memoryMiB":512, "seconds":1}
        for change in [{"rootDisk":"/unused"},
                       {"rootDisk":None,"diskMode":"write","payload":"a"*32},
                       {"rootDisk":"relative","diskMode":"read","payload":"a"*32},
                       {"rootDisk":"/unused","diskMode":"shell","payload":"a"*32},
                       {"rootDisk":"/unused","diskMode":"write","payload":"bad"}]:
            with self.subTest(change=change):
                self.request(json.dumps({**base, **change}).encode())

    def test_project_mode_requires_explicit_project_path(self):
        base = {"kernel":"/unused", "initrd":"/unused", "token":"a"*32,
                "memoryMiB":512, "seconds":1, "rootDisk":"/unused", "payload":"a"*32}
        for change in [{"diskMode":"project"},
                       {"diskMode":"project", "project":"relative"},
                       {"diskMode":"write", "project":"/unused"},
                       {"diskMode":"project", "project":None}]:
            with self.subTest(change=change):
                self.request(json.dumps({**base, **change}).encode())

    def test_snapshot_fields_are_complete_bounded_and_unambiguous(self):
        base = {"kernel": "/unused-kernel", "initrd": "/unused-initrd",
                "token": "a" * 32, "memoryMiB": 512, "seconds": 1}
        cases = [
            {"snapshotMode": "save"},
            {"snapshotMode": "restore"},
            {"snapshotMode": "other", "snapshotPath": "/unused-state"},
            {"snapshotMode": "save", "snapshotPath": "relative-state"},
            {"snapshotMode": "restore", "restorePath": "relative-state"},
            {"snapshotMode": "save", "snapshotPath": "/state", "restorePath": "/state"},
            {"snapshotMode": "restore", "restorePath": "/unused-kernel"},
            {"snapshotMode": "save", "snapshotPath": "/" + "a" * 4096},
            {"snapshotMode": "save", "restorePath": "/state"},
            {"snapshotMode": "restore", "snapshotPath": "/state"},
            {"snapshotPath": "/state"},
            {"restorePath": "/state"},
        ]
        for change in cases:
            with self.subTest(change=change):
                self.request(json.dumps({**base, **change}).encode())
        valid = [
            {"snapshotMode": "save", "snapshotPath": "/unused-state"},
            {"snapshotMode": "restore", "restorePath": "/unused-state"},
            {"rootDisk": "/unused-root", "diskMode": "snapshot", "payload": "a" * 32,
             "snapshotMode": "save", "snapshotPath": "/unused-state"},
        ]
        for change in valid:
            with self.subTest(valid=change):
                report = self.response(json.dumps({**base, **change}).encode())
                self.assertNotEqual(report["status"], "invalid_request")

    def test_existing_snapshot_destination_refuses_before_vm_or_side_effects(self):
        with tempfile.TemporaryDirectory(prefix="cocoon-snapshot-request-") as directory:
            state = pathlib.Path(directory) / "state.vz"
            state.write_bytes(b"existing")
            request = {"kernel": "/unused-kernel", "initrd": "/unused-initrd",
                       "token": "a" * 32, "memoryMiB": 512, "seconds": 1,
                       "snapshotMode": "save", "snapshotPath": str(state)}
            path = pathlib.Path(directory) / "request.json"
            path.write_text(json.dumps(request))
            result = subprocess.run([str(BINARY), str(path)], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            report = json.loads(result.stdout)
            self.assertEqual(report["status"], "snapshot_setup_failed")
            self.assertNotIn("start_requested_ms", report)
            self.assertEqual(state.read_bytes(), b"existing")
            self.assertFalse(pathlib.Path(str(state) + ".machine-id").exists())

    def test_malformed_missing_and_oversized_requests_refuse(self):
        for raw in [b"[]", b"{}", b"not-json", b" " * 16385]:
            with self.subTest(length=len(raw)):
                self.request(raw)

    def test_doctor_reports_read_denials_without_claiming_guest_execution(self):
        result = subprocess.run([str(BINARY), "--doctor"], capture_output=True, timeout=10)
        self.assertIn(result.returncode, [0, 2])
        report = json.loads(result.stdout)
        self.assertEqual(report["schema"], "cocoon-vz-doctor-v1")
        self.assertEqual(report["status"], "diagnostic_only")
        self.assertFalse(report["guest_boot_tested"])
        self.assertFalse(report["termination_confirmed"])
        self.assertNotIn("start_requested_ms", report)
        for probe in report["diagnostics"]["sysctl"].values():
            self.assertIsInstance(probe["readable"], bool)
            self.assertIn("value" if probe["readable"] else "errno", probe)


if __name__ == "__main__":
    if len(sys.argv) != 2:
        raise SystemExit("usage: runner_checks.py /absolute/path/to/cocoon-vz")
    BINARY = pathlib.Path(sys.argv.pop()).resolve(strict=True)
    unittest.main(verbosity=2)
