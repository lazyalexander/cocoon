#!/usr/bin/env python3
"""Run a small headless project through Cocoon's scoped work API.

An operator must start the VM and hand this process its execution authority and
work-grant token. The grant needs guest, export and inspect actions. This example cannot
create, start, register, or reclaim VMs.
Every side effect gets a fresh request ID; uncertain outcomes are reported for
reconciliation, never replayed with a new ID.

Example:
  python3 examples/agent_project.py --state /absolute/private/state \
    --caller agent --grant-file /private/grant --authority-file /private/start.json \
    --project /private/project \
    --command-json '["/bin/sh","src/check.sh"]' \
    --command-json '["/bin/sh","src/build.sh"]' \
    --artifact result.txt --output-name result.txt

After an uncertain response, query its original ID:
  python3 examples/agent_project.py --state /absolute/private/state \
    --caller agent --grant-file /private/grant --authority-file /private/start.json \
    --status-id OPERATION_UUID
"""
import argparse
import hashlib
import json
import os
import pathlib
import socket
import stat
import struct
import sys
import uuid

MAX_FRAME = 32768
CHUNK = 4000
MAX_FILE = 16 * 1024 * 1024
MAX_FILES = 64
MAX_IMPORT = 32 * 1024 * 1024


class OutcomeUnknown(Exception):
    def __init__(self, request_id, cause):
        super().__init__(f"operation {request_id}: outcome unknown ({cause}); query operation_status")
        self.request_id = request_id


def private_file(path, limit):
    path = pathlib.Path(path)
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    info = os.fstat(descriptor)
    if not stat.S_ISREG(info.st_mode) or info.st_mode & 0o077:
        os.close(descriptor)
        raise ValueError(f"{path}: expected a private regular file (mode 0600)")
    if info.st_size > limit:
        os.close(descriptor)
        raise ValueError(f"{path}: file exceeds limit")
    return os.fdopen(descriptor, "rb")


def read_private_text(path, limit):
    with private_file(path, limit) as stream:
        data = stream.read(limit + 1)
        if len(data) > limit:
            raise ValueError(f"{path}: file grew beyond limit")
        return data.decode().strip()


def read_exact(stream, size):
    data = bytearray()
    while len(data) < size:
        part = stream.recv(size - len(data))
        if not part:
            raise ConnectionError("socket closed before complete response")
        data.extend(part)
    return bytes(data)


