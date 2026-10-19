#!/usr/bin/env python3
"""Verify notification delivery, persistence and actual shell interaction on a frozen bundle."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import time
from gi.repository import Gio, GLib

STANDARD = 'org.freedesktop.Notifications'
CONTROL = 'org.knave.Notifications'


def connect(address):
    return Gio.DBusConnection.new_for_address_sync(address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)


def invoke(bus, destination, path, interface, method, signature='()', values=()):
    return bus.call_sync(destination, path, interface, method, GLib.Variant(signature, values), None, Gio.DBusCallFlags.NO_AUTO_START, 3000, None).unpack()


def state(bus):
    return json.loads(invoke(bus, CONTROL, '/org/knave/Notifications', CONTROL, 'Snapshot')[0])


def command(bus, op, **values):
    invoke(bus, CONTROL, '/org/knave/Notifications', CONTROL, 'Command', '(s)', (json.dumps(dict(op=op, **values)),))


def notify(bus, summary, body='', app='Knave demo', app_id='org.knave.Demo', timeout=1500, actions=(), hints=None):
    hints = dict(hints or {})
    hints['desktop-entry'] = GLib.Variant('s', app_id)
    return invoke(bus, STANDARD, '/org/freedesktop/Notifications', STANDARD, 'Notify', '(susssasa{sv}i)', (app, 0, 'dialog-information', summary, body, list(actions), hints, timeout))[0]


def wait(predicate, label, seconds=10):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        try:
            result = predicate()
            if result:
                return result
        except GLib.Error:
            pass
        time.sleep(0.05)
    raise AssertionError(label)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--display', default='96')
    parser.add_argument('--size', default='1280x800')
    parser.add_argument('--light', action='store_true')
    parser.add_argument('--high-contrast', action='store_true')
    args = parser.parse_args()
    directory, bundle = args.run.resolve(), args.bundle.resolve()
    spec = importlib.util.spec_from_file_location('notification_harness', Path(__file__).with_name('desktop-harness.py'))
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    checks, resources = [], {}
    width, height = map(int, args.size.split('x'))
    config = directory.with_suffix('.toml')
    with config.open('x') as output:
        output.write(f'[appearance]\ntheme = "{"light" if args.light else "dark"}"\n[portal]\nhigh_contrast = {str(args.high_contrast).lower()}\n[notifications]\nbanner_timeout_ms = 1500\n')

    def call(operation, *values):
        return harness.request(directory, operation, list(values))

    def check(condition, label):
        if not condition:
            raise AssertionError(label)
        checks.append(label)

    def shot(name):
        time.sleep(0.25)
        return call('screenshot', name)

    def key(*keys):
        call('key', *keys)

    def open_center():
        call('click', width - 28, 18, 1)
        wait(lambda: state(bus)['center_open'], 'bar bell opens center')
        time.sleep(0.25)

    def close_center():
        key('Escape')
        wait(lambda: not state(bus)['center_open'], 'Escape closes center')

    def sample(pid):
        base = Path('/proc') / str(pid)
        fields = (base / 'stat').read_text().split(') ', 1)[1].split()
        status = dict(line.split(':', 1) for line in (base / 'status').read_text().splitlines() if ':' in line)
        return dict(ticks=int(fields[11]) + int(fields[12]), rss_kib=int(status['VmRSS'].split()[0]), threads=int(status['Threads']), fds=len(list((base / 'fd').iterdir())), context_switches=int(status['voluntary_ctxt_switches'])+int(status['nonvoluntary_ctxt_switches']), at=time.monotonic())

    def sample_set():
        return {str(pid): sample(pid) for pid in process_ids if Path(f'/proc/{pid}').exists()}

    started = False
    try:
        subprocess.run(['python3', str(Path(__file__).with_name('desktop-harness.py')), '--run', str(directory), 'start', '--bundle', str(bundle), '--display', args.display, '--no-vnc', '--size', args.size, '--config', str(config)], check=True, capture_output=True, text=True, timeout=90)
        started = True
        session = call('status')['processes']['session']
        raw = Path(f'/proc/{session}/environ').read_bytes().split(b'\0')
        session_env = dict(entry.decode().split('=', 1) for entry in raw if b'=' in entry)
        bus = connect(session_env['DBUS_SESSION_BUS_ADDRESS'])
        wait(lambda: state(bus)['owned'], 'Knave owns delivery')
        check(state(bus)['notifications'] == [], 'fresh harness history is isolated')
        process_ids = [session] + [int(p) for p in Path(f'/proc/{session}/task/{session}/children').read_text().split()]
        resources['idle_before'] = sample_set()
        time.sleep(2)
        resources['idle_after'] = sample_set()
        call('record-start', 'notifications-demo')
        first = notify(bus, 'Your files are ready', '12 files copied to Documents.', app='Files', app_id='org.knave.Files')
        second = notify(bus, 'Project review', 'The revised mockups are ready. This notification remains in history after its banner disappears.', app='Mail', app_id='org.knave.Mail')
        wait(lambda: sum(n['unread'] for n in state(bus)['notifications']) == 2, 'unread count reaches two')
        shot('banners-and-badge')
        wait(lambda: not any(n['banner'] for n in state(bus)['notifications']), 'banners expire')
        check(len(state(bus)['notifications']) == 2, 'banner expiry retains history')
        shot('retained-unread-badge')
        open_center()
        wait(lambda: not any(n['unread'] for n in state(bus)['notifications']), 'visible center entries become read')
        shot('notification-history')
        # Focus starts on Close; keyboard traversal reaches DND without relying on layout coordinates.
        key('Tab', 'space')
        wait(lambda: state(bus)['dnd'], 'DND toggled through actual shell button')
        close_center()
        notify(bus, 'Arrived quietly', 'Do not disturb keeps this notification in history.', app='Mail', app_id='org.knave.Mail')
        check(not any(n['banner'] for n in state(bus)['notifications']), 'DND suppresses banners')
        check(len(state(bus)['notifications']) == 3, 'DND preserves delivery and history')
        shot('do-not-disturb')
        open_center()
        key('Tab', 'space')
        wait(lambda: not state(bus)['dnd'], 'DND disabled through actual shell button')
        check(not any(n['banner'] for n in state(bus)['notifications']), 'disabling DND does not replay backlog')
        close_center()
        transient = notify(bus, 'Volume 65%', hints={'transient': GLib.Variant('b', True)}, timeout=700)
        wait(lambda: any(n['id'] == transient and n['banner'] for n in state(bus)['notifications']), 'transient banner displayed')
        wait(lambda: not any(n['id'] == transient for n in state(bus)['notifications']), 'transient disappears without history')
        command(bus, 'clear')
        action_id = notify(bus, 'Download complete', 'Application actions, progress and urgency use the same notification card.', app='Downloads', app_id='org.knave.Downloads', timeout=0, actions=('default', 'Open containing folder', 'retry', 'Download another copy with a longer action label'), hints={'value': GLib.Variant('i', 100), 'urgency': GLib.Variant('y', 2), 'resident': GLib.Variant('b', True)})
        open_center()
        shot('actions-progress-critical')
        # Close, DND, Mark read, Clear, Dismiss, then the first action.
        key('Tab', 'Tab', 'Tab', 'Tab', 'Tab', 'space')
        wait(lambda: not next(n for n in state(bus)['notifications'] if n['id'] == action_id)['unread'], 'resident action keeps its card')
        close_center()
        command(bus, 'clear')
        muted = notify(bus, 'Per-application controls', 'Mute only this application’s future banners.', timeout=0)
        open_center()
        key('Tab', 'Tab', 'Tab', 'Tab', 'Tab', 'space')
        wait(lambda: state(bus)['app_banners'].get('org.knave.Demo') is False, 'per-app banner preference saved by actual UI')
        close_center()
        notify(bus, 'Muted app still has history', 'This should not display a banner.')
        check(not any(n['banner'] for n in state(bus)['notifications']), 'muted application retains history without banners')
        command(bus, 'set_app_banners', app_id='org.knave.Demo', enabled=True)
        command(bus, 'clear')
        for index in range(30):
            notify(bus, f'History item {index+1}', 'Long wrapping text 日本語 café العربية. ' * 12, app='History test', app_id='org.knave.History', timeout=400)
        wait(lambda: not any(n['banner'] for n in state(bus)['notifications']), 'burst banners expire')
        check(len(state(bus)['notifications']) == 30, 'burst history retained without lost deliveries')
        open_center()
        shot('long-content-and-pagination')
        # The footer remains outside the scroll region, even on narrow/short outputs.
        call('click', width - 42, height - 52, 1)
        shot('older-history-page')
        close_center()
        resources['normal'] = sample_set()
        command(bus, 'set_dnd', enabled=True)
        for index in range(220):
            notify(bus, f'Bounded history stress {index}', timeout=1)
        wait(lambda: all(not n['live'] for n in state(bus)['notifications']), 'stress entries expire')
        check(len(state(bus)['notifications']) == 200, 'history bounded at configured 200 entries')
        resources['stress'] = sample_set()
        for pid, before in resources['idle_before'].items():
            after = resources['stress'].get(pid)
            if after:
                check(after['threads'] <= before['threads'] + 2, f'bounded threads for process {pid}')
                check(after['fds'] <= before['fds'] + 8, f'bounded descriptors for process {pid}')
        command(bus, 'set_dnd', enabled=False)
        open_center()
        key('Tab', 'Tab', 'Tab', 'space')
        wait(lambda: not state(bus)['notifications'], 'Clear all works through shell control')
        shot('empty-history')
        close_center()
        call('record-stop')
        standalone_checks(bundle, directory, session_env, checks)
        (directory / 'notification-result.json').write_text(json.dumps(dict(passed=True, assertions=checks, resources=resources, bundle=str(bundle), size=args.size, light=args.light, high_contrast=args.high_contrast), indent=2)+'\n')
        print(json.dumps(dict(passed=True, assertions=len(checks), video=str(directory / 'notifications-demo.mp4'))))
    finally:
        if started:
            call('stop')


def standalone_checks(bundle, directory, inherited, checks):
    """Restart the real service on a separate bus; never interrupt the desktop session."""
    daemon = subprocess.Popen(['dbus-daemon', '--session', '--nofork', '--print-address=1'], stdout=subprocess.PIPE, text=True)
    service = None
    competitor = None
    connection = None
    try:
        address = daemon.stdout.readline().strip()
        env = dict(inherited, DBUS_SESSION_BUS_ADDRESS=address, XDG_STATE_HOME=str(directory / 'restart-state'))
        env['KNAVE_CONFIG'] = str(directory.with_suffix('.toml'))
        connection = connect(address)
        log = (directory / 'standalone.log').open('w')
        def start():
            return subprocess.Popen([str(bundle / 'bin/knave-session'), 'notifications'], env=env, stdout=log, stderr=log)
        service = start()
        wait(lambda: state(connection)['owned'], 'standalone notification service starts')
        saved = notify(connection, 'Survives restart', 'Stored transactionally', timeout=0, actions=('default', 'Open'))
        command(connection, 'set_dnd', enabled=True)
        service.terminate()
        service.wait(timeout=5)
        service = start()
        wait(lambda: state(connection)['notifications'], 'history restored after service restart')
        restored = state(connection)
        assert restored['dnd'] and restored['notifications'][0]['id'] == saved
        assert not restored['notifications'][0]['live'] and not restored['notifications'][0]['banner'] and not restored['notifications'][0]['actions']
        checks.append('real service restart preserves history/DND and disables stale actions/banners')
        new = notify(connection, 'New protocol identity', timeout=0)
        assert new > saved
        checks.append('protocol ID not reused after restart')
        # A later contender cannot replace Knave, even if it explicitly requests replacement.
        contender = connect(address)
        reply = invoke(contender, 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', '(su)', (STANDARD, 6))[0]
        assert reply == 3
        checks.append('later notification service cannot take over Knave')
        contender.close_sync(None)
        service.terminate()
        service.wait(timeout=5)
        service = None
        # Exercise the same first-owner semantics with an external daemon when available.
        shell_children = Path(f'/proc/{inherited.get("KNAVE_TEST_SESSION_PID", "0")}/task').exists()
        wayland = inherited.get('WAYLAND_DISPLAY')
        if wayland and Path('/usr/bin/swaync').exists():
            competitor = subprocess.Popen(['/usr/bin/swaync'], env=env, stdout=log, stderr=log)
            wait(lambda: invoke(connection, 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'NameHasOwner', '(s)', (STANDARD,))[0], 'swaync owns private notification bus')
        else:
            contender = connect(address)
            invoke(contender, 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', '(su)', (STANDARD, 5))
        service = start()
        wait(lambda: not state(connection)['owned'], 'existing external owner wins')
        checks.append('existing external notification service remains owner')
        if competitor:
            competitor.terminate()
            competitor.wait(timeout=5)
            competitor = None
        else:
            contender.close_sync(None)
        time.sleep(0.3)
        assert not invoke(connection, 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'NameHasOwner', '(s)', (STANDARD,))[0]
        checks.append('losing Knave never queues or reclaims after external service stops')
    finally:
        for process in (competitor, service, daemon):
            if process is not None:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait()
        if connection:
            connection.close_sync(None)


if __name__ == '__main__':
    main()
