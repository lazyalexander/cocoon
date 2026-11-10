"""Select a minimal offline shell from the already pinned Debian installer.

This is not a complete Debian installation. No archives are extracted by path.
BusyBox is GPL-2.0; glibc is LGPL-2.1-or-later. See PROJECT.md for provenance.
"""
import gzip
import pathlib
from probe import BUILD, digest
from root_modules import SOURCES


def install(staging):
    source = BUILD / "debian/initrd.gz"
    if digest(source) != SOURCES["initrd.gz"][1]:
        raise ValueError("userland source digest mismatch")
    selected = {
        "usr/bin/busybox": "bin/busybox",
        "usr/lib/aarch64-linux-gnu/ld-linux-aarch64.so.1": "lib/ld-linux-aarch64.so.1",
        "usr/lib/aarch64-linux-gnu/libc.so.6": "lib/libc.so.6",
    }
    found = set()
    data = gzip.decompress(source.read_bytes())
    offset = 0
    while data[offset:offset+6] == b"070701":
        fields = [int(data[offset+6+i*8:offset+14+i*8], 16) for i in range(13)]
        start = offset + 110
        name = data[start:start+fields[11]-1].decode()
        start = (start+fields[11]+3)&~3
        content = data[start:start+fields[6]]
        offset = (start+fields[6]+3)&~3
        if name in selected:
            if name in found or fields[1] & 0o170000 != 0o100000:
                raise ValueError("unexpected userland entry")
            target = staging / selected[name]
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(content)
            target.chmod(0o755)
            found.add(name)
        if name == "TRAILER!!!": break
    if found != selected.keys(): raise ValueError("missing pinned userland")
    for name in ["sh", "cat", "mkdir", "ls", "rm", "cp", "mv", "sleep"]:
        (staging / "bin" / name).symlink_to("busybox")
    (staging / "workspace").mkdir()
    return {"source_sha256": digest(source), "files": {
        name: digest(staging / name) for name in selected.values()}}
