#!/usr/bin/env python3
"""Exercise original image IDs across process loss/cancel/reopen; fixtures do not boot."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import time
import uuid


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for data in iter(lambda: stream.read(4 * 1024 * 1024), b''):
            digest.update(data)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cocoon', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--repetitions', type=int, choices=range(1, 11), default=10)
    args = parser.parse_args(); cli = args.cocoon.resolve(strict=True)
    root = args.evidence.resolve(); root.mkdir(mode=0o700); state = root / 'state'
    records = []; children = []
    def call(*arguments):
        response = subprocess.run([str(cli), 'host', str(state), *map(str, arguments)], capture_output=True, timeout=120)
        assert response.returncode == 0, response.stderr.decode(errors='replace')
        return json.loads(response.stdout)
    def private(path, value):
        with path.open('x') as stream:
            os.chmod(path, 0o600); json.dump(value, stream); stream.flush(); os.fsync(stream.fileno())
    call('image-init'); credential = (state / 'api-credential').read_text()
    tiny = root / 'boot-fixture'; tiny.write_bytes(b'nonbootable fixture')
    disk = root / 'source.raw'
    with disk.open('xb') as stream:
        stream.truncate(1024 * 1024 * 1024)
    disk_sha = sha(disk); tiny_sha = sha(tiny)
    base = call('image', 'import', tiny, tiny, disk, 'Nonbootable test fixture')['image']
    baseline = call('image-info', base)
    def request(action):
        identity = str(uuid.uuid4()); path = root / (identity + '.json'); query = root / (identity + '-query.json')
        private(path, {'request': identity, 'caller': 'image-faults', 'credential': credential, 'action': action})
        private(query, {'request': identity, 'caller': 'image-faults', 'credential': credential})
        return identity, path, query
    def candidate():
        _, path, _ = request({'operation': 'import_candidate', 'environment': 'a' * 64, 'kernel': str(tiny), 'initrd': str(tiny), 'disk': str(disk), 'kernel_sha256': tiny_sha, 'initrd_sha256': tiny_sha, 'disk_sha256': disk_sha})
        result = call('image-request', path); assert result['state'] == 'succeeded', result
        return result['result']
    def prepared():
        vm = str(uuid.uuid4()); call('vm', 'create', vm, base, 'fixture', 'retain'); return vm
    def action(kind):
        if kind == 'promote_candidate':
            image = candidate(); return {'operation': kind, 'image': image['image'], 'sha256': image['sha256']}
        if kind == 'branch':
            return {'operation': kind, 'image': base, 'sha256': baseline['sha256'], 'vm': str(uuid.uuid4()), 'owner': 'new-fixture-task'}
        vm = prepared()
        if kind == 'save_candidate':
            return {'operation': kind, 'vm': vm, 'generation': 0, 'disk_sha256': disk_sha}
        target = state / 'vms' / vm / 'root.raw'
        with target.open('r+b') as stream:
            stream.write(b'changed working state'); stream.flush(); os.fsync(stream.fileno())
        return {'operation': kind, 'vm': vm, 'generation': 0, 'disk_sha256': sha(target), 'image': base, 'sha256': baseline['sha256']}
    def unpublished(identity, selected):
        with sqlite3.connect(state / 'state.sqlite3') as db:
            if selected['operation'] == 'save_candidate':
                assert db.execute('SELECT count(*) FROM images WHERE id=?', (identity,)).fetchone()[0] == 0
            if selected['operation'] == 'branch':
                assert db.execute('SELECT count(*) FROM vms WHERE id=?', (selected['vm'],)).fetchone()[0] == 0
        if selected['operation'] == 'promote_candidate':
            assert call('image-info', selected['image'])['approved'] is False
        if selected['operation'] == 'rollback':
            assert sha(state / 'vms' / selected['vm'] / 'root.raw') == selected['disk_sha256']
    try:
        for kind in ('save_candidate', 'promote_candidate', 'branch', 'rollback'):
            for boundary in ('prepublication_exit', 'lost_reply', 'cancel_reconnect', 'late_cancel'):
                for repeat in range(args.repetitions):
                    selected = action(kind); identity, path, query = request(selected)
                    log = (root / (identity + '.log')).open('wb')
                    process = subprocess.Popen([str(cli), 'host', str(state), 'image-request', str(path)], stdout=log, stderr=log); children.append(process)
                    if boundary in ('prepublication_exit', 'cancel_reconnect'):
                        deadline = time.monotonic() + 10
                        while True:
                            with sqlite3.connect(state / 'state.sqlite3') as db:
                                row = db.execute('SELECT state FROM image_operations WHERE id=?', (identity,)).fetchone()
                            if row and row[0] == 'running':
                                break
                            assert process.poll() is None and time.monotonic() < deadline, (kind, boundary, row)
                            time.sleep(.002)
                        if boundary == 'prepublication_exit':
                            process.send_signal(signal.SIGKILL); process.wait(timeout=10)
                            result = call('image-status', query); assert result['state'] == 'unknown', result
                            assert call('image-request', path) == result
                            call('image-cancel', query)
                        else:
                            call('image-cancel', query); process.wait(timeout=30)
                        result = call('image-reconcile', query)
                        assert result['state'] == 'cancelled', result
                        unpublished(identity, selected)
                    else:
                        process.wait(timeout=120); assert process.returncode == 0
                        result = call('image-status', query); assert result['state'] == 'succeeded', result
                        assert call('image-request', path) == result
                        if boundary == 'late_cancel':
                            assert call('image-cancel', query) == result
                    assert call('image-status', query) == result
                    log.close(); records.append({'operation': kind, 'boundary': boundary, 'repeat': repeat + 1, 'request': identity, 'state': result['state'], 'worker_exited': True})
                    (root / 'checkpoint.json').write_text(json.dumps(records, indent=2))
                print(json.dumps({'operation': kind, 'boundary': boundary, 'passed': args.repetitions}), flush=True)
        assert sha(state / 'images' / base / 'root.raw') == disk_sha
        (root / 'report.json').write_text(json.dumps({'passed': True, 'cases': records, 'fixture_only': True, 'vm_execution_claimed': False, 'automatic_replays': 0, 'cocoon_sha256': sha(cli)}, indent=2))
    finally:
        # These exact children perform only fixture image writes, never VM execution.
        for process in children:
            if process.poll() is None:
                process.terminate(); process.wait(timeout=10)


if __name__ == '__main__':
    main()
