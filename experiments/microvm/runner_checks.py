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
    def request(self, raw):
        with tempfile.TemporaryDirectory(prefix="cocoon-request-") as directory:
            path = pathlib.Path(directory) / "request.json"
            path.write_bytes(raw)
            result = subprocess.run([str(BINARY), str(path)], capture_output=True, timeout=10)
            self.assertEqual(result.returncode, 2)
            report = json.loads(result.stdout)
            self.assertEqual(report["status"], "invalid_request")
            self.assertFalse(report["termination_confirmed"])
            self.assertFalse(report["guest_ready"])
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
