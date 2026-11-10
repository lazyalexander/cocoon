import gzip
import stat
import struct
import tempfile
import pathlib
import contextlib
import io
import unittest
from types import SimpleNamespace
from unittest.mock import patch

import probe
from probe import cpio, check_elf, run_guest


def unpack(archive):
    data = gzip.decompress(archive)
    offset = 0
    files = {}
    while offset < len(data):
        if data[offset:offset + 6] != b"070701":
            raise AssertionError("invalid newc magic or alignment")
        fields = [int(data[offset + 6 + i * 8:offset + 14 + i * 8], 16) for i in range(13)]
        length, namesize = fields[6], fields[11]
        start = offset + 110
        name = data[start:start + namesize - 1].decode()
        if name == "TRAILER!!!":
            return files
        start = (start + namesize + 3) & ~3
        files[name] = (fields, data[start:start + length])
        offset = (start + length + 3) & ~3
    raise AssertionError("missing cpio trailer")


class GuestImageTests(unittest.TestCase):
    def test_harness_delivers_cancellation_to_its_owned_process(self):
        # Exercises local process signaling only; this is not VM acceptance.
        with tempfile.TemporaryDirectory() as directory:
            child = pathlib.Path(directory) / "child.py"
            child.write_text(
                "#!/usr/bin/env python3\nimport signal, time, sys\n"
                "def stop(*_):\n    print('stopped', flush=True)\n    sys.exit(0)\n"
                "signal.signal(signal.SIGTERM, stop)\nprint('ready', flush=True)\n"
                "while True: time.sleep(0.01)\n"
            )
            child.chmod(0o700)
            result = run_guest(child, pathlib.Path(directory) / "unused", 2, True)
            self.assertEqual(result.returncode, 0)
            self.assertEqual(result.stdout, b"ready\nstopped\n")

    def test_harness_timeout_reaps_runner_without_turning_loss_into_success(self):
        timeout = probe.subprocess.TimeoutExpired(["runner"], 16)
        with patch.object(probe.subprocess, "Popen") as spawn:
            process = spawn.return_value.__enter__.return_value
            process.communicate.side_effect = [timeout, (b"", b"")]
            with self.assertRaises(probe.subprocess.TimeoutExpired):
                run_guest("runner", "request", 1, False)
            process.kill.assert_called_once_with()
            self.assertEqual(process.communicate.call_count, 2)

    def test_archive_preserves_input_modes_devices_and_alignment(self):
        entries = [
            ("dev", stat.S_IFDIR | 0o755, b"", 0, 0),
            ("dev/hvc0", stat.S_IFCHR | 0o600, b"", 229, 0),
            ("init", stat.S_IFREG | 0o755, b"ELF-test-bytes", 0, 0),
            ("input", stat.S_IFREG | 0o400, b"abc\x00xyz\n", 0, 0),
        ]
        archive = cpio(entries)
        self.assertEqual(archive, cpio(entries))
        files = unpack(archive)
        self.assertEqual(files["input"][1], b"abc\x00xyz\n")
        self.assertEqual(files["init"][0][1], stat.S_IFREG | 0o755)
        self.assertEqual(files["dev/hvc0"][0][9:11], [229, 0])

    def test_elf_rejects_wrong_architecture_dynamic_loader_and_truncation(self):
        data = bytearray(120)
        data[:6] = b"\x7fELF\x02\x01"
        struct.pack_into("<HH", data, 16, 2, 183)
        struct.pack_into("<Q", data, 32, 64)
        struct.pack_into("<HH", data, 54, 56, 1)
        struct.pack_into("<I", data, 64, 1)
        check_elf(data)
        for broken in [data[:80], b"MachO", data[:18] + b"\x3e\x00" + data[20:]]:
            with self.assertRaises(ValueError):
                check_elf(broken)
        struct.pack_into("<I", data, 64, 3)
        with self.assertRaises(ValueError):
            check_elf(data)

    def test_benchmark_stops_on_failure_without_inventing_boot_metrics(self):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            (root / "runs").mkdir()
            failure = {"passed": False, "runner": {"status": "blocked"}}
            with patch.object(probe, "ROOT", root), patch.object(
                probe, "boot", return_value=(failure, root / "result.json")
            ) as boot, contextlib.redirect_stdout(io.StringIO()):
                self.assertEqual(probe.benchmark(SimpleNamespace(samples=30)), 2)
            self.assertEqual(boot.call_count, 1)
            report = probe.json.loads(next((root / "runs").glob("benchmark-*.json")).read_text())
            self.assertEqual(report["passed_samples"], 0)
            self.assertFalse(report["complete"])
            self.assertTrue(all(value is None for value in report["metrics"].values()))


if __name__ == "__main__":
    unittest.main()
