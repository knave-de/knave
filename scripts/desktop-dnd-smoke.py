#!/usr/bin/env python3
"""Verify native cross-client file drops and drag-icon pixels in the private harness.

Requires PyGObject with GTK 3 in addition to the harness dependencies.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time

HARNESS = Path(__file__).with_name('desktop-harness.py')


def fixture(role, directory):
    import gi
    gi.require_version('Gtk', '3.0')
    gi.require_version('Gdk', '3.0')
    from gi.repository import Gdk, GdkPixbuf, Gtk
    window = Gtk.Window(title=f'DnD {role}')
    window.set_default_size(440, 400)
    area = Gtk.EventBox()
    area.add(Gtk.Label(label=f'DnD {role}'))
    targets = [Gtk.TargetEntry.new('text/uri-list', 0, 0)]

    def log(message):
        with (directory / f'{role}.jsonl').open('a') as stream:
            stream.write(json.dumps(message) + '\n')

    if role == 'source':
        area.drag_source_set(Gdk.ModifierType.BUTTON1_MASK, targets, Gdk.DragAction.COPY)

        def begin(widget, context):
            icon = GdkPixbuf.Pixbuf.new(GdkPixbuf.Colorspace.RGB, True, 8, 64, 48)
            icon.fill(0xff00ffff)
            Gtk.drag_set_icon_pixbuf(context, icon, 12, 8)
            log({'event': 'begin'})

        def data_get(widget, context, data, info, stamp):
            data.set_uris([(directory / 'payload.txt').as_uri()])

        area.connect('drag-begin', begin)
        area.connect('drag-data-get', data_get)
        area.connect('drag-end', lambda *_: log({'event': 'end'}))
    else:
        area.drag_dest_set(Gtk.DestDefaults.ALL, targets, Gdk.DragAction.COPY)

        def received(widget, context, x, y, data, info, stamp):
            log({'event': 'received', 'uris': data.get_uris()})
            Gtk.drag_finish(context, True, False, stamp)

        area.connect('drag-data-received', received)
    window.add(area)
    window.show_all()
    Gtk.main()


def is_compositor(proc, runtime):
    try:
        return (b'--internal\x00compositor' in (proc / 'cmdline').read_bytes()
                and ('XDG_RUNTIME_DIR=' + runtime).encode() + b'\0' in (proc / 'environ').read_bytes())
    except OSError:
        return False


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='105')
    parser.add_argument('--vnc-port', default='6005')
    parser.add_argument('--fixture', choices=['source', 'target'], help=argparse.SUPPRESS)
    args = parser.parse_args()
    directory = args.run.resolve()
    if args.fixture:
        fixture(args.fixture, directory)
        return
    import gi
    gi.require_version("GdkPixbuf", "2.0")
    from gi.repository import GdkPixbuf
    command = [sys.executable, str(HARNESS), '--run', str(directory)]

    def call(*values):
        return json.loads(subprocess.run(command + list(values), check=True,
                          capture_output=True, text=True, timeout=60).stdout)

    def wait(predicate):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(.1)
        raise AssertionError('timed out waiting for desktop/drag state')

    def events(role):
        path = directory / f'{role}.jsonl'
        return [json.loads(line) for line in path.read_text().splitlines()] if path.exists() else []

    started, children = False, []
    try:
        call('start', '--profile', 'debug', '--display', args.display,
             '--vnc-port', args.vnc_port)
        started = True
        manifest = json.loads((directory / 'manifest.json').read_text())
        runtime = Path(manifest['runtime'])
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), GDK_BACKEND='wayland',
                   WAYLAND_DISPLAY=next(runtime.glob('wayland-*')).name,
                   DISPLAY=f':{args.display}', XAUTHORITY=str(directory / 'Xauthority'))
        env.pop('DBUS_SESSION_BUS_ADDRESS', None)
        (directory / 'payload.txt').write_text('KNAVE_DND_OK\n')
        for role in ['source', 'target']:
            with (directory / f'{role}.log').open('w') as log:
                children.append(subprocess.Popen([sys.executable, str(Path(__file__).resolve()),
                    '--run', str(directory), '--fixture', role], env=env, stdout=log, stderr=log))
            wait(lambda: any(w['title'] == f'DnD {role}'
                             for w in call('snapshot')['payload']['windows']))

        def x(*values):
            subprocess.run(['xdotool', *map(str, values)], env=env, check=True, timeout=10)

        def pixels(name):
            path = call('screenshot', name)['screenshot']
            image = GdkPixbuf.Pixbuf.new_from_file(path)
            data = image.get_pixels()
            stride, channels = image.get_rowstride(), image.get_n_channels()
            return [(x, y) for y in range(image.get_height()) for x in range(image.get_width())
                    if (lambda i: data[i] > 240 and data[i + 1] < 15 and data[i + 2] > 240)(
                        y * stride + x * channels)]

        def begin():
            x('mousemove', 300, 420)
            x('mousedown', 1)
            time.sleep(.15)
            for px in [320, 350]:
                x('mousemove', px, 420)
                time.sleep(.15)

        def cross():
            for px in [450, 550, 650, 750, 850, 940]:
                x('mousemove', px, 420)
                time.sleep(.15)

        compositor = next(p for p in Path('/proc').iterdir()
                          if p.name.isdigit() and p.exists()
                          and is_compositor(p, str(runtime)))

        def resources():
            status = dict(line.split(':', 1) for line in (compositor / 'status').read_text().splitlines())
            stat = (compositor / 'stat').read_text().split(') ', 1)[1].split()
            return {'rss_kib': int(status['VmRSS'].split()[0]),
                    'threads': int(status['Threads']),
                    'fds': len(list((compositor / 'fd').iterdir())),
                    'cpu_ticks': int(stat[11]) + int(stat[12]),
                    'voluntary_switches': int(status['voluntary_ctxt_switches'])}

        def idle_sample():
            before = resources()
            start = time.monotonic()
            time.sleep(.6)
            after = resources()
            seconds = time.monotonic() - start
            after['cpu_percent'] = ((after['cpu_ticks'] - before['cpu_ticks']) /
                                    os.sysconf('SC_CLK_TCK') / seconds * 100)
            after['voluntary_switches_per_second'] = (
                after['voluntary_switches'] - before['voluntary_switches']) / seconds
            return after

        samples = {'idle_before': idle_sample()}
        call('record-start', 'drag-icon-demo')
        assert not pixels('before'), 'icon present before drag'
        begin()
        samples['drag_active'] = resources()
        source_pixels = pixels('source-icon')
        assert len(source_pixels) > 2500, 'source drag icon missing'
        # The GTK hotspot is (12, 8), expressed as a negative surface attach offset.
        assert min(x for x, _ in source_pixels) == 338, 'icon x offset incorrect'
        assert min(y for _, y in source_pixels) == 412, 'icon y offset incorrect'
        cross()
        target_pixels = pixels('target-icon')
        assert len(target_pixels) > 2500, 'icon disappeared across windows'
        assert min(x for x, _ in target_pixels) == 928, 'icon did not follow pointer'
        x('mouseup', 1)
        wait(lambda: len(events('target')) == 1)
        assert events('target')[0]['uris'] == [(directory / 'payload.txt').as_uri()]
        time.sleep(.3)
        assert not pixels('after-drop'), 'icon survived completed drop'
        begin()
        assert len(pixels('cancel-icon')) > 2500
        x('mousemove', 700, 18)  # The shell bar does not accept this drop.
        time.sleep(.3)
        x('mouseup', 1)
        wait(lambda: len(events('source')) == 4)
        assert len(events('target')) == 1, 'cancelled drop delivered data'
        time.sleep(.3)
        assert not pixels('after-cancel'), 'icon survived rejected drop'
        call('record-stop')
        for _ in range(10):
            count = len(events('source'))
            begin()
            x('mousemove', 700, 18)
            time.sleep(.15)
            x('mouseup', 1)
            wait(lambda: len(events('source')) == count + 2)
        samples['idle_after_stress'] = idle_sample()
        assert samples['idle_after_stress']['threads'] == samples['idle_before']['threads']
        assert samples['idle_after_stress']['fds'] <= samples['idle_before']['fds'] + 2
        assert not pixels('after-stress'), 'icon survived repeated rejected drops'
        call('record-start', 'disconnect-demo')
        begin()
        assert len(pixels('disconnect-icon')) > 2500
        children[0].terminate()
        children[0].wait(timeout=10)
        time.sleep(.3)
        assert not pixels('after-disconnect'), 'icon survived source disconnect'
        x('mouseup', 1)
        disconnect_video = call('record-stop')
        report = {'passed': True, 'resources': samples, 'disconnect_video': disconnect_video, 'checks': ['separate native Wayland clients',
                  'file URI delivered', 'icon visible at client offset', 'icon follows pointer',
                  'icon removed on drop, rejection and source disconnect'],
                  'video': str(directory / 'drag-icon-demo.mp4')}
        (directory / 'dnd-result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))
    finally:
        for child in children:
            if child.poll() is None:
                child.terminate()
            child.wait(timeout=10)
        if started:
            call('stop')


if __name__ == '__main__':
    main()
