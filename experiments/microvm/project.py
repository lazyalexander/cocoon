#!/usr/bin/env python3
"""Exercise real offline code editing and artifact reception; never retries failures."""
import argparse
import json
import pathlib
import subprocess
import uuid
from probe import ROOT, BUILD, digest


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--kernel", required=True, type=pathlib.Path)
    parser.add_argument("--rounds", type=int, default=3, choices=range(1, 11))
    args = parser.parse_args()
    fixture = args.fixture.resolve(strict=True)
    kernel = args.kernel.resolve(strict=True)
    manifest = json.loads((fixture / "manifest.json").read_text())
    if not manifest.get("project_userland"):
        raise ValueError("build a --project fixture first")
    for filename, key in [("root.raw", "root_sha256"), ("initrd.cpio.gz", "initrd_sha256")]:
        if digest(fixture / filename) != manifest[key]:
            raise ValueError("fixture digest mismatch")
    state = ROOT / "runs" / ("project-" + uuid.uuid4().hex)
    cli = ROOT.parent.parent / "target/release/cocoon"
    runner = BUILD / "cocoon-vz"

    def call(*arguments):
        result = subprocess.run([str(cli), "host", str(state), *map(str, arguments)], capture_output=True, timeout=180)
        if result.returncode:
            raise RuntimeError("project failed; inspect private state; no replay: " + result.stderr.decode(errors="replace"))
        return json.loads(result.stdout)

    image = call("image", "import", kernel, fixture / "initrd.cpio.gz", fixture / "root.raw",
                 "Pinned Debian kernel/modules GPL-2.0; BusyBox GPL-2.0; glibc LGPL-2.1-or-later; Cocoon MIT; offline experimental shell fixture")["image"]
    vm = str(uuid.uuid4())
    call("vm", "create", vm, image, "local-project-acceptance", "retain")
    baseline = state / "images" / image / "root.raw"
    original = digest(baseline)
    reports = []
    for cycle in range(args.rounds):
        report = call("experiment", "project", vm, runner)
        artifact = state / "vms" / vm / report["boot"] / "artifact.txt"
        if not report["passed"] or artifact.read_bytes() != b"Built for Cocoon\n":
            raise RuntimeError("artifact acceptance failed")
        if digest(artifact) != report["artifact"]["sha256"]:
            raise RuntimeError("artifact digest mismatch")
        if call("vm", "show", vm)["state"] != "stopped":
            raise RuntimeError("VM not confirmed stopped")
        reports.append(report)
        print(json.dumps({"cycle": cycle + 1, "passed": True, "termination_confirmed": report["termination_confirmed"], "artifact": report["artifact"]}), flush=True)
    if digest(baseline) != original:
        raise RuntimeError("baseline changed")
    summary = {"schema": "cocoon-project-summary-v1", "boots": len(reports),
               "passed": True, "baseline_unchanged": True, "host_sha256": digest(cli),
               "runner_sha256": digest(runner), "fixture_manifest_sha256": digest(fixture / "manifest.json"),
               "production_backend_qualified": False}
    (state / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
