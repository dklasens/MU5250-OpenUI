"""Failure injection against the scripts shipped to the modem, on a temp tree."""
import importlib.util
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]

class Credentials(unittest.TestCase):
    def test_background_child_inherits_hangup_protection_before_nohup_runs(self):
        spec = importlib.util.spec_from_file_location('startup', ROOT / 'scripts/render-agent-startup.py')
        renderer = importlib.util.module_from_spec(spec); spec.loader.exec_module(renderer)
        with tempfile.TemporaryDirectory() as temporary:
            script = Path(temporary) / 'start.sh'
            script.write_text(renderer.render('test-only password'))
            command = '''nohup() { python3 -c 'import signal; assert signal.getsignal(signal.SIGHUP) == signal.SIG_IGN'; }; . "$1"; wait $!'''
            subprocess.run(['sh', '-c', command, 'test', str(script)], check=True)

    def test_shell_round_trip_does_not_execute_password(self):
        spec = importlib.util.spec_from_file_location('startup', ROOT / 'scripts/render-agent-startup.py')
        renderer = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(renderer)
        with tempfile.TemporaryDirectory() as temp:
            marker = Path(temp) / 'executed'
            passwords = ["quote'and\"double", f'$(touch {marker})', f'`touch {marker}`', 'line1\nline2', ' spaces & ; \\ $HOME ', '-n']
            for password in passwords:
                for pin in ['', '012345']:
                    with self.subTest(password=password, pin=pin):
                        script = Path(temp) / 'start.sh'
                        script.write_text(renderer.render(password, pin))
                        subprocess.run(['sh', '-n', str(script)], check=True)
                        # Stub nohup, then source the actual generated file.
                        command = 'nohup() { :; }; . "$1"; python3 -c \'import json,os; print(json.dumps([os.getenv("ZTE_AGENT_PASSWORD"),os.getenv("ZTE_AGENT_PIN")]))\''
                        env = {**os.environ, 'ZTE_AGENT_PIN': '999999'}
                        result = subprocess.run(['sh', '-c', command, 'test', str(script)], env=env, capture_output=True, text=True, check=True)
                        self.assertEqual(json.loads(result.stdout), [password, pin or None])
                        self.assertFalse(marker.exists())

