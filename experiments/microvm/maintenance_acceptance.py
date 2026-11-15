#!/usr/bin/env python3
"""Qualify manual retirement on nonbootable fixtures; no VM execution claim."""
import argparse
import json
import os
from pathlib import Path
import signal
import sqlite3
import subprocess
import time
import uuid


class Trial:
    def __init__(self, binary, root):
        self.binary, self.root = binary, root
        self.state = root / 'state'
        root.mkdir(mode=0o700)
        source = root / 'source'
        source.write_bytes(b'retirement fixture; not bootable\n' * 128)
        self.image = self.call('image', 'import', source, source, source, 'test fixture; MIT')['image']
        self.call('image-init')
        self.request = str(uuid.uuid4())
        query = {'request': self.request, 'caller': 'maintenance-test',
                 'credential': (self.state / 'api-credential').read_text()}
        self.query = root / 'query.json'
        self.query.write_text(json.dumps(query)); self.query.chmod(0o600)
        summary = self.call('image-info', self.image)
        self.path = root / 'request.json'
        self.path.write_text(json.dumps(dict(query, action={
            'operation': 'retire', 'image': self.image, 'sha256': summary['sha256']})))
        self.path.chmod(0o600)

    def call(self, *arguments):
        result = subprocess.run([str(self.binary), 'host', str(self.state), *map(str, arguments)],
                                capture_output=True, timeout=30)
        if result.returncode:
            raise RuntimeError(result.stderr.decode(errors='replace'))
        return json.loads(result.stdout)

    def perform(self):
        return self.call('image-request', self.path)

    def status(self):
        return self.call('image-status', self.query)

    def intact(self):
        return (self.state / 'images' / self.image / 'root.raw').is_file()


def simple(trial, scenario):
    if scenario == 'cancel_before_admission':
        assert trial.call('image-cancel', trial.query)['state'] == 'cancelled'
        try:
            trial.perform()
        except RuntimeError:
            pass
        else:
            raise AssertionError('sealed request was admitted')
        assert trial.intact()
    else:
        if scenario == 'lost_reply':
            subprocess.run([str(trial.binary), 'host', str(trial.state), 'image-request', str(trial.path)],
                           stdout=subprocess.DEVNULL, stderr=subprocess.PIPE, check=True, timeout=30)
        else:
            assert trial.perform()['state'] == 'succeeded'
        original = trial.status()
        assert original['state'] == 'succeeded' and not trial.intact()
        assert trial.perform() == original
        assert trial.call('image-cancel', trial.query)['state'] == 'succeeded'
        assert trial.call('image-reconcile', trial.query)['result'] == original['result']
    assert (trial.root / 'source').is_file()


def stop_at_intent(trial):
    # Observe the real durable fence. Missed windows remain separate trials, never invented hits.
    with (trial.root / 'worker.log').open('wb') as log:
        child = subprocess.Popen([str(trial.binary), 'host', str(trial.state), 'image-request', str(trial.path)],
                                 stdout=log, stderr=log)
        db = sqlite3.connect(trial.state / 'state.sqlite3', timeout=1)
        seen = False
        try:
            deadline = time.monotonic() + 15
            while child.poll() is None and time.monotonic() < deadline:
                row = db.execute('SELECT status FROM maintenance_intents WHERE request=?', (trial.request,)).fetchone()
                if row and row[0] == 'pending':
                    child.send_signal(signal.SIGSTOP)
                    # Recheck after the process is stopped; an already committed result is not this window.
                    row = db.execute('SELECT status FROM maintenance_intents WHERE request=?', (trial.request,)).fetchone()
                    seen = bool(row and row[0] == 'pending')
                    break
            if child.poll() is None:
                child.kill()
            child.wait(timeout=10)
        finally:
            db.close()
            if child.poll() is None:
                child.kill(); child.wait(timeout=10)
    observed = trial.status()
    if seen:
        assert observed['state'] == 'unknown', observed
        assert trial.call('image-cancel', trial.query)['state'] == 'unknown'
        assert trial.call('image-reconcile', trial.query)['state'] == 'succeeded'
        assert not trial.intact()
        return True
    if observed['state'] in ('running', 'unknown'):
        trial.call('image-cancel', trial.query)
        observed = trial.call('image-reconcile', trial.query)
    assert observed['state'] in ('succeeded', 'cancelled')
    return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--cocoon', type=Path, required=True)
    parser.add_argument('--evidence', type=Path, required=True)
    parser.add_argument('--repetitions', type=int, default=10)
    args = parser.parse_args()
    binary = args.cocoon.resolve(strict=True)
    root = args.evidence.resolve(); root.mkdir(mode=0o700)
    rows = []
    for scenario in ('cancel_before_admission', 'lost_reply', 'late_cancel'):
        for repeat in range(args.repetitions):
            trial = Trial(binary, root / (scenario + '-' + str(repeat + 1)))
            simple(trial, scenario)
            rows.append({'scenario': scenario, 'passed': True, 'observed_window': True})
    hits = 0
    for attempt in range(args.repetitions * 10):
        trial = Trial(binary, root / ('deletion-owner-loss-' + str(attempt + 1)))
        hit = stop_at_intent(trial)
        rows.append({'scenario': 'deletion_owner_loss', 'passed': True, 'observed_window': hit})
        hits += int(hit)
        (root / 'progress.json').write_text(json.dumps({'hits': hits, 'rows': rows}, indent=2))
        if hits == args.repetitions:
            break
    report = {'passed': hits == args.repetitions, 'real_vm': False,
              'deletion_owner_loss_hits': hits, 'rows': rows, 'repetitions': args.repetitions}
    (root / 'report.json').write_text(json.dumps(report, indent=2))
    assert report['passed'], 'insufficient observed durable deletion windows; retain evidence'
    print(json.dumps({'passed': True, 'trials': len(rows), 'observed_deletion_windows': hits}))


if __name__ == '__main__':
    main()
