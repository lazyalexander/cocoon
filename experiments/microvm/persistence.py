"""Cross-process machine-state persistence experiment; no host admission claims."""
import argparse
import base64
import json
import re
import shutil
import stat
import uuid
from pathlib import Path
from probe import BUILD, ROOT, cpio, digest, run_guest


def require(condition, message="persistence acceptance failed"):
    # Acceptance must remain active under python -O.
    if not condition:
        raise ValueError(message)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--kernel', type=Path, required=True)
    args = parser.parse_args()
    run = ROOT / 'runs' / ('persistence-' + uuid.uuid4().hex)
    run.mkdir(mode=0o700, parents=True)
    token = uuid.uuid4().hex
    initrd = run / 'initrd.gz'
    initrd.write_bytes(cpio([
        ('dev', stat.S_IFDIR | 0o755, b'', 0, 0),
        ('dev/hvc0', stat.S_IFCHR | 0o600, b'', 229, 0),
        ('init', stat.S_IFREG | 0o755, (BUILD / 'init').read_bytes(), 0, 0),
        ('token', stat.S_IFREG | 0o400, token.encode(), 0, 0),
        ('mode', stat.S_IFREG | 0o400, b'p', 0, 0),
    ]))
    disk = run / 'work.raw'
    with disk.open('xb') as f:
        f.truncate(1024 * 1024)
    state = run / 'machine.state'
    request = dict(kernel=str(args.kernel.resolve(strict=True)), initrd=str(initrd),
                   token=token, memoryMiB=128, seconds=30, disk=str(disk), state=str(state))
    report = dict(passed=False, runner_sha256=digest(BUILD / 'cocoon-vz'),
                  kernel_sha256=digest(args.kernel), initrd_sha256=digest(initrd))
    try:
        for mode in ['save', 'restore']:
            request['persistence'] = mode
            path = run / (mode + '-request.json')
            path.write_text(json.dumps(request))
            result = run_guest(BUILD / 'cocoon-vz', path, 30, False)
            (run / (mode + '-stderr.txt')).write_bytes(result.stderr)
            observed = json.loads(result.stdout)
            report[mode] = observed
            require(result.returncode == 0 and observed['termination_confirmed'], observed)
            serial = base64.b64decode(observed['serial_base64'], validate=True).replace(b'\r\n', b'\n')
            if mode == 'save':
                require(observed.get('saved') and observed['stop_reason'] == 'saved')
                match = re.search(rb'COCOON_CHECKPOINT:([0-9a-f]{32})\n', serial)
                require(match)
                marker = match[1]
                require(disk.read_bytes()[:2] == b'1P')
                require(marker not in disk.read_bytes())
                snapshot = run / 'saved.raw'
                shutil.copyfile(disk, snapshot)
                report['disk_sha256'] = digest(snapshot)
                report['state_sha256'] = digest(state)
                restored = run / 'restored.raw'
                shutil.copyfile(snapshot, restored)
                request['disk'] = str(restored)
            else:
                require(observed.get('restored_paused'))
                require(observed.get('guest_done') and observed.get('stop_reason') == 'guest_done')
                require(b'COCOON_CHECKPOINT:' not in serial, 'guest cold booted')
                require(b'COCOON_RESTORED:' + marker + b'\n' in serial)
                require(Path(request['disk']).read_bytes()[:2] == b'2D')
                require(digest(run / 'saved.raw') == report['disk_sha256'])
        report['passed'] = True
    except Exception as error:
        report['error'] = str(error)
        raise
    finally:
        (run / 'report.json').write_text(json.dumps(report, indent=2))
        print(json.dumps({'report': str(run / 'report.json'), 'passed': report['passed']}))


if __name__ == '__main__':
    main()
