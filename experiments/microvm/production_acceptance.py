#!/usr/bin/env python3
"""Real host-socket, guest-command, binary-artifact and lifecycle acceptance."""
import argparse
import hashlib
import json
import os
import pathlib
import subprocess
import sys
import time
import uuid
from probe import ROOT

sys.path.insert(0, str(ROOT.parent.parent / "examples"))
from agent_project import WorkClient


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--kernel", required=True, type=pathlib.Path)
    parser.add_argument("--corrupt-before-resume", action="store_true")
    parser.add_argument("--automatic-suspend", action="store_true")
    args = parser.parse_args()
    fixture = args.fixture.resolve(strict=True)
    kernel = args.kernel.resolve(strict=True)
    cli = ROOT.parent.parent / "target/release/cocoon"
    runner = ROOT.parent.parent / "target/cocoon-vz/cocoon-vz"
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
    try:
        for _ in range(500):
            if socket.exists():
                break
            if server.poll() is not None:
                raise RuntimeError(server.stderr.read().decode(errors="replace"))
            time.sleep(0.01)
        else:
            raise TimeoutError("host API socket readiness")
    except BaseException:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
        socket.unlink(missing_ok=True)
        raise
    authority = None
    branch = None

    tokens = {}

    def api(operation):
        grant = ""
        if operation.get("operation") in {"guest", "show", "export", "stop"}:
            target = operation.get("vm") or operation["request"]["identity"]["vm"]
            grant = tokens[target]
        request_path = state / (uuid.uuid4().hex + ".json")
        request_path.write_text(json.dumps({"request": str(uuid.uuid4()), "caller": "agent",
                                            "credential": "", "grant": grant, "operation": operation}))
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

    def refresh(target):
        issued = api({"operation": "issue_grant", "subject": "agent", "vm": target,
                      "actions": ["inspect", "guest", "export", "stop"],
                      "expires_unix": int(time.time()) + 3600, "max_export_bytes": 1048576})
        tokens[target] = issued["token"]

    def guest_status():
        response = api({"operation": "guest", "request": {"identity": identity(),
            "operation": {"kind": "status"}}})
        return response["result"]["Ok"]["nonce"]

    def runner_events(active):
        path = state / "vms" / vm / active["attempt"]["id"] / "events.jsonl"
        return [json.loads(line)["event"] for line in path.read_text().splitlines()]

    try:
        authority = api({"operation": "start", "vm": vm})
        refresh(vm)
        project = state / "project"
        (project / "knowledge").mkdir(parents=True)
        (project / "src").mkdir()
        (project / "knowledge" / "recipe.txt").write_text("Cocoon project\n")
        script = project / "src" / "build.sh"
        script.write_text("exit 1\n")
        primary = WorkClient(state, "agent", tokens[vm], authority)
        assert primary.context()["work"]["guest_ready"]
        imported = primary.import_project(project)
        assert len(imported) == 2
        script.write_text("set -eu\ncat knowledge/recipe.txt > result.txt\n")
        primary.import_file(script, "src/build.sh")
        checked = primary.run(["/bin/sh", "-c", "test -s knowledge/recipe.txt && test -s src/build.sh"])
        built = primary.run(["/bin/sh", "src/build.sh"])
        assert checked["code"] == 0 and built["code"] == 0
        project_artifact = primary.export("result.txt", "project-result.txt")
        project_bytes = (state / "vms" / vm / authority["attempt"]["id"] / "exports" / "project-result.txt").read_bytes()
        assert project_bytes == b"Cocoon project\n"
        assert project_artifact["sha256"] == hashlib.sha256(project_bytes).hexdigest()
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
        original_nonce = guest_status()
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
            refresh(vm)
            assert authority["generation"] == first_generation + 2
        else:
            authority = api({"operation": "resume", "vm": vm})
            refresh(vm)
            assert authority["generation"] == first_generation + 1
        assert authority["attempt"]["id"] != first_attempt
        recovered_nonce = guest_status()
        events = runner_events(authority)
        if args.corrupt_before_resume:
            assert recovered_nonce != original_nonce and "vm_resumed" not in events
        else:
            assert recovered_nonce == original_nonce and "vm_resumed" in events
        resumed = api({"operation": "guest", "request": {"identity": identity(), "operation": {
            "kind": "exec", "argv": ["/bin/sh", "-c", "test -s data.bin && printf RESUME_PASS"],
            "cwd": "/workspace"}}})
        assert resumed["result"]["Ok"]["stdout"] == "RESUME_PASS"
        if args.automatic_suspend:
            automatic_nonce = guest_status()
            api({"operation": "set_protection", "policy": {"enabled": True,
                "minimum_free_percent": 100, "emergency_stop": True, "max_suspend_per_pass": 1}})
            for _ in range(300):
                current = api({"operation": "show", "vm": vm})
                if current["state"] == "suspended":
                    break
                time.sleep(0.05)
            else:
                raise TimeoutError("automatic suspension")
            api({"operation": "set_protection", "policy": {"enabled": True,
                "minimum_free_percent": 1, "emergency_stop": True, "max_suspend_per_pass": 1}})
            authority = api({"operation": "resume", "vm": vm})
            refresh(vm)
            assert guest_status() == automatic_nonce and "vm_resumed" in runner_events(authority)
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
        refresh(branch)
        branch_result = api({"operation": "guest", "request": {
            "identity": identity_for(branch, branch_authority),
            "operation": {"kind": "exec", "argv": ["/bin/sh", "-c",
                "test -s data.bin && printf BRANCH_PASS"], "cwd": "/workspace"}}})
        assert branch_result["result"]["Ok"]["stdout"] == "BRANCH_PASS"
        api({"operation": "stop", "vm": branch})
        api({"operation": "rollback", "vm": vm, "baseline": image})
        rollback_authority = api({"operation": "start", "vm": vm})
        refresh(vm)
        rolled_back = api({"operation": "guest", "request": {"identity": identity_for(vm, rollback_authority),
            "operation": {"kind": "exec", "argv": ["/bin/sh", "-c",
                "test ! -e data.bin && printf ROLLBACK_PASS"], "cwd": "/workspace"}}})
        assert rolled_back["result"]["Ok"]["stdout"] == "ROLLBACK_PASS"
        api({"operation": "stop", "vm": vm})
        stopped = api({"operation": "show", "vm": vm})
        assert stopped["state"] == "stopped" and stopped["active_lease"] is None
        api({"operation": "reclaim", "vm": branch})
        api({"operation": "reclaim", "vm": vm})
        archived = state / "artifacts" / vm / first_attempt / "project-result.txt"
        assert archived.read_bytes() == project_bytes
        assert not (state / "vms" / vm).exists()
        assert not (state / "vms" / branch).exists()
        summary = {"schema": "cocoon-production-acceptance-v1", "passed": True,
                   "primary_agent_project": True, "project_artifact_sha256": project_artifact["sha256"],
                   "binary_bytes": len(expected), "artifact_sha256": artifact["sha256"],
                   "generation": rollback_authority["generation"],
                   "snapshot_resume": not args.corrupt_before_resume or args.automatic_suspend,
                   "cold_recovery": args.corrupt_before_resume, "automatic_suspend": args.automatic_suspend,
                   "branch": True, "rollback_verified": True, "reclaimed": True,
                   "state": stopped["state"]}
    except BaseException as error:
        cleanup_errors = []
        for target in {vm, branch} - {None}:
            try:
                current = api({"operation": "show", "vm": target})
                if current["state"] not in ["prepared", "stopped", "suspended", "recovery_required"]:
                    api({"operation": "emergency_stop", "vm": target})
            except BaseException:
                try:
                    api({"operation": "emergency_stop", "vm": target})
                except BaseException as cleanup_error:
                    cleanup_errors.append(str(cleanup_error))
        (state / "failure.txt").write_text(str(error) + "\n" + "\n".join(cleanup_errors) + "\n")
        raise
    finally:
        server.terminate()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            server.kill()
            server.wait()
        socket.unlink(missing_ok=True)
    destination = state / "summary.json"
    destination.write_text(json.dumps(summary, indent=2) + "\n")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
