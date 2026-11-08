#!/usr/bin/env python3
"""Exercise tray registration, pixels, actions and exported menus in a frozen desktop."""
import argparse
import importlib.util
import json
from pathlib import Path
import subprocess
import time
import gi
gi.require_version('GdkPixbuf', '2.0')
from gi.repository import Gio, GLib, GdkPixbuf

KDE = 'org.kde.StatusNotifierWatcher'
ITEM = 'org.kde.StatusNotifierItem'
MENU = 'com.canonical.dbusmenu'
XML = '''<node><interface name="org.kde.StatusNotifierItem">
<property name="Status" type="s" access="read"/><property name="Title" type="s" access="read"/>
<property name="IconName" type="s" access="read"/><property name="IconPixmap" type="a(iiay)" access="read"/>
<property name="AttentionIconName" type="s" access="read"/><property name="AttentionIconPixmap" type="a(iiay)" access="read"/>
<property name="ItemIsMenu" type="b" access="read"/><property name="Menu" type="o" access="read"/>
<method name="Activate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<method name="ContextMenu"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<method name="SecondaryActivate"><arg type="i" direction="in"/><arg type="i" direction="in"/></method>
<method name="Scroll"><arg type="i" direction="in"/><arg type="s" direction="in"/></method>
<signal name="NewIcon"/><signal name="NewStatus"><arg type="s"/></signal>
</interface><interface name="com.canonical.dbusmenu">
<method name="AboutToShow"><arg type="i" direction="in"/><arg type="b" direction="out"/></method>
<method name="GetLayout"><arg type="i" direction="in"/><arg type="i" direction="in"/><arg type="as" direction="in"/>
<arg type="u" direction="out"/><arg type="(ia{sv}av)" direction="out"/></method>
<method name="Event"><arg type="i" direction="in"/><arg type="s" direction="in"/><arg type="v" direction="in"/><arg type="u" direction="in"/></method>
</interface></node>'''


def connect(address):
    return Gio.DBusConnection.new_for_address_sync(address, Gio.DBusConnectionFlags.AUTHENTICATION_CLIENT | Gio.DBusConnectionFlags.MESSAGE_BUS_CONNECTION, None, None)


def invoke(bus, destination, path, interface, method, signature='()', values=()):
    return bus.call_sync(destination, path, interface, method, GLib.Variant(signature, values), None, Gio.DBusCallFlags.NONE, 3000, None).unpack()


def pump(seconds=0.1):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        while GLib.MainContext.default().pending():
            GLib.MainContext.default().iteration(False)
        time.sleep(0.01)


def wait(predicate, label, seconds=10):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        pump()
        if predicate():
            return
    raise AssertionError(label)


class Fixture:
    def __init__(self, address, title, color, menu_only=False, freedesktop=False, name=None):
        self.bus = connect(address)
        self.title, self.color, self.menu_only = title, color, menu_only
        self.status, self.actions = 'Active', []
        self.interface = ITEM.replace('org.kde.', 'org.freedesktop.') if freedesktop else ITEM
        self.watcher = KDE.replace('org.kde.', 'org.freedesktop.') if freedesktop else KDE
        self.name = name or '/StatusNotifierItem'
        if name:
            invoke(self.bus, 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'RequestName', '(su)', (name, 4))
        node = Gio.DBusNodeInfo.new_for_xml(XML.replace(ITEM, self.interface))
        self.bus.register_object('/StatusNotifierItem', node.interfaces[0], self.method, self.prop, None)
        self.bus.register_object('/Menu', node.interfaces[1], self.method, None, None)
        self.register()

    def register(self):
        invoke(self.bus, self.watcher, '/StatusNotifierWatcher', self.watcher, 'RegisterStatusNotifierItem', '(s)', (self.name,))

    def prop(self, _bus, _sender, _path, _iface, name):
        if name == 'Status': return GLib.Variant('s', self.status)
        if name == 'Title': return GLib.Variant('s', self.title)
        if name in ('IconName', 'AttentionIconName'): return GLib.Variant('s', '')
        if name in ('IconPixmap', 'AttentionIconPixmap'):
            color = self.color if name == 'IconPixmap' else (70, 130, 240)
            return GLib.Variant('a(iiay)', [(22, 22, bytes((255, *color)) * (22 * 22))])
        if name == 'ItemIsMenu': return GLib.Variant('b', self.menu_only)
        if name == 'Menu': return GLib.Variant('o', '/Menu')

    def method(self, _bus, _sender, _path, _iface, method, params, reply):
        values = params.unpack()
        if method == 'AboutToShow': reply.return_value(GLib.Variant('(b)', (False,)))
        elif method == 'GetLayout':
            def row(id, label, **kwargs):
                props = {'label': GLib.Variant('s', label)}
                props.update({key.replace('_', '-'): GLib.Variant('b' if isinstance(value, bool) else 's', value) for key, value in kwargs.items()})
                return GLib.Variant('(ia{sv}av)', (id, props, []))
            children = [row(1, '_Show app'), row(2, 'Unavailable', enabled=False), row(42, 'More', children_display='submenu')] if values[0] == 0 else [row(43, '_Quit app')]
            reply.return_value(GLib.Variant('(u(ia{sv}av))', (1, (values[0], {}, children))))
        else:
            self.actions.append((method, values))
            reply.return_value(GLib.Variant('()', ()))

    def change(self, status=None, color=None):
        if status is not None:
            self.status = status
            self.bus.emit_signal(None, '/StatusNotifierItem', self.interface, 'NewStatus', GLib.Variant('(s)', (status,)))
        if color is not None:
            self.color = color
            self.bus.emit_signal(None, '/StatusNotifierItem', self.interface, 'NewIcon', None)


