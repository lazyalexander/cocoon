#!/usr/bin/env python3
"""Explicitly fetch the pinned Debian ARM64 acceptance kernel, not a root image."""
import hashlib
import json
import os
import pathlib
import tempfile
import urllib.request

URL = "https://deb.debian.org/debian/dists/trixie/main/installer-arm64/current/images/netboot/debian-installer/arm64/linux"
SHA256 = "9c37ace54ef7e1919411fd912e9f34d6982ddd7f4c8ef3c8aee56f4c4d70db67"
MAX_BYTES = 128 * 1024 * 1024
DESTINATION = pathlib.Path(__file__).resolve().parent / "build" / "debian" / "linux"


def verify(path):
    digest = hashlib.sha256()
    size = 0
    with path.open("rb") as source:
        header = source.read(64)
        if header[56:60] != b"ARM\x64":
            raise ValueError("not an uncompressed ARM64 Linux Image")
        digest.update(header)
        size += len(header)
        while chunk := source.read(1024 * 1024):
            size += len(chunk)
            if size > MAX_BYTES:
                raise ValueError("kernel exceeds download limit")
            digest.update(chunk)
    if digest.hexdigest() != SHA256:
        raise ValueError("kernel digest mismatch; review upstream changes, never silently repin")
    return size


def fetch(destination=DESTINATION):
    destination.parent.mkdir(parents=True, exist_ok=True)
    if destination.exists():
        return verify(destination)
    descriptor, name = tempfile.mkstemp(prefix="kernel-", dir=destination.parent)
    temporary = pathlib.Path(name)
    try:
        with os.fdopen(descriptor, "wb") as output:
            with urllib.request.urlopen(URL, timeout=30) as response:
                if not response.url.startswith("https://"):
                    raise ValueError("insecure kernel redirect")
                size = 0
                while chunk := response.read(1024 * 1024):
                    size += len(chunk)
                    if size > MAX_BYTES:
                        raise ValueError("kernel exceeds download limit")
                    output.write(chunk)
            output.flush()
            os.fsync(output.fileno())
        size = verify(temporary)
        # Publish only verified bytes; never overwrite an existing operator file.
        os.link(temporary, destination)
        return size
    finally:
        temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    size = fetch()
    print(json.dumps({"source": URL, "sha256": SHA256, "bytes": size,
                      "guest_boot_tested": False,
                      "license": "Linux kernel: GPL-2.0-only; Debian packaging and notices apply"}, indent=2))
