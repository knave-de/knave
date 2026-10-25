#!/usr/bin/env python3
"""Exercise native charge-limit UI with a fake UPower service on a private bus."""

import argparse
import json
import os
from pathlib import Path
import subprocess
import threading
import time

from gi.repository import Gio, GLib

UP_NAME = 'org.freedesktop.UPower'
UP_ROOT = '/org/freedesktop/UPower'
DEVICE = UP_ROOT + '/devices/battery_knave_smoke'
UP_IFACE = 'org.freedesktop.UPower'
DEV_IFACE = 'org.freedesktop.UPower.Device'
PROPS_IFACE = 'org.freedesktop.DBus.Properties'
CONTROL = 'org.knave.QuickSettings'

XML = f'''<node>
  <interface name="{UP_IFACE}">
    <method name="EnumerateDevices"><arg type="ao" name="devices" direction="out"/></method>
  </interface>
  <interface name="{DEV_IFACE}">
    <method name="EnableChargeThreshold"><arg type="b" name="enabled" direction="in"/></method>
    <property name="NativePath" type="s" access="read"/>
    <property name="Type" type="u" access="read"/>
    <property name="PowerSupply" type="b" access="read"/>
    <property name="IsPresent" type="b" access="read"/>
    <property name="ChargeThresholdSupported" type="b" access="read"/>
    <property name="ChargeThresholdEnabled" type="b" access="read"/>
    <property name="ChargeThresholdSettingsSupported" type="u" access="read"/>
    <property name="ChargeStartThreshold" type="u" access="read"/>
    <property name="ChargeEndThreshold" type="u" access="read"/>
  </interface>
</node>'''


def read_battery():
    root = Path('/sys/class/power_supply')
    for path in sorted(root.iterdir()):
        try:
            kind = (path / 'type').read_text().strip()
            if kind != 'Battery':
                continue
            percent = min(100, int((path / 'capacity').read_text().strip()))
            status = (path / 'status').read_text().strip()
            driver = (path / 'charge_control_end_threshold').read_text().strip()
            driver_limit = int(driver) if driver.isdecimal() else None
            return dict(name=path.name, percent=percent, status=status,
                        driver_limit=driver_limit)
        except (OSError, ValueError):
            continue
    raise RuntimeError('no readable native battery found under /sys/class/power_supply')


class FakeUPower:
    def __init__(self, address, battery_name):
        self.connection = Gio.DBusConnection.new_for_address_sync(
            address,
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT |
            Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION,
            None, None)
        self.loop = GLib.MainLoop()
        self.lock = threading.RLock()
        self.enabled = False
        self.supported = True
        self.mode = 'accept'
        self.delay_seconds = 4.0
        self.calls = []
        self.battery_name = battery_name
        self.root_info, self.device_info = Gio.DBusNodeInfo.new_for_xml(XML).interfaces
        self.registrations = [
            self.connection.register_object(UP_ROOT, self.root_info,
                                            self.method_call, None, None),
            self.connection.register_object(DEVICE, self.device_info,
                                            self.method_call,
                                            self.get_property,
                                            self.set_property),
        ]
        reply = self.connection.call_sync(
            'org.freedesktop.DBus', '/org/freedesktop/DBus',
            'org.freedesktop.DBus', 'RequestName',
            GLib.Variant('(su)', (UP_NAME, 0)), GLib.VariantType.new('(u)'),
            Gio.DBusCallFlags.NONE, 5000, None)
        if reply.unpack()[0] not in (1, 4):
            raise RuntimeError('could not own private UPower bus name')
        self.thread = threading.Thread(target=self.loop.run, name='fake-upower', daemon=True)
        self.thread.start()

    def method_call(self, connection, sender, path, interface, method, params, invocation):
        if interface == UP_IFACE and method == 'EnumerateDevices':
            invocation.return_value(GLib.Variant('(ao)', ([DEVICE],)))
            return
        if interface == DEV_IFACE and method == 'EnableChargeThreshold':
            enabled, = params.unpack()
            with self.lock:
                mode = self.mode
                self.calls.append(dict(enabled=enabled, mode=mode, at=time.monotonic()))

            def finish():
                if mode == 'reject':
                    invocation.return_dbus_error('org.freedesktop.UPower.Error.NotAuthorized',
                                                 'Authentication was denied by the fake service')
                else:
                    with self.lock:
                        if mode != 'mismatch':
                            self.enabled = enabled
                    self.emit_enabled()
                    invocation.return_value(None)
                return GLib.SOURCE_REMOVE

            if mode == 'delay':
                GLib.timeout_add(int(self.delay_seconds * 1000), finish)
            else:
                finish()
            return
        invocation.return_dbus_error('org.freedesktop.DBus.Error.UnknownMethod', method)

    def get_property(self, connection, sender, path, interface, name):
        if interface != DEV_IFACE:
            return None
        with self.lock:
            values = {
                'NativePath': GLib.Variant('s', '/sys/class/power_supply/' + self.battery_name),
                'Type': GLib.Variant('u', 2),
                'PowerSupply': GLib.Variant('b', True),
                'IsPresent': GLib.Variant('b', True),
                'ChargeThresholdSupported': GLib.Variant('b', self.supported),
                'ChargeThresholdEnabled': GLib.Variant('b', self.enabled),
                'ChargeThresholdSettingsSupported': GLib.Variant('u', 2),
                'ChargeStartThreshold': GLib.Variant('u', 75),
                'ChargeEndThreshold': GLib.Variant('u', 80),
            }
            return values.get(name)

    def set_property(self, connection, sender, path, interface, name, value):
        return False

    def emit_enabled(self):
        with self.lock:
            current = self.enabled
        self.connection.emit_signal(
            None, DEVICE, PROPS_IFACE, 'PropertiesChanged',
            GLib.Variant('(sa{sv}as)',
                         (DEV_IFACE, {'ChargeThresholdEnabled': GLib.Variant('b', current)}, [])))

    def close(self):
        self.loop.quit()
        self.thread.join(timeout=3)
        for registration in self.registrations:
            self.connection.unregister_object(registration)
        self.connection.close_sync(None)


