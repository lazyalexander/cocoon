#!/usr/bin/env python3
"""Build the fixed Rust ext4 root fixture with explicit, recorded tool identities."""
import json
import os
import pathlib
import stat
import subprocess
import sys
import uuid
from probe import BUILD, ROOT, cpio, check_elf, digest
from root_modules import modules, VERSION

REPO = ROOT.parent.parent


def tool(variable, default, argument, expected):
    path = pathlib.Path(os.environ.get(variable, default)).absolute()
    if not path.is_file():
        raise ValueError(f"{variable} must name a regular file")
    result = subprocess.run([str(path), argument], capture_output=True, text=True, timeout=10)
    version = result.stdout + result.stderr
    if expected not in version:
        raise ValueError(f"unexpected {variable} tool identity")
    return path, {"sha256": digest(path), "version": version.strip()}


def build():
    project = "--project" in sys.argv[1:]
    linker, linker_info = tool("COCOON_LLD", "/opt/homebrew/opt/lld@22/bin/ld.lld", "--version", "LLD")
    mkfs, mkfs_info = tool("COCOON_MKE2FS", "/opt/homebrew/opt/e2fsprogs/sbin/mke2fs", "-V", "mke2fs")
    env = dict(os.environ, CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER=str(linker))
    # Reject ambient target/linker overrides that could silently change the build.
    for name in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "RUSTC", "RUSTC_WRAPPER", "RUSTUP_TOOLCHAIN", "CARGO_TARGET_DIR", "CARGO_BUILD_TARGET"]:
        if env.get(name):
            raise ValueError(f"unset ambient {name} before building the fixture")
    subprocess.run(["cargo", "build", "--locked", "-p", "cocoon-guest-fixture", "--target",
                    "aarch64-unknown-linux-musl", "--release"], cwd=REPO, env=env, check=True, timeout=180)
    if project:
        subprocess.run(["cargo", "build", "--locked", "-p", "cocoon-guest-helper", "--target", "aarch64-unknown-linux-musl", "--release"], cwd=REPO, env=env, check=True, timeout=180)
    binary = REPO / "target/aarch64-unknown-linux-musl/release/cocoon-guest-fixture"
    data = binary.read_bytes()
    check_elf(data)
    output = BUILD / ("root-" + uuid.uuid4().hex)
    output.mkdir(mode=0o700)
    staging = output / "tree"
    staging.mkdir()
    for name in ["proc", "dev", "tmp"]:
        (staging / name).mkdir()
    (staging / "guest-init").write_bytes(data)
    (staging / "guest-init").chmod(0o755)
    userland = None
    if project:
        from project_userland import install
        userland = install(staging)
        helper = REPO / "target/aarch64-unknown-linux-musl/release/cocoon-guest-helper"
        check_elf(helper.read_bytes())
        (staging / "guest-helper").write_bytes(helper.read_bytes())
        (staging / "guest-helper").chmod(0o755)
        userland["helper_sha256"] = digest(helper)
    disk = output / "root.raw"
    with disk.open("xb") as stream:
        stream.truncate(8 * 1024**3)
    # Keep mkfs behavior independent of machine-global mke2fs.conf.
    config = output / "mke2fs.conf"
    config.write_text("[defaults]\nbase_features = sparse_super,large_file,filetype,resize_inode,dir_index,ext_attr\nblocksize = 4096\ninode_size = 256\ninode_ratio = 16384\n[fs_types]\next4 = {\nfeatures = has_journal,extent\n}\n")
    subprocess.run([str(mkfs), "-q", "-t", "ext4", "-F", "-b", "4096", "-m", "0",
                    "-E", "lazy_itable_init=0,lazy_journal_init=0", "-d", str(staging), str(disk)],
                   env=dict(os.environ, MKE2FS_CONFIG=str(config)), check=True, timeout=120)
    initrd = output / "initrd.cpio.gz"
    loaded_modules = modules(project)
    initrd.write_bytes(cpio([
        ("dev", stat.S_IFDIR | 0o755, b"", 0, 0),
        ("dev/console", stat.S_IFCHR | 0o600, b"", 5, 1),
        ("dev/hvc0", stat.S_IFCHR | 0o600, b"", 229, 0),
        ("init", stat.S_IFREG | 0o755, data, 0, 0),
        ("modules", stat.S_IFDIR | 0o755, b"", 0, 0),
        ("module-order", stat.S_IFREG | 0o400, "\n".join(name for name,_ in loaded_modules).encode(), 0, 0),
    ] + [("modules/"+name+".ko", stat.S_IFREG | 0o400, content, 0, 0) for name,content in loaded_modules]))
    manifest = {"schema": "cocoon-root-fixture-v1", "guest": "fixed Rust Linux ARM64 PID1; MIT",
                "kernel_modules_version":VERSION, "modules":[name for name,_ in loaded_modules],
                "target": "aarch64-unknown-linux-musl", "tools": {"lld": linker_info, "mke2fs": mkfs_info},
                "rustc": subprocess.check_output(["rustc", "--version"], cwd=REPO, text=True).strip(),
                "guest_sha256": digest(binary), "initrd_sha256": digest(initrd), "root_sha256": digest(disk),
                "mkfs_config_sha256": digest(config), "logical_bytes": disk.stat().st_size,
                "allocated_bytes": disk.stat().st_blocks * 512, "guest_boot_tested": False, "project_userland": userland}
    (output / "manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    print(json.dumps({"private_output": str(output), **manifest}, indent=2))


if __name__ == "__main__":
    build()
