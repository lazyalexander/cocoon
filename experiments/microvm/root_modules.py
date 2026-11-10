#!/usr/bin/env python3
"""Explicit pinned Debian module acquisition for the fixed root experiment."""
import gzip
import hashlib
import io
import json
import lzma
import pathlib
import re
import subprocess
import tarfile
import urllib.request
from probe import BUILD, digest

VERSION = "6.12.107+deb13-arm64"
SOURCES = {
    "initrd.gz": ("https://deb.debian.org/debian/dists/trixie/main/installer-arm64/current/images/netboot/debian-installer/arm64/initrd.gz", "f8f48fe31f84fca1a43c3d57c09cd1c2ee728549dd18a986463efd0305249451"),
    "ext4-modules-6.12.107+deb13-arm64-di_6.12.107-1_arm64.udeb": ("https://deb.debian.org/debian/pool/main/l/linux-signed-arm64/ext4-modules-6.12.107+deb13-arm64-di_6.12.107-1_arm64.udeb", "ef0bd6fd637ec1f65c3923e2f0e3af0a580068061849bb823a755357391deae0"),
    "scsi-modules-6.12.107+deb13-arm64-di_6.12.107-1_arm64.udeb": ("https://deb.debian.org/debian/pool/main/l/linux-signed-arm64/scsi-modules-6.12.107+deb13-arm64-di_6.12.107-1_arm64.udeb", "7b6e3770378f1b020f6bf09571383cdbc928a51e8dc733e93a6b1afa1fa788dc"),
}


def modules():
    files = {}
    for name, (_, expected) in SOURCES.items():
        path = BUILD / "debian" / name
        if digest(path) != expected:
            raise ValueError("module source digest mismatch")
        if name == "initrd.gz":
            data = gzip.decompress(path.read_bytes())
            offset = 0
            while data[offset:offset+6] == b"070701":
                fields = [int(data[offset+6+i*8:offset+14+i*8],16) for i in range(13)]
                start = offset+110
                member = data[start:start+fields[11]-1].decode()
                start = (start+fields[11]+3)&~3
                content = data[start:start+fields[6]]
                offset = (start+fields[6]+3)&~3
                if member.endswith(".ko.xz"):
                    files[pathlib.PurePosixPath(member).name[:-6].replace("-","_")] = lzma.decompress(content)
                if member == "TRAILER!!!":
                    break
        else:
            archive = subprocess.check_output(["/usr/bin/tar", "-xOf", str(path), "data.tar.xz"],timeout=30)
            with tarfile.open(fileobj=io.BytesIO(archive), mode="r:xz") as tar:
                for item in tar:
                    if item.isfile() and item.name.endswith(".ko.xz"):
                        files[pathlib.PurePosixPath(item.name).name[:-6].replace("-","_")] = lzma.decompress(tar.extractfile(item).read())
    order = []
    visiting = set()
    def add(name):
        if name in order: return
        if name in visiting: raise ValueError("module dependency cycle")
        visiting.add(name)
        data = files[name]
        if VERSION.encode() not in data: raise ValueError("module kernel version mismatch")
        match = re.search(rb"(?:^|\x00)depends=([^\x00]*)",data)
        for dep in (match.group(1).decode().split(",") if match else []):
            if dep: add(dep.replace("-","_"))
        order.append(name)
    add("virtio_blk")
    # ext4 requests this crypto algorithm dynamically; it is not a depends= edge.
    add("crc32c_generic")
    add("ext4")
    return [(name,files[name]) for name in order]


if __name__ == "__main__":
    (BUILD / "debian").mkdir(parents=True,exist_ok=True)
    for name,(url,expected) in SOURCES.items():
        path = BUILD / "debian" / name
        if not path.exists():
            with urllib.request.urlopen(url,timeout=30) as stream:
                data=stream.read(64*1024*1024+1)
            if len(data)>64*1024*1024 or hashlib.sha256(data).hexdigest()!=expected:
                raise ValueError("module source size/digest mismatch")
            with path.open("xb") as output: output.write(data)
        if digest(path)!=expected: raise ValueError("existing module source mismatch")
    print(json.dumps({"kernel_version":VERSION,"modules":[name for name,_ in modules()],"guest_boot_tested":False}))
