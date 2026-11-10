#!/usr/bin/env python3
"""Real fixed-root acceptance through the Rust supervisor; stops on first failure."""
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
    parser.add_argument("--rounds", type=int, default=10, choices=range(1, 11))
    args = parser.parse_args()
    fixture = args.fixture.resolve(strict=True)
    kernel = args.kernel.resolve(strict=True)
    state = ROOT / "runs" / ("persistence-" + uuid.uuid4().hex)
    cli = ROOT.parent.parent / "target/release/cocoon"
    runner = BUILD / "cocoon-vz"
    reports = []

    def call(*arguments):
        result = subprocess.run([str(cli), "host", str(state), *map(str, arguments)], capture_output=True, timeout=180)
        if result.returncode:
            raise RuntimeError("host operation failed; inspect private state; no automatic retry: " + result.stderr.decode(errors="replace"))
        return json.loads(result.stdout)

    image = call("image", "import", kernel, fixture / "initrd.cpio.gz", fixture / "root.raw",
                 "Debian pinned Linux kernel (GPL-2.0); Cocoon Rust root fixture (MIT); experimental, not a Debian userland")["image"]
    vm = str(uuid.uuid4())
    call("vm", "create", vm, image, "local-persistence-acceptance", "retain")
    baseline = state / "images" / image / "root.raw"
    original_hash = digest(baseline)
    for cycle in range(args.rounds):
        payload = uuid.uuid4().hex
        for mode in ["write", "read"]:
            report = call("experiment", "persistence", vm, runner, mode, payload)
            reports.append(report)
            print(json.dumps({"cycle":cycle + 1,"mode":mode,"passed":report["passed"],
                              "guest_ready_ms":report["runner"].get("guest_ready_ms")}), flush=True)
            if not report["passed"]:
                raise RuntimeError("acceptance failed; do not replay an unknown boot")
    unchanged = original_hash == digest(baseline)
    if not unchanged:
        raise RuntimeError("baseline mutated")
    summary = {"schema":"cocoon-persistence-summary-v1", "rounds":args.rounds,
               "boots":len(reports), "host_sha256":digest(cli), "fixture_manifest_sha256":digest(fixture / "manifest.json"), "baseline_unchanged":unchanged,
               "unique_boots":len({r["boot"] for r in reports}) == len(reports),
               "passed":all(r["passed"] for r in reports), "production_backend_qualified":False}
    (state / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
