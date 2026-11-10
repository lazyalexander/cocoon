#!/usr/bin/env python3
"""Real VM negative acceptance for experimental snapshots; never retries a fault."""
import argparse
import json
import pathlib
import shutil
import uuid
from probe import BUILD, ROOT, digest, run_guest, verify_snapshot


def invoke(binary, request):
    result = run_guest(binary, request, 60, False)
    try:
        report = json.loads(result.stdout)
    except json.JSONDecodeError:
        report = {"status": "invalid_runner_json", "termination_confirmed": False}
    report["exit_code"] = result.returncode
    report["serial"] = report.get("guest_serial", "")
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fixture", required=True, type=pathlib.Path)
    parser.add_argument("--kernel", required=True, type=pathlib.Path)
    parser.add_argument("--case", required=True,
                        choices=["corrupt-state", "wrong-machine-id", "cold-boot", "double-restore"])
    args = parser.parse_args()
    fixture = args.fixture.resolve(strict=True)
    kernel = args.kernel.resolve(strict=True)
    runs = ROOT / "runs"
    runs.mkdir(exist_ok=True)
    work = runs / ("snapshot-fault-" + args.case + "-" + uuid.uuid4().hex)
    work.mkdir(mode=0o700)
    disk = work / "root.raw"
    shutil.copyfile(fixture / "root.raw", disk)
    state = work / "state.vz"
    token = uuid.uuid4().hex
    base = {"kernel": str(kernel), "initrd": str(fixture / "initrd.cpio.gz"),
            "token": token, "memoryMiB": 512, "seconds": 60,
            "rootDisk": str(disk), "diskMode": "snapshot", "payload": token}
    save_request = work / "save.json"
    save_request.write_text(json.dumps({**base, "snapshotMode": "save", "snapshotPath": str(state)}))
    save = invoke(BUILD / "cocoon-vz", save_request)
    passed = False
    detail = {"save_accepted": save.get("snapshot_saved") is True
              and save.get("termination_confirmed") is True and save["exit_code"] == 0}
    if detail["save_accepted"]:
        disk_before = digest(disk)
        if args.case == "corrupt-state":
            content = bytearray(state.read_bytes())
            content[0] ^= 1
            state.write_bytes(content)
        elif args.case == "wrong-machine-id":
            identity = pathlib.Path(str(state) + ".machine-id")
            content = bytearray(identity.read_bytes())
            content[0] ^= 1
            identity.write_bytes(content)
        if args.case == "cold-boot":
            restore_request = work / "cold-boot.json"
            restore_request.write_text(json.dumps(base))
            observed = invoke(BUILD / "cocoon-vz", restore_request)
            errors = verify_snapshot(save, observed, state.stat().st_size, disk_before,
                                     digest(disk), expected_token=token)
            complete_cold_boot = (observed["exit_code"] == 0
                                  and observed.get("termination_confirmed") is True
                                  and observed.get("guest_ready") is True
                                  and observed.get("guest_done") is True
                                  and observed.get("stop_reason") == "guest_done")
            passed = (complete_cold_boot and "framework restore not confirmed" in errors
                      and "restore emitted a cold-boot marker" in errors
                      and "guest RAM nonce mismatch" in errors)
            detail.update(errors=errors, complete_cold_boot=complete_cold_boot, cold_boot=observed)
        else:
            restore_request = work / "restore.json"
            restore_request.write_text(json.dumps({**base, "snapshotMode": "restore", "restorePath": str(state)}))
            observed = invoke(BUILD / "cocoon-vz", restore_request)
            if args.case == "double-restore":
                first_accepted = observed.get("snapshot_restored") is True and observed["exit_code"] == 0
                second = invoke(BUILD / "cocoon-vz", restore_request)
                passed = first_accepted and second["exit_code"] != 0 and second.get("status") == "snapshot_restore_claimed"
                detail.update(first_restore_accepted=first_accepted, second_restore=second)
            else:
                if args.case == "corrupt-state":
                    expected_rejection = (observed.get("status") == "snapshot_restore_failed"
                                          and observed.get("snapshot_restore_claimed") is True)
                else:
                    expected_rejection = observed.get("status") in ["snapshot_setup_failed", "snapshot_restore_failed"]
                passed = (observed["exit_code"] != 0 and expected_rejection
                          and observed.get("snapshot_restored") is not True)
                detail.update(restore=observed)
    detail.update(schema="cocoon-snapshot-fault-v1", case=args.case, passed=passed,
                  production_backend_qualified=False)
    destination = work / "result.json"
    destination.write_text(json.dumps(detail, indent=2) + "\n")
    print(json.dumps({"result": str(destination), **detail}, indent=2))
    return 0 if passed else 2


if __name__ == "__main__":
    raise SystemExit(main())
