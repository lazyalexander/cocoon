"""Exercise the actual runner callback gate without starting a VM."""
import pathlib
import platform
import subprocess
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parent


@unittest.skipUnless(platform.system() == 'Darwin', 'Apple framework compilation required')
class SnapshotCancellationTests(unittest.TestCase):
    def test_callback_gate_preserves_stop_intent_and_refuses_late_progress(self):
        # Compile the real Runner, excluding only its CLI entrypoint. No production
        # protocol, environment flag or timing hook is added for testing.
        source = (ROOT / 'runner.swift').read_text().split('\nlet runner = Runner()\n')[0]
        source += '\n' + (ROOT / 'ProjectChannel.swift').read_text()
        source += '\n' + (ROOT / 'ControlChannel.swift').read_text()
        source += '''
let testRunner = Runner()
precondition(testRunner.continueSnapshotCallback())
testRunner.stopping = true
testRunner.report["stop_reason"] = "signal"
precondition(!testRunner.continueSnapshotCallback())
precondition(!testRunner.continueSnapshotCallback())
precondition(testRunner.report["stop_reason"] as? String == "signal")
precondition(testRunner.report["termination_confirmed"] as? Bool == false)
precondition(testRunner.report["snapshot_saved"] == nil)
precondition(testRunner.report["snapshot_restored"] == nil)
testRunner.finished = true
precondition(!testRunner.continueSnapshotCallback())
testRunner.stopping = false
precondition(!testRunner.continueSnapshotCallback())
print("callback gate passed")
'''
        with tempfile.TemporaryDirectory(prefix='cocoon-callback-') as directory:
            root = pathlib.Path(directory)
            main = root / 'main.swift'
            main.write_text(source)
            binary = root / 'check'
            built = subprocess.run(['/usr/bin/swiftc', '-swift-version', '5',
                                    '-module-cache-path', str(root / 'modules'),
                                    str(main), '-framework', 'Virtualization', '-o', str(binary)],
                                   capture_output=True, text=True, timeout=180)
            self.assertEqual(built.returncode, 0, built.stderr)
            result = subprocess.run([str(binary)], capture_output=True, text=True, timeout=10)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn('callback gate passed', result.stdout)
