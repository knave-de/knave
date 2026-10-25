#!/usr/bin/env python3
"""Verify the native custom charge-limit panel against a private fake service."""

import argparse
import json
import os
from pathlib import Path
import signal
import subprocess
import threading
import time

from gi.repository import Gio, GLib

SERVICE = 'org.knave.QuickSettings'
OBJECT = '/org/knave/QuickSettings'
INTERFACE = SERVICE
XML = '''<node>
  <interface name="org.knave.QuickSettings">
    <method name="Snapshot"><arg type="s" direction="out"/></method>
    <method name="Command"><arg type="s" direction="in"/></method>
    <signal name="Changed"/>
  </interface>
</node>'''


class FakeQuickSettings:
    """Private session-bus API fixture. Commands never reach knave-session."""

    def __init__(self, address):
        self.connection = Gio.DBusConnection.new_for_address_sync(
            address,
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT |
            Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)
        self.loop = GLib.MainLoop()
        self.lock = threading.RLock()
        self.mode = 'accept'
        self.commands = []
        self.completed = []
        self.state = dict(
            open=True,
            light=False,
            high_contrast=False,
            network=None,
            output=None,
            input=None,
            brightness=None,
            bluetooth=None,
            power=None,
            battery=dict(
                name='BAT_TEST',
                percent=100,
                status='Full',
                driver_limit=95,
                configured_limit=73,
                charge_limit=dict(supported=True, enabled=False,
                                  start_threshold=75, end_threshold=80)),
            night=dict(enabled=False, active=False, temperature=4000,
                       start=None, end=None),
            errors=[])
        self.info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces[0]
        self.registration = self.connection.register_object(
            OBJECT, self.info, self.method_call, None, None)
        reply = self.connection.call_sync(
            'org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', 'RequestName',
            GLib.Variant('(su)', (SERVICE, 4)), GLib.VariantType.new('(u)'),
            Gio.DBusCallFlags.NONE, 5000, None)
        if reply.unpack()[0] != 1:
            raise RuntimeError('could not own Quick Settings name on the private bus')
        self.thread = threading.Thread(target=self.loop.run,
                                      name='fake-quick-settings', daemon=True)
        self.thread.start()

    def snapshot(self):
        with self.lock:
            return json.loads(json.dumps(self.state))

    def set_mode(self, mode):
        with self.lock:
            self.mode = mode

    def emit_changed(self):
        self.connection.emit_signal(None, OBJECT, INTERFACE, 'Changed', None)

    def method_call(self, connection, sender, path, interface, name, params, invocation):
        if name == 'Snapshot':
            invocation.return_value(GLib.Variant('(s)', (json.dumps(self.snapshot()),)))
            return
        if name != 'Command':
            invocation.return_dbus_error('org.freedesktop.DBus.Error.UnknownMethod', name)
            return
        command = json.loads(params.unpack()[0])
        with self.lock:
            self.commands.append(command)
        action = command.get('action')
        if action == 'close':
            with self.lock:
                self.state['open'] = False
            self.emit_changed()
            invocation.return_value(None)
            return
        if action != 'set-charge-limit':
            invocation.return_value(None)
            return
        percent = command.get('percent')
        if not isinstance(percent, int) or not 50 <= percent <= 100:
            invocation.return_dbus_error('org.freedesktop.DBus.Error.InvalidArgs',
                                         'custom limit must be from 50 to 100')
            return
        with self.lock:
            mode = self.mode
            record = dict(percent=percent, mode=mode, done=False)
            self.completed.append(record)

        def finish():
            if mode == 'cancel':
                invocation.return_dbus_error('org.freedesktop.DBus.Error.Failed',
                                             'Authentication cancelled by test user')
            else:
                with self.lock:
                    if mode != 'mismatch':
                        self.state['battery']['configured_limit'] = percent
                        self.state['battery']['driver_limit'] = percent
                self.emit_changed()
                invocation.return_value(None)
            with self.lock:
                record['done'] = True
            return GLib.SOURCE_REMOVE

        if mode == 'delay':
            GLib.timeout_add(4000, finish)
        else:
            finish()

    def close(self):
        self.loop.quit()
        self.thread.join(timeout=3)
        self.connection.unregister_object(self.registration)
        self.connection.close_sync(None)