class Transactions(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.root = Path(self.temp.name)
        for directory in ['data/local/tmp', 'data/bin', 'etc/config', 'shim']:
            (self.root / directory).mkdir(parents=True)
        # macOS mv has no -T. Emulate only its no-directory-target behavior,
        # leaving the production shell transaction unchanged.
        shim = self.root / 'shim/mv'
        shim.write_text('#!/usr/bin/env python3\nimport os,sys\na=sys.argv[1:]\nif a[0]=="-Tf": os.replace(a[1],a[2])\nelse: os.execv("/bin/mv",["mv",*a])\n')
        shim.chmod(0o700)
        self.env = {**os.environ, 'U60_TEST_ROOT': str(self.root), 'PATH': f'{shim.parent}:{os.environ["PATH"]}'}
        self.write('etc/rc.local', '#!/bin/sh\nif [ "$(cat /sys/usb_op)" = 2 ]; then\n  : # stock flash protection\nfi\nexit 0\n')
        self.write('data/zte-agent', 'old binary')
        self.original_rc = (self.root / 'etc/rc.local').read_bytes()

    def tearDown(self):
        self.temp.cleanup()

    def write(self, path, value):
        target = self.root / path
        target.parent.mkdir(parents=True, exist_ok=True)
        target.write_text(value)

    def action(self, action, identity='aabb', transaction='test-123', success=True):
        result = subprocess.run(['sh', str(ROOT / 'scripts/device/transaction.sh'), action, transaction, identity], env=self.env, capture_output=True, text=True, timeout=15)
        if success:
            self.assertEqual(result.returncode, 0, result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0)
        return result

    def test_failure_restores_files_and_removes_new_installation(self):
        self.action('begin')
        self.write('data/zte-agent', 'partial replacement')
        self.write('etc/rc.local', '#!/bin/sh\nexit 0\n')
        self.write('data/bin/dropbear', 'new ssh')
        self.write('data/local/tmp/start_dashboard.sh', '#!/bin/sh\nfalse\n')
        self.action('restore')
        self.assertEqual((self.root / 'data/zte-agent').read_text(), 'old binary')
        self.assertEqual((self.root / 'etc/rc.local').read_bytes(), self.original_rc)
        self.assertFalse((self.root / 'data/bin/dropbear').exists())
        self.assertFalse((self.root / 'data/local/tmp/start_dashboard.sh').exists())
        self.action('restore') # repeated recovery remains safe

    def test_identity_mismatch_and_concurrent_deployment_cannot_write(self):
        self.action('begin')
        self.write('data/zte-agent', 'new binary')
        self.action('restore', identity='bbbb', success=False)
        self.action('begin', transaction='another', success=False)
        self.assertEqual((self.root / 'data/zte-agent').read_text(), 'new binary')
        self.action('restore')

    def test_incomplete_snapshot_cannot_restore_or_remove_live_files(self):
        base = self.root / 'data/local/tmp/open-u60-transactions'
        (base / 'lock').mkdir(parents=True)
        (base / 'lock/owner').write_text('test-123')
        transaction = base / 'test-123'; transaction.mkdir()
        (transaction / 'identity').write_text('aabb')
        self.action('restore', success=False)
        self.assertEqual((self.root / 'data/zte-agent').read_text(), 'old binary')
        self.action('discard-incomplete')
        self.assertFalse(transaction.exists())
        self.assertEqual((self.root / 'data/zte-agent').read_text(), 'old binary')

    def test_interruption_after_snapshot_can_restore_with_owned_lock(self):
        self.action('begin')
        (self.root / 'data/local/tmp/open-u60-transactions/active').unlink()
        self.write('data/zte-agent', 'new binary')
        self.action('restore')
        self.assertEqual((self.root / 'data/zte-agent').read_text(), 'old binary')

    def test_committed_snapshot_preserves_previous_dashboard_symlink(self):
        self.write('data/releases/old/index.html', 'old UI')
        os.symlink(self.root / 'data/releases/old', self.root / 'data/www.current')
        self.action('begin')
        (self.root / 'data/www.current').unlink()
        self.write('data/releases/new/index.html', 'new UI')
        os.symlink(self.root / 'data/releases/new', self.root / 'data/www.current')
        self.action('complete')
        self.action('restore')
        self.assertEqual((self.root / 'data/www.current/index.html').read_text(), 'old UI')
        self.assertEqual((self.root / 'data/releases/new/index.html').read_text(), 'new UI')


class TerminalDeployment(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        import sys
        sys.path.insert(0, str(ROOT / 'scripts'))
        spec = importlib.util.spec_from_file_location('deploy_components', ROOT / 'scripts/deploy-components.py')
        cls.module = importlib.util.module_from_spec(spec); spec.loader.exec_module(cls.module)

    def test_blocked_stdin_and_inherited_stdout_respect_deadline(self):
        import sys, time
        for command, data in [
            ([sys.executable, '-c', 'import time; time.sleep(10)'], b'x' * (4 * 1024 * 1024)),
            (['sh', '-c', 'sleep 10 & exit 0'], None),
        ]:
            start = time.monotonic()
            with self.assertRaisesRegex(RuntimeError, 'timed out'):
                self.module.run(command, data, timeout=0.1)
            self.assertLess(time.monotonic() - start, 1)

    def test_excess_output_is_bounded(self):
        import sys
        with self.assertRaisesRegex(RuntimeError, 'large management response'):
            self.module.run([sys.executable, '-c', 'import sys; sys.stdout.write("x" * 4000000)'])

    def test_complete_stdin_is_delivered_and_output_is_collected(self):
        import sys
        self.assertEqual(self.module.run([sys.executable, '-c', 'import sys; print(len(sys.stdin.buffer.read()))'], b'x' * 200000), '200000')

    def test_legacy_adb_input_uses_private_file_and_cleans_up_on_failure(self):
        from unittest.mock import patch
        payload = bytes(range(256)) * 100
        transfers, commands, local_paths = [], [], []
        def legacy_run(args, data=None):
            self.assertIsNone(data, 'Legacy adbd cannot reliably terminate streamed stdin')
            if args[3] == 'push':
                source = Path(args[4]); local_paths.append(source)
                self.assertEqual(source.stat().st_mode & 0o777, 0o600)
                transfers.append(source.read_bytes())
                return ''
            commands.append(args[-1])
            if '(failing-command) <' in args[-1]:
                return '__U60_RESULT__1'
            return '__U60_RESULT__0'
        with patch.object(self.module, 'run', side_effect=legacy_run):
            transport = self.module.Transport('192.168.0.1', 'test-serial')
            with self.assertRaisesRegex(RuntimeError, 'rejected'):
                transport.shell('failing-command', payload)
        self.assertEqual(transfers, [payload])
        self.assertTrue(all(not path.exists() for path in local_paths))
        self.assertIn('sha256sum', commands[0])
        self.assertIn('rm -f /data/local/tmp/open-u60-input-', commands[-1])

    def test_dry_run_and_wrong_identity_do_not_push_files(self):
        from types import SimpleNamespace
        from unittest.mock import patch
        class Fake:
            def __init__(self, *_): self.pushes = []
            def identity(self): return {'model': 'ZTE MU5250', 'firmware': 'test', 'fingerprint': 'aabb'}
            def shell(self, command): return '1000000\n1000000\n1000000\n100'
            def push(self, *args): self.pushes.append(args); raise AssertionError('dry run attempted a write')
        with tempfile.TemporaryDirectory() as temp:
            binary = Path(temp) / 'agent'
            header = bytearray(64); header[:6] = b'\x7fELF\x02\x01'; header[18:20] = (183).to_bytes(2, 'little'); binary.write_bytes(header)
            fake = Fake()
            args = SimpleNamespace(agent=str(binary), esim_helper=str(binary), dashboard=None, harden=False, auto_adb=False, bundle=None,
                gateway='192.168.0.1', adb_serial=None, dry_run=True, identity_file=None)
            with patch.object(self.module, 'Transport', return_value=fake), patch.dict(os.environ, {'ZTE_AGENT_PASSWORD': 'test password'}):
                self.module.deploy(args)
                identity = Path(temp) / 'identity.json'; identity.write_text('{}'); args.identity_file = str(identity)
                with self.assertRaisesRegex(RuntimeError, 'not the modem selected'): self.module.deploy(args)
            self.assertEqual(fake.pushes, [])


class ListenerSelection(unittest.TestCase):
    def test_only_our_listening_process_is_selected_including_replaced_executables(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary); (root / 'net').mkdir()
            # Linux proc net table: local address, state, socket inode.
            (root / 'net/tcp').write_text('0: 00000000:08AE 00000000:0000 0A 0:0 00:0 0 0 0 123\n0: 00000000:1F90 00000000:0000 0A 0:0 00:0 0 0 0 999\n')
            (root / 'net/tcp6').write_text('')
            for pid, executable, socket in [
                (101, '/data/bin/dropbear', 'socket:[456]'), # established SSH session
                (102, '/data/bin/dropbear (deleted)', 'socket:[123]'), # old listener after upgrade
                (103, '/usr/sbin/dropbear', 'socket:[123]'), # firmware executable
                (104, '/data/bin/dashboard-uhttpd', 'socket:[999]'),
            ]:
                (root / str(pid) / 'fd').mkdir(parents=True)
                os.symlink(executable, root / str(pid) / 'exe')
                os.symlink(socket, root / str(pid) / 'fd/3')
            result = subprocess.run(['sh', str(ROOT / 'scripts/device/stop-owned-listener.sh'), 'dropbear', '08AE', '--list'],
                env={**os.environ, 'U60_TEST_PROC_ROOT': str(root)}, capture_output=True, text=True, check=True)
            self.assertEqual(result.stdout.strip(), '102')

if __name__ == '__main__': unittest.main()
