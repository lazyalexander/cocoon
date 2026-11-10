#!/usr/bin/env python3
"""Build a tiny ARM64 Linux guest and exercise one real, diskless macOS VM."""
import argparse
import base64
import gzip
import hashlib
import json
import pathlib
import shutil
import math
import stat
import statistics
import struct
import subprocess
import signal
import threading
import time
import uuid
import zlib

ROOT = pathlib.Path(__file__).resolve().parent
BUILD = ROOT / "build"


def cpio(entries):
    """Deterministic Linux newc archive, including explicit device nodes."""
    result = bytearray()
    for inode, (name, mode, data, major, minor) in enumerate(
        [*entries, ("TRAILER!!!", 0, b"", 0, 0)], 1
    ):
        filename = name.encode() + b"\0"
        fields = [inode, mode, 0, 0, 1, 0, len(data), 0, 0, major, minor, len(filename), 0]
        result.extend(b"070701" + b"".join(f"{x:08x}".encode() for x in fields))
        result.extend(filename)
        result.extend(b"\0" * (-len(result) % 4))
        result.extend(data)
        result.extend(b"\0" * (-len(result) % 4))
    return gzip.compress(result, compresslevel=1, mtime=0)


def check_elf(data):
    if len(data) < 64 or data[:6] != b"\x7fELF\x02\x01":
        raise ValueError("guest must be a little-endian ELF64 executable")
    if struct.unpack_from("<HH", data, 16) != (2, 183):
        raise ValueError("guest must be an ARM64 ET_EXEC binary")
    offset = struct.unpack_from("<Q", data, 32)[0]
    size, count = struct.unpack_from("<HH", data, 54)
    if size < 56 or offset + size * count > len(data):
        raise ValueError("invalid ELF program header table")
    types = [struct.unpack_from("<I", data, offset + i * size)[0] for i in range(count)]
    if 1 not in types or 3 in types:
        raise ValueError("guest must have loadable segments and no dynamic interpreter")


def digest(path):
    h = hashlib.sha256()
    with path.open("rb") as stream:
        for part in iter(lambda: stream.read(1024 * 1024), b""):
            h.update(part)
    return h.hexdigest()


def check_kernel(path):
    """Reject known-incompatible Images; configuration is not boot evidence."""
    data = path.read_bytes()
    if data[56:60] != b"ARM\x64":
        raise ValueError("kernel is not an uncompressed Linux ARM64 Image")
    report = {"schema": "cocoon-kernel-check-v1", "kernel_sha256": digest(path),
              "embedded_config": False, "compatibility": "unknown",
              "guest_boot_tested": False}
    offset = data.find(b"IKCFG_ST")
    if offset < 0:
        return report
    # Kernel input is untrusted. Bound decompression before interpreting IKCONFIG.
    decoder = zlib.decompressobj(31)
    try:
        config = decoder.decompress(data[offset + 8:], 2 * 1024 * 1024)
        if not decoder.eof or not decoder.unused_data.startswith(b"IKCFG_ED"):
            raise ValueError("invalid or oversized embedded kernel configuration")
        lines = config.decode("utf-8").splitlines()
    except (zlib.error, UnicodeDecodeError) as error:
        raise ValueError("invalid embedded kernel configuration") from error
    options = dict(line.split("=", 1) for line in lines
                   if line.startswith("CONFIG_") and "=" in line)
    required = ["CONFIG_BLK_DEV_INITRD", "CONFIG_RD_GZIP", "CONFIG_BINFMT_ELF",
                "CONFIG_VIRTIO", "CONFIG_VIRTIO_CONSOLE"]
    missing = [name for name in required if options.get(name) != "y"]
    report.update(embedded_config=True, compatibility="incompatible" if missing else "candidate",
                  required_options={name: options.get(name, "disabled") for name in required})
    if missing:
        report["error"] = "kernel requires built-in (=y): " + ", ".join(missing)
    return report


