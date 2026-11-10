#!/usr/bin/env python3
"""Real host-socket, guest-command, binary-artifact and lifecycle acceptance."""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import time
import uuid
from probe import ROOT, BUILD


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--kernel", required=True, type=pathlib.Path)
    parser.add_argument("--corrupt-before-resume", action="store_true")
    args = parser.parse_args()
    fixture = args.fixture.resolve(strict=True)
    kernel = args.kernel.resolve(strict=True)
    cli = ROOT.parent.parent / "target/release/cocoon"
    runner = BUILD / "cocoon-vz"
    state = ROOT / "runs" / ("production-" + uuid.uuid4().hex)
    state.mkdir(mode=0o700)

    def direct(*arguments):
        result = subprocess.run([str(cli), "host", str(state), *map(str, arguments)],
                                capture_output=True, timeout=120)
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors="replace"))
        return json.loads(result.stdout)

    image = direct("image", "import", kernel, fixture / "initrd.cpio.gz", fixture / "root.raw",
                   "Pinned Debian inputs and Cocoon fixture; acceptance only")["image"]
    vm = str(uuid.uuid4())
    direct("vm", "create", vm, image, "production-acceptance", "retain")
    server = subprocess.Popen([str(cli), "host", str(state), "serve", str(runner)],
                              stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    socket = pathlib.Path("/tmp/cocoon-host-" + hashlib.sha256(os.fsencode(state)).hexdigest() + ".sock")
    for _ in range(500):
        if socket.exists():
            break
        if server.poll() is not None:
            raise RuntimeError(server.stderr.read().decode(errors="replace"))
        time.sleep(0.01)
    else:
        raise TimeoutError("host API socket readiness")
    authority = None

    def api(operation):
        request_path = state / (uuid.uuid4().hex + ".json")
        request_path.write_text(json.dumps({"request": str(uuid.uuid4()), "caller": "agent",
                                            "credential": "", "operation": operation}))
        result = subprocess.run([str(cli), "host", str(state), "api", str(request_path)],
                                capture_output=True, timeout=120)
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors="replace"))
        response = json.loads(result.stdout)
        if "Ok" not in response["result"]:
            raise RuntimeError(response["result"]["Err"])
        return response["result"]["Ok"]

    def identity_for(target, active):
        return {"vm": target, "caller": "agent", "task": "binary-artifact",
                "attempt": active["attempt"]["id"].replace("-", ""),
                "generation": active["generation"], "request": 0}

    def identity():
        return identity_for(vm, authority)

    try:
        authority = api({"operation": "start", "vm": vm})
        expected = bytes((index * 31) % 256 for index in range(12000))
        for offset in range(0, len(expected), 4000):
            chunk = expected[offset:offset + 4000]
            api({"operation": "guest", "request": {"identity": identity(), "operation": {
                "kind": "write_bytes", "path": "/workspace/data.bin", "content_hex": chunk.hex(),
                "offset": offset, "sync": True}}})
        executed = api({"operation": "guest", "request": {"identity": identity(), "operation": {
            "kind": "exec", "argv": ["/bin/sh", "-c", "test -s data.bin && printf BINARY_PASS"],
            "cwd": "/workspace"}}})
        assert executed["result"]["Ok"]["stdout"] == "BINARY_PASS"
        first_generation = authority["generation"]
        first_attempt = authority["attempt"]["id"]
        api({"operation": "suspend", "vm": vm})
        suspended = api({"operation": "show", "vm": vm})
        assert suspended["state"] == "suspended" and suspended["active_lease"] is None
        if args.corrupt_before_resume:
            state_path = pathlib.Path(suspended["suspend_state"])
            content = bytearray(state_path.read_bytes())
            content[0] ^= 1
            state_path.write_bytes(content)
            try:
                api({"operation": "resume", "vm": vm})
                raise AssertionError("corrupt snapshot unexpectedly resumed")
            except RuntimeError:
                pass
            recovery = api({"operation": "show", "vm": vm})
            assert recovery["state"] == "recovery_required" and recovery["active_lease"] is None
            authority = api({"operation": "recover_cold", "vm": vm})
            assert authority["generation"] == first_generation + 2
        else:
            authority = api({"operation": "resume", "vm": vm})
            assert authority["generation"] == first_generation + 1
        assert authority["attempt"]["id"] != first_attempt
        resumed = api({"operation": "guest", "request": {"identity": identity(), "operation": {
            "kind": "exec", "argv": ["/bin/sh", "-c", "test -s data.bin && printf RESUME_PASS"],
            "cwd": "/workspace"}}})
        assert resumed["result"]["Ok"]["stdout"] == "RESUME_PASS"
        artifact = api({"operation": "export", "request": {"identity": identity(), "operation": {
            "kind": "read", "path": "/workspace/data.bin"}},
            "guest_path": "/workspace/data.bin", "name": "artifact.bin"})
        exported = (state / "vms" / vm / authority["attempt"]["id"] / "exports" / "artifact.bin").read_bytes()
        assert exported == expected
        assert artifact["sha256"] == hashlib.sha256(expected).hexdigest()
        api({"operation": "stop", "vm": vm})
        stopped = api({"operation": "show", "vm": vm})
        assert stopped["state"] == "stopped" and stopped["active_lease"] is None
        candidate = api({"operation": "save_candidate", "vm": vm})
        api({"operation": "promote_candidate", "image": candidate})
        branch = str(uuid.uuid4())
        api({"operation": "branch", "source": candidate, "vm": branch, "owner": "production-acceptance"})
        branch_authority = api({"operation": "start", "vm": branch})
        branch_result = api({"operation": "guest", "request": {
            "identity": identity_for(branch, branch_authority),
            "operation": {"kind": "exec", "argv": ["/bin/sh", "-c",
                "test -s data.bin && printf BRANCH_PASS"], "cwd": "/workspace"}}})
        assert branch_result["result"]["Ok"]["stdout"] == "BRANCH_PASS"
        api({"operation": "stop", "vm": branch})
        api({"operation": "rollback", "vm": vm, "baseline": image})
        summary = {"schema": "cocoon-production-acceptance-v1", "passed": True,
                   "binary_bytes": len(expected), "artifact_sha256": artifact["sha256"],
                   "generation": authority["generation"], "resumed": True,
                   "cold_recovery": args.corrupt_before_resume, "branch": True,
                   "rollback": True, "state": stopped["state"]}
    except BaseException as error:
        try:
            if authority is not None:
                current = api({"operation": "show", "vm": vm})
                if current["state"] == "running":
                    api({"operation": "stop", "vm": vm})
        except BaseException:
            pass
        (state / "failure.txt").write_text(str(error) + "\n")
        raise
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
    destination = state / "summary.json"
    destination.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