class WorkClient:
    def __init__(self, state, caller, token, authority):
        state = pathlib.Path(state)
        if not state.is_absolute():
            raise ValueError("state path must be absolute")
        if len(token) != 64 or any(byte not in "0123456789abcdefABCDEF" for byte in token):
            raise ValueError("invalid work grant token")
        self.socket_path = "/tmp/cocoon-host-" + hashlib.sha256(os.fsencode(state)).hexdigest() + ".sock"
        self.caller = caller
        self.token = token
        self.vm = authority["attempt"]["vm"]
        self.attempt = authority["attempt"]["id"].replace("-", "")
        self.generation = authority["generation"]
        self.task = str(uuid.uuid4())

    def identity(self):
        return {"vm": self.vm, "caller": self.caller, "task": self.task,
                "attempt": self.attempt, "generation": self.generation, "request": 0}

    def context(self):
        return self.call({"operation": "context", "vm": self.vm})

    def call(self, operation):
        request_id = str(uuid.uuid4())
        request = {"request": request_id, "caller": self.caller, "credential": "",
                   "grant": self.token, "operation": operation}
        body = json.dumps(request, separators=(",", ":")).encode()
        if len(body) > MAX_FRAME:
            raise ValueError("request exceeds the 32 KiB transport limit")
        # A transport error after send is never a reason to submit a new ID.
        sent = False
        try:
            with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as stream:
                stream.settimeout(60)
                stream.connect(self.socket_path)
                sent = True
                stream.sendall(struct.pack(">I", len(body)) + body)
                count = struct.unpack(">I", read_exact(stream, 4))[0]
                if count < 1 or count > MAX_FRAME:
                    raise ValueError("invalid response frame length")
                response = json.loads(read_exact(stream, count))
        except (OSError, ValueError, json.JSONDecodeError) as error:
            if sent:
                raise OutcomeUnknown(request_id, error) from error
            raise
        if response.get("request") != request_id or response.get("caller") != self.caller:
            raise OutcomeUnknown(request_id, "response identity mismatch")
        result = response["result"]
        if "Err" in result:
            raise RuntimeError(f"operation {request_id}: {result['Err']}")
        return result["Ok"]

    def guest(self, command):
        response = self.call({"operation": "guest", "request": {
            "identity": self.identity(), "operation": command}})
        result = response["result"]
        if "Err" in result:
            raise RuntimeError(f"guest rejected operation: {result['Err']}")
        return result["Ok"]

    def import_file(self, source, guest_name=None):
        source = pathlib.Path(source)
        guest_name = guest_name or source.name
        parts = pathlib.PurePosixPath(guest_name).parts
        if not parts or any(part in {"", ".", ".."} for part in parts) or guest_name.startswith("/"):
            raise ValueError("invalid input filename")
        destination = "/workspace/" + "/".join(parts)
        if len(parts) > 1:
            parent = "/workspace/" + "/".join(parts[:-1])
            made = self.run(["/bin/mkdir", "-p", parent])
            if made.get("code") != 0 or made.get("timed_out"):
                raise RuntimeError("guest directory creation failed")
        offset = 0
        digest = hashlib.sha256()
        descriptor = os.open(source, os.O_RDONLY | os.O_NOFOLLOW)
        with os.fdopen(descriptor, "rb") as stream:
            info = os.fstat(stream.fileno())
            if not stat.S_ISREG(info.st_mode) or info.st_size > MAX_FILE:
                raise ValueError(f"{source}: expected a regular file of at most {MAX_FILE} bytes")
            while chunk := stream.read(CHUNK):
                if offset + len(chunk) > MAX_FILE:
                    raise ValueError(f"{source}: file grew beyond import limit")
                result = self.guest({"kind": "write_bytes", "path": destination,
                                     "content_hex": chunk.hex(), "offset": offset, "sync": True})
                if result.get("kind") != "written":
                    raise RuntimeError("guest did not confirm chunk write")
                digest.update(chunk)
                offset += len(chunk)
        if offset == 0:
            self.guest({"kind": "write_bytes", "path": destination,
                        "content_hex": "", "offset": 0, "sync": True})
        return {"guest_path": destination, "bytes": offset, "sha256": digest.hexdigest()}

    def import_project(self, root):
        root = pathlib.Path(root)
        if root.is_symlink() or not root.is_dir():
            raise ValueError("project must be a real directory")
        files = []
        total = 0
        for parent, dirs, names in os.walk(root, followlinks=False):
            dirs.sort()
            names.sort()
            for name in [*dirs, *names]:
                if (pathlib.Path(parent) / name).is_symlink():
                    raise ValueError("project symlinks are unsupported")
            for name in names:
                path = pathlib.Path(parent) / name
                info = path.stat(follow_symlinks=False)
                if not stat.S_ISREG(info.st_mode):
                    raise ValueError("project contains a non-regular file")
                total += info.st_size
                files.append((path, path.relative_to(root).as_posix()))
                if len(files) > MAX_FILES or total > MAX_IMPORT or info.st_size > MAX_FILE:
                    raise ValueError("project exceeds import limits")
        if not files:
            raise ValueError("project has no files")
        return [self.import_file(path, relative) for path, relative in files]

    def run(self, argv):
        result = self.guest({"kind": "exec", "argv": argv, "cwd": "/workspace"})
        if result.get("kind") != "exited":
            raise RuntimeError("guest did not return an exit result")
        return result

    def export(self, guest_name, output_name):
        if not guest_name or "/" in guest_name or guest_name in {".", ".."}:
            raise ValueError("artifact must be a filename under /workspace")
        if not output_name or "/" in output_name or output_name in {".", ".."}:
            raise ValueError("output name must be one filename")
        return self.call({"operation": "export", "request": {"identity": self.identity(),
            "operation": {"kind": "read", "path": "/workspace/" + guest_name}},
            "guest_path": "/workspace/" + guest_name, "name": output_name})


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--state", required=True, help="Absolute Cocoon state path; used to locate the socket")
    parser.add_argument("--caller", required=True, help="Subject bound to the work grant")
    parser.add_argument("--grant-file", required=True, help="Private file containing the one-time grant token")
    parser.add_argument("--authority-file", required=True, help="Private JSON response from VM start")
    parser.add_argument("--input", action="append", help="File imported into /workspace; repeat as needed")
    parser.add_argument("--project", help="Directory recursively imported into /workspace")
    parser.add_argument("--command-json", action="append", help='argv JSON; repeat for test then build')
    parser.add_argument("--artifact", help="Artifact filename inside /workspace")
    parser.add_argument("--output-name", help="Host-chosen export filename")
    parser.add_argument("--status-id", help="Inspect an original operation ID; never resubmit it")
    args = parser.parse_args()
    authority = json.loads(read_private_text(args.authority_file, 8192))
    if "result" in authority:
        authority = authority["result"]["Ok"]
    client = WorkClient(args.state, args.caller, read_private_text(args.grant_file, 128), authority)
    if args.status_id:
        if any([args.input, args.project, args.command_json, args.artifact, args.output_name]):
            parser.error("--status-id cannot be combined with a new project run")
        print(json.dumps(client.call({"operation": "operation_status",
                                      "id": str(uuid.UUID(args.status_id))}), indent=2))
        return
    if not (args.input or args.project) or not all([args.command_json, args.artifact, args.output_name]):
        parser.error("run requires --project or --input, command, artifact and output name")
    context = client.context()
    if context.get("schema") != "cocoon-agent-context-v1" or not context["work"]["guest_ready"]:
        raise RuntimeError("VM is not ready for this work grant")
    commands = [json.loads(command) for command in args.command_json]
    if any(not isinstance(argv, list) or not argv or not all(isinstance(s, str) for s in argv)
           or not argv[0].startswith("/") for argv in commands):
        raise ValueError("each command must be a JSON array with an absolute executable")
    names = [pathlib.Path(path).name for path in args.input or []]
    if len(names) != len(set(names)):
        raise ValueError("input filenames must be unique")
    if args.project and set(names) & {entry.name for entry in os.scandir(args.project)}:
        raise ValueError("input filenames must not replace project root entries")
    inputs = client.import_project(args.project) if args.project else []
    inputs += [client.import_file(path) for path in args.input or []]
    outcomes = []
    for argv in commands:
        outcome = client.run(argv)
        if outcome.get("code") != 0 or outcome.get("timed_out"):
            raise RuntimeError("guest command failed; inspect the original operation before retrying")
        outcomes.append({"code": outcome["code"], "timed_out": outcome["timed_out"]})
    artifact = client.export(args.artifact, args.output_name)
    print(json.dumps({"vm": client.vm, "attempt": client.attempt, "inputs": inputs,
                      "commands": outcomes,
                      "artifact": artifact}, indent=2))


if __name__ == "__main__":
    try:
        main()
    except OutcomeUnknown as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(3)