def build(linker):
    BUILD.mkdir(exist_ok=True)
    if not linker:
        linker = shutil.which("ld.lld")
    if not linker:
        choices = sorted(pathlib.Path("/opt/homebrew/opt").glob("lld*/bin/ld.lld"))
        linker = str(choices[-1]) if choices else None
    if not linker:
        raise RuntimeError("LLVM ld.lld required; supply --linker")
    subprocess.run(["/usr/bin/clang", "--target=aarch64-linux-gnu", "-Oz", "-ffreestanding",
                    "-fno-builtin", "-fno-stack-protector", "-fno-pic", "-fno-pie",
                    "-Wall", "-Wextra", "-Werror", "-c", str(ROOT / "guest/init.c"),
                    "-o", str(BUILD / "init.o")], check=True, timeout=60)
    subprocess.run([linker, "-static", "--strip-all", "-e", "_start", "-z", "max-page-size=4096",
                    str(BUILD / "init.o"), "-o", str(BUILD / "init")], check=True, timeout=30)
    check_elf((BUILD / "init").read_bytes())
    subprocess.run(["/usr/bin/swiftc", "-swift-version", "5", "-O", "-module-cache-path",
                    str(BUILD / "modules"), str(ROOT / "runner.swift"), "-o", str(BUILD / "cocoon-vz"),
                    "-framework", "Virtualization"], check=True, timeout=180)
    subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", "--entitlements",
                    str(ROOT.parent / "macos-preflight/entitlements.plist"), str(BUILD / "cocoon-vz")],
                   check=True, timeout=30)
    report = {"schema": "cocoon-microvm-build-v1", "artifacts": {
        name: {"bytes": (BUILD / name).stat().st_size, "sha256": digest(BUILD / name)}
        for name in ["init", "cocoon-vz"]}, "guest_boot_tested": False}
    (BUILD / "manifest.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report, indent=2))


def doctor():
    binary = BUILD / "cocoon-vz"
    result = subprocess.run([str(binary), "--doctor"], capture_output=True,
                            text=True, timeout=15)
    observed = json.loads(result.stdout)
    if observed.get("schema") != "cocoon-vz-doctor-v1":
        raise ValueError("unexpected diagnostic protocol")
    runs = ROOT / "runs"
    runs.mkdir(exist_ok=True)
    report = {"runner_sha256": digest(binary), "exit_code": result.returncode,
              "report": observed}
    destination = runs / ("doctor-" + uuid.uuid4().hex + ".json")
    destination.write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"result": str(destination), **report}, indent=2))
    return result.returncode


def run_guest(binary, request, seconds, cancel):
    """Own one runner; cancellation is a request, never termination evidence."""
    with subprocess.Popen([str(binary), str(request)], stdout=subprocess.PIPE,
                          stderr=subprocess.PIPE) as process:
        timer = None
        if cancel:
            def signal_runner():
                try:
                    process.send_signal(signal.SIGTERM)
                except ProcessLookupError:
                    pass
            timer = threading.Timer(min(2, seconds / 2), signal_runner)
            timer.start()
        try:
            stdout, stderr = process.communicate(timeout=seconds + 15)
            return subprocess.CompletedProcess(process.args, process.returncode, stdout, stderr)
        except BaseException:
            # A lost/expired harness cannot leave its local runner unbounded.
            # Reaping this process still does not prove its VM stopped.
            process.kill()
            process.communicate()
            raise
        finally:
            if timer:
                timer.cancel()
                timer.join()


