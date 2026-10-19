"""Regressions for configuration isolation, service readiness and daemon startup."""

import argparse
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile
import tomllib
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('desktop_harness', Path(__file__).with_name('desktop-harness.py'))
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


class HarnessTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix='kh-regression-')
        self.addCleanup(self.temporary.cleanup)
        self.directory = Path(self.temporary.name)

    def normalize(self, source):
        config = self.directory / 'source.toml'
        config.write_text(source)
        target = self.directory / 'run'
        target.mkdir()
        harness.write_config(target / 'config.toml', argparse.Namespace(config=config),
                             {'villain': Path('/build/villain'), 'knave-shell': Path('/build/knave-shell')})
        self.assertEqual(config.read_text(), source)
        return target, tomllib.loads((target / 'config.toml').read_text())

    def test_unconfigured_sibling_environment_is_not_imported(self):
        (self.directory / 'environment').write_text('UNREQUESTED_OVERRIDE=yes\n')
        target, config = self.normalize('[compositor]\nmaster_percent = 60\n')
        self.assertNotIn('environment_file', config['compositor'])
        self.assertFalse((target / 'environment').exists())

    def test_explicit_relative_environment_is_copied(self):
        (self.directory / 'custom-env').write_text('REQUESTED_OVERRIDE=yes\n')
        target, config = self.normalize('[compositor]\nenvironment_file = "custom-env"\n')
        self.assertEqual(config['compositor']['environment_file'], 'environment')
        self.assertEqual((target / 'environment').read_text(), 'REQUESTED_OVERRIDE=yes\n')

    def test_spawned_process_cannot_satisfy_shell_readiness(self):
        (self.directory / 'spawned-pid').write_text(str(os.getpid()))
        self.assertFalse(harness.shell_services_ready(self.directory, {'bar', 'overview'}))
        (self.directory / 'bar.ready').write_text('ready\n')
        (self.directory / 'overview.ready').touch()
        self.assertFalse(harness.shell_services_ready(self.directory, {'bar', 'overview'}))
        (self.directory / 'overview.ready').write_text('ready\n')
        self.assertTrue(harness.shell_services_ready(self.directory, {'bar', 'overview'}))
        self.assertTrue(harness.shell_services_ready(self.directory, set()))

    def fake_daemon(self, response, delay=0.4):
        script = self.directory / 'fake-daemon.py'
        script.write_text(
            'import json, os, socket, time\n'
            f'directory = {str(self.directory)!r}\n'
            'open(directory + "/pid", "w").write(str(os.getpid()))\n'
            'server = socket.socket(socket.AF_UNIX)\n'
            'server.bind(directory + "/control.sock")\n'
            f'time.sleep({delay})\n'
            'server.listen(1)\n'
            'client, _ = server.accept()\n'
            'client.recv(4096)\n'
            f'client.sendall({(json.dumps(response) + chr(10)).encode()!r})\n'
            'client.close()\n'
            'time.sleep(30)\n')
        return [sys.executable, str(script)]

    def assert_daemon_reaped(self):
        pid = int((self.directory / 'pid').read_text())
        with self.assertRaises(ProcessLookupError):
            os.kill(pid, 0)

    def test_bound_socket_is_retried_until_server_listens(self):
        import subprocess
        spawn = subprocess.Popen
        children = []

        def tracked_spawn(*args, **kwargs):
            child = spawn(*args, **kwargs)
            children.append(child)
            return child

        try:
            with patch.object(harness.subprocess, 'Popen', side_effect=tracked_spawn):
                response = harness.start_daemon(self.fake_daemon({'ready': True}), self.directory, timeout=3)
        finally:
            for child in children:
                if child.poll() is None:
                    child.terminate()
                child.wait(timeout=5)
        self.assertEqual(response, {'ready': True})

    def test_unexpected_status_error_terminates_and_reaps_daemon(self):
        with self.assertRaisesRegex(RuntimeError, 'invalid startup'):
            harness.start_daemon(self.fake_daemon({'error': 'invalid startup'}), self.directory, timeout=3)
        self.assert_daemon_reaped()

    def test_timeout_terminates_and_reaps_daemon(self):
        with self.assertRaisesRegex(RuntimeError, 'startup timed out'):
            harness.start_daemon(self.fake_daemon({}, delay=5), self.directory, timeout=0.4)
        self.assert_daemon_reaped()

    def test_password_uses_stdin_not_process_arguments(self):
        password = 'abcd1234'
        with patch.object(harness, 'run') as invoke:
            harness.create_vnc_auth(self.directory / 'vnc.auth', password)
        argv = invoke.call_args.args[0]
        self.assertNotIn(password, argv)
        self.assertEqual(argv, ['x11vnc', '-storepasswd', str(self.directory / 'vnc.auth')])
        self.assertEqual(invoke.call_args.kwargs['input'], f'{password}\n{password}\ny\n')
        self.assertTrue(invoke.call_args.kwargs['start_new_session'])


if __name__ == '__main__':
    unittest.main()