def wait(predicate, label, seconds=15):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            result = predicate()
            if result:
                return result
        except GLib.Error:
            pass
        time.sleep(.1)
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


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='97')
    args = parser.parse_args()
    bundle, directory = args.bundle.resolve(), args.run.resolve()
    directory.mkdir(parents=True, exist_ok=False)
    battery = read_battery()
    checks = []
    def check(value, label):
        if not value:
            raise AssertionError(label)
        checks.append(label)

    bus = subprocess.Popen(['dbus-daemon', '--session', '--nofork', '--print-address=1'],
                           stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                           text=True, start_new_session=True)
    fake = None
    started = False
    try:
        address = bus.stdout.readline().strip()
        if not address:
            raise RuntimeError('private system-bus daemon did not publish an address')
        fake = FakeUPower(address, battery['name'])
        harness_script = Path(__file__).with_name('desktop-harness.py')
        env = dict(os.environ, DBUS_SYSTEM_BUS_ADDRESS=address)
        subprocess.run(['python3', str(harness_script), '--run', str(directory), 'start',
                        '--bundle', str(bundle), '--display', args.display, '--no-vnc'],
                       env=env, check=True, capture_output=True, text=True, timeout=90)
        started = True
        import importlib.util
        spec = importlib.util.spec_from_file_location('charge_harness', harness_script)
        harness = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(harness)
        def call(op, *values):
            return harness.request(directory, op, list(values))

        session = call('status')['processes']['session']
        session_env = dict(entry.decode().split('=', 1)
                           for entry in Path(f'/proc/{session}/environ').read_bytes().split(b'\0')
                           if b'=' in entry)
        session_bus = Gio.DBusConnection.new_for_address_sync(
            session_env['DBUS_SESSION_BUS_ADDRESS'],
            Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT |
            Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)

        def snapshot():
            value = session_bus.call_sync(
                CONTROL, '/org/knave/QuickSettings', CONTROL, 'Snapshot', None,
                GLib.VariantType.new('(s)'), Gio.DBusCallFlags.NO_AUTO_START, 5000, None)
            return json.loads(value.unpack()[0])

        wait(lambda: snapshot(), 'native quick-settings service')
        state = snapshot()
        check(state['battery']['name'] == battery['name'], 'Snapshot identifies actual read-only sysfs battery')
        check(state['battery'].get('driver_limit') == battery['driver_limit'],
              'Snapshot reports driver threshold separately from UPower preset')
        limit = state['battery']['charge_limit']
        check(limit == dict(supported=True, enabled=False, start_threshold=75, end_threshold=80),
              'UPower reports supported 75%/80% preset independently of driver threshold')

        children = [int(p) for p in Path(f'/proc/{session}/task/{session}/children').read_text().split()]
        monitored = [pid for pid in [session] + children if Path(f'/proc/{pid}').exists()]
        def samples():
            return {str(pid): sample(pid) for pid in monitored if Path(f'/proc/{pid}').exists()}
        resources = {'idle_before': samples()}
        time.sleep(3)
        resources['idle_after'] = samples()

        call('record-start', 'charge-limit-demo')
        call('click', 1248, 18, 1)
        wait(lambda: snapshot()['open'], 'quick settings opens')
        time.sleep(.4)
        call('screenshot', 'charge-limit-main')
        # The charge row is below the fold at 1280×800; scroll it into view.
        call('click', 1080, 710, 5)
        time.sleep(.3)
        call('screenshot', 'charge-limit-main-scrolled')
        call('click', 1080, 740, 1)
        time.sleep(.5)
        call('screenshot', 'charge-limit-page')
        check(Path(directory / 'charge-limit-page.png').exists(), 'native Battery charging page screenshot captured')

        def apply_and_wait(target, expected, mode='accept', timeout=20):
            with fake.lock:
                fake.mode = mode
                before = len(fake.calls)
            call('click', 1080, 300, 1)
            wait(lambda: len(fake.calls) > before, f'UPower EnableChargeThreshold({target}) request')
            if mode == 'delay':
                time.sleep(1)
                call('screenshot', f'charge-limit-{target}-pending')
            wait(lambda: snapshot()['battery']['charge_limit']['enabled'] == expected,
                 f'UPower reports enabled={expected}', timeout)
            time.sleep(.5)
            call('screenshot', f'charge-limit-{target}-result')

        apply_and_wait('enabled', True, mode='delay', timeout=20)
        check(fake.calls[-1]['enabled'] is True, 'delayed native enable reaches UPower')
        apply_and_wait('disabled', False)
        check(fake.calls[-1]['enabled'] is False, 'native disable reaches UPower')
        apply_and_wait('reapply', True)
        check(fake.calls[-1]['enabled'] is True, 'native reapply reaches UPower')

        # Drive failure states from the same native control. Service readback stays false.
        def failure(mode, filename, label):
            with fake.lock:
                fake.enabled = False
                fake.mode = mode
                before = len(fake.calls)
            fake.emit_enabled()
            time.sleep(.3)
            call('click', 1080, 300, 1)
            wait(lambda: len(fake.calls) > before, label + ' request')
            time.sleep(1.5)
            call('screenshot', filename)
            check(fake.enabled is False, label + ' does not claim UPower changed state')
            check(Path(directory / (filename + '.png')).exists(), label + ' UI error screenshot captured')
            process_status = call('status')
            check(process_status['processes']['session'] == session, label + ' leaves native session alive')

        failure('reject', 'charge-limit-denied', 'UPower denial')
        failure('mismatch', 'charge-limit-mismatch', 'UPower readback mismatch')
        check(all(c['mode'] in ('accept', 'delay', 'reject', 'mismatch') for c in fake.calls),
              'all enable requests handled by selected fake UPower modes')
        call('key', 'Escape')
        time.sleep(1)
        resources['normal'] = samples()
        for _ in range(8):
            session_bus.call_sync(CONTROL, '/org/knave/QuickSettings', CONTROL, 'Command',
                                  GLib.Variant('(s)', (json.dumps({'action': 'refresh'}),)), None,
                                  Gio.DBusCallFlags.NO_AUTO_START, 10000, None)
        resources['stress'] = samples()
        video = call('record-stop')
        check(Path(directory / 'charge-limit-demo.mp4').exists(), 'MP4 demonstration captured')
        report = dict(passed=True, battery=battery, fake_upower_calls=fake.calls,
                      checks=checks, resources=resources, video=video,
                      bus='private DBUS_SYSTEM_BUS_ADDRESS; no host system bus or polkit authorization used')
        (directory / 'charge-limit-result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))
    finally:
        if started:
            try:
                call('stop')
            except Exception:
                pass
        if fake:
            fake.close()
        if bus.poll() is None:
            bus.terminate()
            try:
                bus.wait(timeout=5)
            except subprocess.TimeoutExpired:
                bus.kill()
                bus.wait(timeout=5)


if __name__ == '__main__':
    main()
