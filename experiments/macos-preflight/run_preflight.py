#!/usr/bin/env python3
"""Build and run the real Apple framework preflight; exit 2 means blocked."""
import argparse
import json
import pathlib
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--kernel", type=pathlib.Path, required=True,
                        help="Existing ARM64 Linux Image, opened for configuration only")
    args = parser.parse_args()
    kernel = args.kernel.resolve(strict=True)
    if not kernel.is_file():
        parser.error("kernel must be a regular file")
    root = pathlib.Path(__file__).resolve().parent
    with tempfile.TemporaryDirectory(prefix="cocoon-vm-preflight-") as tmp:
        binary = pathlib.Path(tmp) / "preflight"
        subprocess.run(["/usr/bin/swiftc", "-module-cache-path", str(pathlib.Path(tmp) / "modules"),
                        str(root / "preflight.swift"), "-o", str(binary),
                        "-framework", "Virtualization", "-framework", "Hypervisor"],
                       check=True, timeout=180)
        subprocess.run(["/usr/bin/codesign", "--force", "--sign", "-", "--entitlements",
                        str(root / "entitlements.plist"), str(binary)], check=True, timeout=30)
        result = subprocess.run([str(binary), str(kernel)], capture_output=True,
                                text=True, timeout=30)
        if result.stderr:
            print(result.stderr, end="")
        report = json.loads(result.stdout)
        report["preflight_exit_code"] = result.returncode
        report["stage_a_complete"] = False
        report["stage_a_note"] = "Preflight only; no Linux guest was booted."
        destination = root / "preflight-result.json"
        destination.write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report, indent=2))
        return result.returncode


if __name__ == "__main__":
    raise SystemExit(main())
