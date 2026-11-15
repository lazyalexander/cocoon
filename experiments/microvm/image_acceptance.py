#!/usr/bin/env python3
"""Qualify cold disk images through original Cocoon management and work interfaces."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import time
import uuid
sys.path.insert(0, str(Path(__file__).resolve().parents[2] / 'examples'))
from agent_project import WorkClient


def sha(path):
    digest = hashlib.sha256()
    with path.open('rb') as stream:
        for data in iter(lambda: stream.read(4 * 1024 * 1024), b''):
            digest.update(data)
    return digest.hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for key in ('cocoon', 'runner', 'kernel', 'initrd', 'disk', 'project', 'evidence'):
        parser.add_argument('--' + key, type=Path, required=True)
    parser.add_argument('--rounds', type=int, choices=range(1, 4), default=3)
    args = parser.parse_args()
    root = args.evidence.resolve(); root.mkdir(mode=0o700)
    state = root / 'state'; cli = args.cocoon.resolve(strict=True); runner = args.runner.resolve(strict=True)
    records = []; active = {}; tokens = {}; server = None
    def direct(*arguments):
        result = subprocess.run([str(cli), 'host', str(state), *map(str, arguments)], capture_output=True, timeout=240)
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors='replace'))
        return json.loads(result.stdout)
    def private(path, value):
        with path.open('x') as stream:
            os.chmod(path, 0o600); json.dump(value, stream); stream.flush(); os.fsync(stream.fileno())
    def mutation(action):
        identity = str(uuid.uuid4()); request = root / (identity + '.json')
        private(request, {'request': identity, 'caller': 'image-qualification', 'credential': (state / 'api-credential').read_text(), 'action': action})
        result = direct('image-request', request)
        assert result['state'] == 'succeeded', result
        query = root / (identity + '-query.json')
        private(query, {'request': identity, 'caller': 'image-qualification', 'credential': (state / 'api-credential').read_text()})
        # Discarding the first reply must not require resubmission.
        assert direct('image-status', query) == result
        assert direct('image-request', request) == result
        records.append(result); return result['result']
    def api(operation, grant=''):
        request = root / (str(uuid.uuid4()) + '-api.json')
        private(request, {'request': str(uuid.uuid4()), 'caller': 'image-qualification', 'credential': '', 'grant': grant, 'operation': operation})
        response = direct('api', request)['result']
        if 'Ok' not in response:
            raise RuntimeError(response['Err'])
        return response['Ok']
    def start(vm):
        authority = api({'operation': 'start', 'vm': vm}); active[vm] = authority
        grant = api({'operation': 'issue_grant', 'subject': 'image-qualification', 'vm': vm, 'actions': ['inspect', 'guest', 'export', 'stop'], 'expires_unix': int(time.time()) + 900, 'max_export_bytes': 4194304})
        tokens[vm] = grant['token']
        return WorkClient(state, 'image-qualification', grant['token'], authority)
    def stop(vm):
        api({'operation': 'stop', 'vm': vm}, tokens[vm])
        report = api({'operation': 'reconcile_report', 'vm': vm})
        assert report['framework_stop_confirmed'] and report['runner_exited'], report
        del active[vm]
    def approved(image):
        reference = direct('image-info', image)
        return mutation({'operation': 'promote_candidate', 'image': image, 'sha256': reference['sha256']})
    def branch(reference):
        vm = str(uuid.uuid4())
        mutation({'operation': 'branch', 'image': reference['image'], 'sha256': reference['sha256'], 'vm': vm, 'owner': 'independent-image-task:' + vm})
        return vm
    try:
        assert direct('image-init')['memory_restore_qualified'] is False
        inputs = {key: getattr(args, key).resolve(strict=True) for key in ('kernel', 'initrd', 'disk')}
        hashes = {key + '_sha256': sha(path) for key, path in inputs.items()}
        environment_digest = hashlib.sha256(json.dumps(hashes, sort_keys=True).encode()).hexdigest()
        candidate = mutation({'operation': 'import_candidate', 'environment': environment_digest, **{key: str(path) for key, path in inputs.items()}, **hashes})
        environment = approved(candidate['image'])
        log = (root / 'control.log').open('w')
        server = subprocess.Popen([str(cli), 'host', str(state), 'serve', str(runner)], stdout=log, stderr=log)
        socket = Path('/tmp/cocoon-host-' + hashlib.sha256(os.fsencode(state)).hexdigest() + '.sock')
        deadline = time.monotonic() + 20
        while not socket.exists():
            assert server.poll() is None and time.monotonic() < deadline
            time.sleep(.025)
        for cycle in range(args.rounds):
            original = branch(environment); client = start(original)
            client.import_project(args.project.resolve(strict=True))
            saved = client.run(['/opt/python/bin/python3.12', '-I', '-B', '/workspace/checkpoint.py', 'save', '/workspace/input.md', '/workspace/input.xlsx', '/workspace/project'])
            assert saved['code'] == 0 and json.loads(saved['stdout'])['passed'], saved
            stop(original)
            disk = direct('disk-info', original)
            saved_image = mutation({'operation': 'save_candidate', **disk})
            summary = direct('image-info', saved_image['image'])
            assert summary['source_environment'] is None and summary['source_vm'] == original
            project = approved(saved_image['image']); resumed = branch(project); reader = start(resumed)
            assert uuid.UUID(active[resumed]['attempt']['id']) != uuid.UUID(client.attempt)
            verification = reader.run(['/opt/python/bin/python3.12', '-I', '-B', '/workspace/checkpoint.py', 'verify', '/workspace/project'])
            assert verification['code'] == 0 and json.loads(verification['stdout'])['passed'], verification
            copied = reader.run(['/bin/cp', '/workspace/project/result.xlsx', '/workspace/result.xlsx']); assert copied['code'] == 0
            exported = reader.export('result.xlsx', 'resumed.xlsx')
            assert exported['sha256'] == sha(args.project / 'input.xlsx')
            try:
                WorkClient(state, 'image-qualification', tokens[original], active[resumed]).context()
                raise AssertionError('original VM grant authorized a different task branch')
            except RuntimeError:
                pass
            stop(resumed)
            working = direct('disk-info', resumed)
            mutation({'operation': 'rollback', **working, 'image': environment['image'], 'sha256': environment['sha256']})
            fresh = start(resumed)
            empty = fresh.run(['/bin/sh', '-c', 'test ! -e /workspace/project && /opt/python/bin/python3.12 --version'])
            assert empty['code'] == 0, empty
            stop(resumed)
            assert sha(state / 'images' / environment['image'] / 'root.raw') == hashes['disk_sha256']
            (root / f'cycle-{cycle + 1}.json').write_text(json.dumps({'passed': True, 'original_vm': original, 'project_vm': resumed, 'project': project, 'export_sha256': exported['sha256']}, indent=2))
            print(json.dumps({'cycle': cycle + 1, 'passed': True}), flush=True)
        summary = {'passed': True, 'rounds': args.rounds, 'cold_boot_only': True, 'memory_restore_qualified': False, 'cocoon_sha256': sha(cli), 'runner_sha256': sha(runner), 'receipts': records, 'automatic_replays': 0}
        (root / 'report.json').write_text(json.dumps(summary, indent=2))
    finally:
        for vm in list(active):
            try:
                stop(vm)
            except Exception:
                pass
        if active:
            (root / 'unknown.json').write_text(json.dumps({'active_originals': active, 'automatic_retry': False}, indent=2))
        if server is not None and not active:
            server.terminate(); server.wait(timeout=15)


if __name__ == '__main__':
    main()