def boot(args, emit=True):
    kernel = args.kernel.resolve(strict=True)
    if not kernel.is_file():
        raise ValueError("kernel must be a regular Linux ARM64 Image")
    kernel_check = check_kernel(kernel)
    if kernel_check["compatibility"] == "incompatible":
        raise ValueError(kernel_check["error"])
    run_id = uuid.uuid4().hex
    runs = ROOT / "runs"
    runs.mkdir(exist_ok=True)
    run = runs / run_id
    run.mkdir(mode=0o700)
    init = (BUILD / "init").read_bytes()
    check_elf(init)
    payload = b"hello cocoon\n"
    initrd = run / "initrd.cpio.gz"
    initrd.write_bytes(cpio([
        ("dev", stat.S_IFDIR | 0o755, b"", 0, 0),
        ("dev/console", stat.S_IFCHR | 0o600, b"", 5, 1),
        ("dev/hvc0", stat.S_IFCHR | 0o600, b"", 229, 0),
        ("init", stat.S_IFREG | 0o755, init, 0, 0),
        ("input", stat.S_IFREG | 0o400, payload, 0, 0),
        ("token", stat.S_IFREG | 0o400, run_id.encode(), 0, 0),
        ("mode", stat.S_IFREG | 0o400, {"roundtrip": b"r", "lifecycle": b"l"}.get(args.case, b"h"), 0, 0),
    ]))
    request = {"kernel": str(kernel), "initrd": str(initrd), "token": run_id,
               "memoryMiB": args.memory_mib, "seconds": args.seconds}
    if args.case == "lifecycle":
        request["lifecycle"] = True
    request_path = run / "request.json"
    request_path.write_text(json.dumps(request))
    preparation_start = time.monotonic()
    report = {"schema": "cocoon-microvm-acceptance-v1", "case": args.case,
              "kernel_sha256": kernel_check["kernel_sha256"], "kernel_check": kernel_check,
              "runner_sha256": digest(BUILD / "cocoon-vz"),
              "initrd_sha256": digest(initrd), "initrd_bytes": initrd.stat().st_size,
              "passed": False}
    try:
        report["identity_preparation_ms"] = (time.monotonic() - preparation_start) * 1000
        start = time.monotonic()
        result = run_guest(BUILD / "cocoon-vz", request_path, args.seconds, args.case == "cancel")
        report["process_wall_ms"] = (time.monotonic() - start) * 1000
        report["exit_code"] = result.returncode
        report["runner"] = json.loads(result.stdout)
        (run / "stderr.txt").write_bytes(result.stderr)
        observed = report["runner"]
        text = base64.b64decode(observed.get("serial_base64", ""), validate=True).replace(b"\r\n", b"\n")
        expected = b"COCOON_OUTPUT:" + run_id.encode() + b"\n" + payload.upper() + b"\nCOCOON_DONE:" + run_id.encode() + b"\n"
        report["output_matched"] = expected in text
        common = result.returncode == 0 and observed.get("termination_confirmed") is True and observed.get("guest_ready") is True
        if args.case == "roundtrip":
            report["passed"] = common and report["output_matched"] and observed.get("stop_reason") == "guest_done"
        elif args.case == "lifecycle":
            report["passed"] = (common and observed.get("verified_cycles") == 5
                                and observed.get("stop_reason") == "lifecycle_complete")
        else:
            reason = "signal" if args.case == "cancel" else "deadline"
            report["passed"] = common and observed.get("stop_reason") == reason and observed.get("guest_done") is False
    except subprocess.TimeoutExpired:
        report["error"] = "runner watchdog expired; termination unconfirmed, no automatic replay"
    except (json.JSONDecodeError, ValueError) as error:
        report["error"] = f"invalid runner evidence: {error}"
    destination = run / "result.json"
    destination.write_text(json.dumps(report, indent=2) + "\n")
    if emit:
        print(json.dumps({"result": str(destination), **report}, indent=2))
    return report, destination


def benchmark(args):
    samples = []
    paths = []
    for _ in range(args.samples):
        report, path = boot(args, emit=False)
        samples.append(report)
        paths.append(str(path))
        if not report["passed"]:
            break  # A failed/unknown attempt is never automatically replayed.
    good = [r for r in samples if r["passed"]]
    metrics = {}
    for name in ["guest_ready_ms", "guest_done_ms", "stopped_ms"]:
        values = sorted(r["runner"][name] for r in good if name in r["runner"])
        metrics[name] = {"median": statistics.median(values),
                         "p95": values[math.ceil(len(values) * .95) - 1]} if values else None
    summary = {"schema": "cocoon-microvm-benchmark-v1", "requested_samples": args.samples,
               "attempted_samples": len(samples), "passed_samples": len(good),
               "cache_condition": "Repeated boots; host filesystem caches not flushed. Not a cold-cache benchmark.",
               "metrics": metrics, "sample_results": paths,
               "complete": len(good) == args.samples}
    destination = ROOT / "runs" / ("benchmark-" + uuid.uuid4().hex + ".json")
    destination.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps({"result": str(destination), **summary}, indent=2))
    return 0 if summary["complete"] else 2


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("doctor")
    kernel_parser = commands.add_parser("check-kernel")
    kernel_parser.add_argument("--kernel", required=True, type=pathlib.Path)
    compile_parser = commands.add_parser("build")
    compile_parser.add_argument("--linker")
    for name in ["boot", "benchmark"]:
        launch = commands.add_parser(name)
        launch.add_argument("--kernel", required=True, type=pathlib.Path)
        launch.add_argument("--case", choices=["roundtrip", "deadline", "cancel", "lifecycle"], default="roundtrip")
        launch.add_argument("--memory-mib", type=int, choices=range(128, 4097), metavar="128..4096", default=128)
        launch.add_argument("--seconds", type=int, choices=range(1, 61), metavar="1..60", default=10)
        if name == "benchmark":
            launch.add_argument("--samples", type=int, choices=range(1, 101), metavar="1..100", default=30)
    args = parser.parse_args()
    if args.command == "check-kernel":
        report = check_kernel(args.kernel.resolve(strict=True))
        print(json.dumps(report, indent=2))
        return 2 if report["compatibility"] == "incompatible" else 0
    if args.command == "doctor":
        return doctor()
    if args.command == "build":
        build(args.linker)
        return 0
    if args.command == "benchmark":
        return benchmark(args)
    report, _ = boot(args)
    return 0 if report["passed"] else 2


if __name__ == "__main__":
    raise SystemExit(main())
