import pathlib
import tempfile
import unittest
from unittest.mock import patch

import fetch_kernel


class KernelAcquisitionTests(unittest.TestCase):
    def test_existing_corrupt_file_is_not_replaced_or_downloaded(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "linux"
            path.write_bytes(b"operator data")
            with patch.object(fetch_kernel.urllib.request, "urlopen") as download:
                with self.assertRaises(ValueError):
                    fetch_kernel.fetch(path)
                download.assert_not_called()
            self.assertEqual(path.read_bytes(), b"operator data")

    def test_correct_header_does_not_bypass_digest(self):
        with tempfile.TemporaryDirectory() as directory:
            path = pathlib.Path(directory) / "linux"
            data = bytearray(64)
            data[56:60] = b"ARM\x64"
            path.write_bytes(data)
            with self.assertRaisesRegex(ValueError, "digest mismatch"):
                fetch_kernel.verify(path)
