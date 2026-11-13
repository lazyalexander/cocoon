#!/usr/bin/env python3
"""Run real create/execute/stop/reclaim cycles; raw state stays private."""
import argparse
import hashlib
import json
import pathlib
import subprocess
import sys
import time
import uuid

ROOT = pathlib.Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "examples"))
from agent_project import WorkClient


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--kernel", required=True, type=pathlib.Path)
    parser.add_argument("--cycles", type=int, default=100)
    args = parser.parse_args()
    if args.cycles < 1:
        parser.error("--cycles must be positive")
    state = ROOT / "experiments/microvm/runs" / ("reclaim-" + uuid.uuid4().hex)
    state.mkdir(mode=0o700)
    cli = ROOT / "target/release/cocoon"
    runner = ROOT / "target/cocoon-vz/cocoon-vz"
    fixture = args.fixture.resolve(strict=True)
    kernel = args.kernel.resolve(strict=True)

    def direct(*items):
        result = subprocess.run([str(cli), "host", str(state), *map(str, items)],
                                capture_output=True, timeout=120)
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors="replace"))
        return json.loads(result.stdout)

    image = direct("image", "import", kernel, fixture / "initrd.cpio.gz",
                   fixture / "root.raw", "Pinned Debian inputs; reclaim cycles")["image"]
    server = subprocess.Popen([str(cli), "host", str(state), "serve", str(runner)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    socket = pathlib.Path("/tmp/cocoon-host-" + hashlib.sha256(str(state).encode()).hexdigest() + ".sock")
    completed = 0
    current_vm = None
    tokens = {}

    def api(operation):
        target = operation.get("vm")
        grant = tokens.get(target, "") if operation["operation"] in {"show", "stop"} else ""
        request = state / (uuid.uuid4().hex + ".json")
        request.write_text(json.dumps({"request": str(uuid.uuid4()), "caller": "agent" if grant else "operator",
                                       "credential": "", "grant": grant, "operation": operation}))
        response = direct("api", request)
        if "Err" in response["result"]:
            raise RuntimeError(response["result"]["Err"])
        return response["result"]["Ok"]

    try:
        for _ in range(500):
            if socket.exists():
                break
            if server.poll() is not None:
                raise RuntimeError("host API server failed before VM startup")
            time.sleep(0.01)
        else:
            raise TimeoutError("host API socket readiness")
        for _ in range(args.cycles):
            vm = str(uuid.uuid4())
            current_vm = vm
            direct("vm", "create", vm, image, "reclaim-cycles", "retain")
            authority = api({"operation": "start", "vm": vm})
            issued = api({"operation": "issue_grant", "subject": "agent", "vm": vm,
                          "actions": ["inspect", "guest", "stop"],
                          "expires_unix": int(time.time()) + 3600,
                          "max_export_bytes": 1})
            tokens[vm] = issued["token"]
            client = WorkClient(state, "agent", issued["token"], authority)
            if not client.context()["work"]["guest_ready"]:
                raise RuntimeError("guest was not ready")
            result = client.run(["/bin/sh", "-c", "true"])
            if result["code"] != 0 or result["timed_out"]:
                raise RuntimeError("guest command failed")
            api({"operation": "stop", "vm": vm})
            stopped = api({"operation": "show", "vm": vm})
            if stopped["state"] != "stopped" or stopped["active_lease"] is not None:
                raise RuntimeError("stop lacks confirmation")
            api({"operation": "reclaim", "vm": vm})
            if (state / "vms" / vm).exists() or api({"operation": "reclaim_status", "vm": vm}) != "complete":
                raise RuntimeError("reclaim did not finish")
            completed += 1
            current_vm = None
            if completed % 10 == 0:
                print(f"completed {completed}/{args.cycles} confirmed cycles", flush=True)
        if direct("vm", "list") or list((state / "vms").iterdir()):
            raise RuntimeError("owned VM or working directory survived")
        summary = {"schema": "cocoon-reclaim-cycles-v1", "passed": True,
                   "cycles": completed, "registered_vms": 0, "working_directories": 0}
        (state / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
        print(json.dumps(summary, indent=2))
    except BaseException as error:
        cleanup = None
        if current_vm is not None:
            try:
                current = api({"operation": "show", "vm": current_vm}) if current_vm in tokens else None
                if current is None or current["state"] in {"starting", "running", "stopping", "unknown"}:
                    api({"operation": "emergency_stop", "vm": current_vm})
            except BaseException as cleanup_error:
                cleanup = str(cleanup_error)
        (state / "failure.txt").write_text(str(error) + "\n" + (cleanup or "") + "\n")
        raise
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
        socket.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