def wait(predicate, label, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            value = predicate()
            if value:
                return value
        except GLib.Error:
            pass
        time.sleep(.05)
    raise AssertionError('timed out waiting for ' + label)


def sample(pid):
    base = Path('/proc') / str(pid)
    stat = (base / 'stat').read_text().split(') ', 1)[1].split()
    status = dict(line.split(':', 1) for line in (base / 'status').read_text().splitlines()
                  if ':' in line)
    return dict(ticks=int(stat[11]) + int(stat[12]),
                rss_kib=int(status['VmRSS'].split()[0]),
                threads=int(status['Threads']),
                fds=len(list((base / 'fd').iterdir())),
                switches=int(status['voluntary_ctxt_switches']) +
                int(status['nonvoluntary_ctxt_switches']), at=time.monotonic())


def terminate_group(process):
    if process and process.poll() is None:
        os.killpg(process.pid, signal.SIGTERM)
        try:
            process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            os.killpg(process.pid, signal.SIGKILL)
            process.wait(timeout=5)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='96')
    args = parser.parse_args()
    bundle, directory = args.bundle.resolve(), args.run.resolve()
    harness_script = bundle / 'tools' / 'desktop-harness.py'
    checks = []

    def check(condition, label):
        if not condition:
            raise AssertionError(label)
        checks.append(label)

    started = False
    daemon = None
    fake = None
    panel = None
    session_bus = None
    try:
        subprocess.run(['python3', str(harness_script), '--run', str(directory),
                        'start', '--bundle', str(bundle), '--display', args.display,
                        '--no-vnc'], check=True, capture_output=True, text=True,
                       timeout=90)
        started = True
        import importlib.util
        spec = importlib.util.spec_from_file_location('charge_harness', harness_script)
        harness = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(harness)

        def call(operation, *values):
            return harness.request(directory, operation, list(values))

        session = call('status')['processes']['session']
        session_env = dict(entry.decode().split('=', 1)
                           for entry in Path(f'/proc/{session}/environ').read_bytes().split(b'\0')
                           if b'=' in entry)
        runtime = Path(session_env['XDG_RUNTIME_DIR'])
        wayland = next(path for path in runtime.glob('wayland-*') if path.is_socket())

        daemon = subprocess.Popen(['dbus-daemon', '--session', '--nofork',
                                   '--print-address=1'], stdout=subprocess.PIPE,
                                  stderr=subprocess.DEVNULL, text=True,
                                  start_new_session=True)
        address = daemon.stdout.readline().strip()
        if not address:
            raise RuntimeError('private panel bus did not publish an address')
        fake = FakeQuickSettings(address)
        panel_env = dict(session_env, DBUS_SESSION_BUS_ADDRESS=address,
                         WAYLAND_DISPLAY=wayland.name)
        for key in ('KNAVE_SESSION', 'KNAVE_SOCKET', 'KNAVE_OVERVIEW_SERVICE',
                    'KNAVE_HARNESS_READY_DIR'):
            panel_env.pop(key, None)
        log = (directory / 'charge-limit-panel.log').open('ab')
        panel = subprocess.Popen([str(bundle / 'bin' / 'knave-shell'), 'quick-settings'],
                                 env=panel_env, stdin=subprocess.DEVNULL,
                                 stdout=log, stderr=log, start_new_session=True)
        time.sleep(1.5)
        check(panel.poll() is None, 'standalone native Quick Settings panel starts')
        initial_state = fake.snapshot()
        check(initial_state['open'], 'fake service publishes an open Battery panel')
        check(initial_state['battery']['configured_limit'] == 73 and
              initial_state['battery']['driver_limit'] == 95,
              'configured 73% value is distinct from driver-reported 95%')
        check(initial_state['battery']['charge_limit'] == dict(
            supported=True, enabled=False, start_threshold=75, end_threshold=80),
            'UPower preset metadata remains separate from the custom limit')

        call('record-start', 'custom-charge-limit-demo')
        time.sleep(.5)
        call('screenshot', 'charge-limit-main')
        call('click', 1095, 600, 1)
        time.sleep(.5)
        call('screenshot', 'charge-limit-battery')

        def panel_sample():
            return sample(panel.pid)
        resources = {'idle_before': panel_sample()}
        time.sleep(3)
        resources['idle_after'] = panel_sample()

        def select(percent, label):
            if panel.poll() is not None:
                raise AssertionError('native Shell exited before ' + label)
            before = len(fake.commands)
            call('click', 1095, 163, 1)
            steps = percent - 50
            call('key', 'Home', *(['Right'] * steps))
            time.sleep(.25)
            check(len(fake.commands) == before,
                  f'{percent}% selection sends no command before Apply')
            call('screenshot', f'charge-limit-selected-{percent}')
            call('key', 'Tab', 'space')
            wait(lambda: len(fake.commands) > before,
                 f'Apply sends {percent}% custom limit')
            wait(lambda: len(fake.completed) > before,
                 f'{percent}% action reaches the fake service')
            record = fake.completed[-1]
            check(fake.commands[-1] == dict(action='set-charge-limit', percent=percent),
                  f'Quick Settings sends the selected {percent}% value')
            return record

        fake.set_mode('delay')
        delayed = select(73, 'delayed accepted limit')
        time.sleep(1)
        check(panel.poll() is None and not delayed['done'],
              'delayed authentication leaves panel process alive while action is pending')
        call('screenshot', 'charge-limit-73-pending')
        wait(lambda: delayed['done'], 'delayed 73% action returns', 15)
        wait(lambda: fake.snapshot()['battery']['driver_limit'] == 73,
             'fake driver accepts 73%', 5)
        time.sleep(.7)
        check(panel.poll() is None, 'native panel returns after delayed response')
        call('screenshot', 'charge-limit-73-accepted')
        check(fake.snapshot()['battery']['configured_limit'] == 73,
              'accepted 73% updates configured value in the fake service')

        fake.set_mode('cancel')
        cancelled = select(85, 'cancelled authentication')
        wait(lambda: cancelled['done'], 'cancelled 85% action returns')
        time.sleep(.7)
        check(panel.poll() is None and fake.snapshot()['open'],
              'authentication cancellation leaves native panel open')
        check(fake.snapshot()['battery']['driver_limit'] == 73 and
              fake.snapshot()['battery']['configured_limit'] == 73,
              'cancelled request preserves the last accepted value')
        call('screenshot', 'charge-limit-85-cancelled')

        fake.set_mode('mismatch')
        mismatch = select(90, 'mismatched driver readback')
        wait(lambda: mismatch['done'], 'mismatched 90% action returns')
        time.sleep(.7)
        check(panel.poll() is None and fake.snapshot()['open'],
              'driver mismatch leaves native panel open')
        check(fake.snapshot()['battery']['driver_limit'] == 73 and
              fake.snapshot()['battery']['configured_limit'] == 73,
              'mismatched request preserves the last accepted driver/config value')
        call('screenshot', 'charge-limit-90-mismatch')

        fake.set_mode('accept')
        for percent in (95, 100):
            record = select(percent, f'accepted {percent}% limit')
            wait(lambda: record['done'], f'{percent}% action returns')
            wait(lambda: fake.snapshot()['battery']['driver_limit'] == percent,
                 f'fake driver accepts {percent}%', 5)
            time.sleep(.7)
            check(fake.snapshot()['battery']['configured_limit'] == percent,
                  f'accepted {percent}% updates configured value')
            check(panel.poll() is None, f'native panel stays alive after {percent}%')
            call('screenshot', f'charge-limit-{percent}-accepted')

        check([entry['percent'] for entry in fake.completed] == [73, 85, 90, 95, 100],
              'transport received custom values 73, 85, 90, 95, and 100 percent')
        check([entry['mode'] for entry in fake.completed] ==
              ['delay', 'cancel', 'mismatch', 'accept', 'accept'],
              'success, authentication cancellation, and readback mismatch used explicit modes')
        check(fake.snapshot()['battery']['driver_limit'] == 100 and
              fake.snapshot()['battery']['configured_limit'] == 100 and
              fake.snapshot()['battery']['charge_limit'] == dict(
                  supported=True, enabled=False, start_threshold=75, end_threshold=80),
              '100% custom result leaves UPower preset metadata unchanged')
        resources['normal'] = panel_sample()
        for _ in range(8):
            call('key', 'Tab')
        resources['stress'] = panel_sample()
        check(resources['stress']['threads'] <= resources['normal']['threads'] + 1 and
              resources['stress']['fds'] <= resources['normal']['fds'] + 3,
              'native Shell client thread and fd counts remain bounded after input stress')

        call('key', 'Escape', 'Escape')
        wait(lambda: not fake.snapshot()['open'], 'native panel closes')
        check(panel.poll() is None, 'panel process remains alive after closing its surface')
        resources['idle_final_before'] = panel_sample()
        time.sleep(3)
        resources['idle_final_after'] = panel_sample()
        check(resources['idle_final_after']['ticks'] - resources['idle_final_before']['ticks'] <= 5,
              'closed native panel is idle')
        video = call('record-stop')
        check((directory / 'custom-charge-limit-demo.mp4').exists(),
              'native custom charge-limit MP4 captured')
        report = dict(
            passed=True,
            fixture=dict(configured_limit_initial=73, driver_limit_initial=95,
                         upower_preset=dict(enabled=False, start=75, end=80)),
            fake_quick_settings_commands=fake.commands,
            action_results=fake.completed,
            checks=checks,
            resources=resources,
            coverage='Native bundled Shell UI and Quick Settings D-Bus transport against a private fake service. Custom commands never reach knave-session, UPower, sysfs, or real authentication.',
            video=video)
        (directory / 'charge-limit-result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))
    finally:
        if panel:
            terminate_group(panel)
        if fake:
            try:
                fake.close()
            except GLib.Error:
                pass
        if daemon and daemon.poll() is None:
            daemon.terminate()
            try:
                daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                daemon.kill()
                daemon.wait(timeout=5)
        if started:
            try:
                harness.request(directory, 'stop', [])
            except Exception:
                pass


if __name__ == '__main__':
    main()