def sample(pid):
    base = Path('/proc') / str(pid)
    fields = (base / 'stat').read_text().split(') ', 1)[1].split()
    state = dict(line.split(':', 1) for line in (base / 'status').read_text().splitlines() if ':' in line)
    return dict(ticks=int(fields[11]) + int(fields[12]), rss_kib=int(state['VmRSS'].split()[0]), threads=int(state['Threads']), fds=len(list((base / 'fd').iterdir())), wakeups=int(state['voluntary_ctxt_switches']), at=time.monotonic())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='104')
    parser.add_argument('--harness', type=Path, default=Path(__file__).resolve().parents[2] / 'knave/scripts/desktop-harness.py')
    args = parser.parse_args()
    directory = args.run.resolve()
    spec = importlib.util.spec_from_file_location('tray_harness', args.harness)
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    checks, resources, fixtures = [], {}, []
    started = False
    counter = 0

    def call(op, *values): return harness.request(directory, op, list(values))
    def shot(label):
        pump(0.4)
        return call('screenshot', label)
    def pixel(x, y):
        nonlocal counter
        counter += 1
        subprocess.run(['xdotool', 'mousemove', '640', '400'], env=env, check=True)
        result = shot(f'pixel-{counter}')
        path = Path(result['screenshot'])
        image = GdkPixbuf.Pixbuf.new_from_file(str(path))
        i = y * image.get_rowstride() + x * image.get_n_channels()
        return tuple(image.get_pixels()[i:i+3])
    def color(x, expected, label):
        wait(lambda: pixel(x, 18) == expected, label)
        checks.append(label)
    def key(*keys): call('key', *keys); pump(0.3)
    def click(x, y, button=1): call('click', x, y, button); pump(0.4)
    def registered(): return invoke(bus, KDE, '/StatusNotifierWatcher', 'org.freedesktop.DBus.Properties', 'Get', '(ss)', (KDE, 'RegisteredStatusNotifierItems'))[0]

    try:
        subprocess.run(['python3', str(args.harness), '--run', str(directory), 'start', '--bundle', str(args.bundle.resolve()), '--display', args.display, '--no-vnc'], check=True, capture_output=True, text=True, timeout=90)
        started = True
        session = call('status')['processes']['session']
        env = dict(entry.decode().split('=', 1) for entry in Path(f'/proc/{session}/environ').read_bytes().split(b'\0') if b'=' in entry)
        bus = connect(env['DBUS_SESSION_BUS_ADDRESS'])
        children = [int(p) for p in Path(f'/proc/{session}/task/{session}/children').read_text().split()]
        pids = [pid for pid in children if 'knave-shell' in Path(f'/proc/{pid}/cmdline').read_text()]
        resources['empty_before'] = {pid: sample(pid) for pid in pids}
        pump(2)
        resources['empty_after'] = {pid: sample(pid) for pid in pids}
        call('record-start', 'applets-demo')
        alpha = Fixture(env['DBUS_SESSION_BUS_ADDRESS'], 'Chat', (240, 90, 100))
        fixtures.append(alpha)
        beta = Fixture(env['DBUS_SESSION_BUS_ADDRESS'], 'Sync', (60, 190, 130), True)
        fixtures.append(beta)
        alpha.register()
        wait(lambda: len(registered()) == 2, 'registration deduplicated')
        checks.append('duplicate registration remains two items')
        color(1172, alpha.color, 'first icon immediately left of second icon')
        color(1204, beta.color, 'second icon immediately left of notification bell')
        shot('applets-in-bar')
        gamma = Fixture(env['DBUS_SESSION_BUS_ADDRESS'], 'Backup', (100, 90, 220), freedesktop=True, name='org.knave.TrayFixture')
        fixtures.append(gamma)
        color(1204, gamma.color, 'freedesktop item registered by service name renders')
        invoke(gamma.bus, 'org.freedesktop.DBus', '/org/freedesktop/DBus', 'org.freedesktop.DBus', 'ReleaseName', '(s)', ('org.knave.TrayFixture',))
        wait(lambda: len(registered()) == 2, 'service release unregisters live connection')
        color(1204, beta.color, 'service name release removes item before connection exit')
        click(1172, 18)
        wait(lambda: any(a[0] == 'Activate' for a in alpha.actions), 'left click activates')
        click(1172, 18, 2)
        wait(lambda: any(a[0] == 'SecondaryActivate' for a in alpha.actions), 'middle click activates secondary action')
        click(1172, 18, 4)
        wait(lambda: any(a[0] == 'Scroll' for a in alpha.actions), 'scroll action delivered')
        checks.append('left, middle and scroll actions reach the app')
        alpha.change(color=(245, 190, 60))
        color(1172, alpha.color, 'live icon update replaces pixels')
        alpha.change(status='NeedsAttention')
        color(1172, (70, 130, 240), 'attention icon updates without polling')
        alpha.change(status='Passive')
        wait(lambda: pixel(1172, 18) != (70,130,240), 'passive icon hidden')
        color(1204, beta.color, 'passive icon removal preserves bell spacing')
        alpha.change(status='Active')
        color(1172, alpha.color, 'active status restores icon')
        key('Super_L')
        wait(lambda: call('snapshot')['payload']['overview_visible'], 'overview opens')
        color(1172, alpha.color, 'Overview renders same live applets')
        shot('applets-in-overview')
        click(1172, 18)
        wait(lambda: sum(a[0] == 'Activate' for a in alpha.actions) == 2, 'Overview activation')
        checks.append('Overview applet activates the same app')
        key('Escape')
        click(1204, 18)
        pump(1)
        shot('applet-menu')
        # Popup is clamped to the right edge: x=1000..1280, header y=48..76.
        click(1100, 94)
        wait(lambda: any(a[0] == 'Event' and a[1][0] == 1 for a in beta.actions), 'menu-only applet action')
        checks.append('menu-only left click opens exported menu and dispatches action')
        click(1172, 18, 3)
        pump(1)
        before = len(alpha.actions)
        click(1100, 126)
        assert len(alpha.actions) == before, 'disabled menu action must not dispatch'
        checks.append('disabled menu action remains inactive')
        click(1100, 158)
        pump(0.6)
        shot('applet-submenu')
        click(1100, 94)
        wait(lambda: any(a[0] == 'Event' and a[1][0] == 43 for a in alpha.actions), 'submenu action')
        checks.append('right click opens exported menu and nested action reaches app')
        # Opening/closing repeatedly must reap popups and retain a bounded worker set.
        for _ in range(5):
            click(1204,18)
            pump(0.2)
            key('Escape')
        resources['normal'] = {pid: sample(pid) for pid in pids}
        for index in range(30):
            alpha.change(color=(index, 120, 180))
            pump(0.05)
        color(1172, alpha.color, 'icon burst converges to latest state')
        resources['stress'] = {pid: sample(pid) for pid in pids}
        pump(2)
        resources['idle_before'] = {pid: sample(pid) for pid in pids}
        pump(2)
        resources['idle_after'] = {pid: sample(pid) for pid in pids}
        for pid in pids:
            assert resources['stress'][pid]['threads'] <= resources['empty_after'][pid]['threads'] + 1
            assert resources['idle_after'][pid]['ticks'] - resources['idle_before'][pid]['ticks'] <= 10
        checks.append('idle CPU and worker counts remain bounded after menu and icon stress')
        alpha.bus.close_sync(None)
        wait(lambda: len(registered()) == 1, 'owner exit removes icon')
        color(1204, beta.color, 'owner exit removes only its own icon')
        beta.bus.close_sync(None)
        wait(lambda: len(registered()) == 0, 'last owner removed')
        wait(lambda: pixel(1204,18) != beta.color, 'empty tray disappears')
        checks.append('bus owner exit removes registrations and pixels')
        click(1252,18)
        wait(lambda: json.loads(invoke(bus,'org.knave.Notifications','/org/knave/Notifications','org.knave.Notifications','Snapshot')[0])['center_open'], 'bell remains clickable')
        key('Escape')
        checks.append('notification bell remains interactive beside applets')
        video = call('record-stop')
        report = dict(passed=True, checks=checks, resources=resources, video=video)
        (directory / 'tray-result.json').write_text(json.dumps(report,indent=2)+'\n')
        print(json.dumps(report,indent=2))
    finally:
        for fixture in fixtures:
            if not fixture.bus.is_closed(): fixture.bus.close_sync(None)
        if started: call('stop')


if __name__ == '__main__':
    main()
