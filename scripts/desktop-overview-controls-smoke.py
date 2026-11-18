#!/usr/bin/env python3
"""Verify overview window controls and save input, screenshots, video and resource evidence."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('harness', ROOT / 'scripts/desktop-harness.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def resources(root):
    stats = {}
    for path in Path('/proc').iterdir():
        if not path.name.isdigit():
            continue
        try:
            fields = (path / 'stat').read_text().rsplit(') ', 1)[1].split()
            stats[int(path.name)] = (int(fields[1]), int(fields[11]) + int(fields[12]))
        except (OSError, ValueError, IndexError):
            continue
    pids = {root}
    while True:
        children = {pid for pid, (parent, _) in stats.items() if parent in pids}
        if children <= pids:
            break
        pids |= children
    result = dict(at=time.monotonic(), pids=sorted(pids), processes=len(pids), rss_kib=0, threads=0,
                  fds=0, cpu_ticks=0, context_switches=0)
    for pid in pids:
        try:
            path = Path('/proc') / str(pid)
            status = dict(line.split(':', 1) for line in (path / 'status').read_text().splitlines())
            result['rss_kib'] += int(status.get('VmRSS', '0').split()[0])
            result['threads'] += int(status['Threads'])
            result['fds'] += len(list((path / 'fd').iterdir()))
            result['cpu_ticks'] += stats[pid][1]
            result['context_switches'] += sum(int(status[key]) for key in
                                             ('voluntary_ctxt_switches', 'nonvoluntary_ctxt_switches'))
        except (OSError, KeyError):
            continue
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='95')
    parser.add_argument('--app', choices=('foliate', 'evince'), default='foliate')
    args = parser.parse_args()
    run = args.run.resolve()
    app_id = {'foliate': 'com.github.johnfactotum.Foliate', 'evince': 'org.gnome.Evince'}[args.app]
    command = [args.app]
    if args.app == 'evince':
        document = Path('/usr/share/doc/speex/manual.pdf')
        if not document.is_file():
            raise RuntimeError('Evince verification requires /usr/share/doc/speex/manual.pdf')
        command.append(str(document))
    checks, samples = [], {}
    started = False

    def call(op, *values):
        return harness.request(run, op, list(values))

    def snapshot():
        return call('snapshot')['payload']

    def windows():
        return snapshot()['windows']

    def window():
        return next((item for item in windows() if item['app_id'] == app_id), None)

    def wait(predicate, label):
        deadline = time.monotonic() + 20
        while time.monotonic() < deadline:
            if predicate():
                checks.append(label)
                return
            time.sleep(.1)
        raise AssertionError(label)

    def capture(name):
        hover(640, 705)
        return Path(call('screenshot', name)['screenshot'])

    def hover(x, y):
        subprocess.run(['xdotool', 'mousemove', str(x), str(y)], env=input_env, check=True)
        time.sleep(.18)

    def control(slot, screenshot=None):
        state = snapshot()
        assert state['overview_visible']
        layout = state['overview_layout']
        target = next(item for item in layout['windows'] if item['id'] == window()['id'])
        # Main preview interior in the 1280x800 harness, with no minimized windows.
        px, py, width, height = 271, 244, 738, 414
        scale = max(width / layout['width'], height / layout['height'])
        ox = px + (width - scale * layout['width']) / 2
        oy = py + (height - scale * layout['height']) / 2
        left = max(px, ox + target['x'] * scale)
        right = min(px + width, ox + (target['x'] + target['width']) * scale)
        top = max(py, oy + target['y'] * scale)
        bottom = min(py + height, oy + (target['y'] + target['height']) * scale)
        x = round(max(px + 4, right - 128)) + 17 + slot * 30
        y = round(min(top + 6, py + height - 36)) + 16
        hover(round((left + right) / 2), round((top + bottom) / 2))
        hover(x, y)
        if screenshot:
            call('screenshot', screenshot)
        call('click', x, y, 1)

    try:
        with tempfile.TemporaryDirectory(prefix='knave-overview-config-') as directory:
            config = Path(directory) / 'config.toml'
            config.write_text('[[session.startup]]\ncommand = ' + json.dumps(command) + '\n')
            subprocess.run(['python3', str(ROOT / 'scripts/desktop-harness.py'), '--run', str(run),
                            'start', '--profile', 'debug', '--no-vnc', '--display', args.display,
                            '--config', str(config)], check=True, capture_output=True, text=True, timeout=60)
        started = True
        status = call('status')
        session_pid = status['processes']['session']
        compositor_pid = next(pid for pid in resources(session_pid)['pids']
                              if b'--internal\0compositor' in Path(f'/proc/{pid}/cmdline').read_bytes())
        input_env = dict(os.environ, DISPLAY=status['display'], XAUTHORITY=str(run / 'Xauthority'))
        wait(lambda: window() is not None, 'application opens')
        time.sleep(1)
        capture('gtk-client-header')
        samples['idle_before'] = resources(compositor_pid)
        time.sleep(2)
        samples['idle_after'] = resources(compositor_pid)
        call('record-start', 'overview-window-controls')
        call('key', 'Super_L')
        wait(lambda: snapshot()['overview_visible'], 'overview opens')
        capture('unhovered')
        control(0, 'hover-float')
        wait(lambda: window()['floating'], 'float hovered window')
        control(0, 'hover-tile')
        wait(lambda: not window()['floating'], 'tile hovered window')
        control(2, 'hover-maximize')
        wait(lambda: window()['maximized'], 'maximize hovered window')
        control(2, 'hover-restore')
        wait(lambda: not window()['maximized'], 'restore hovered window')
        control(1, 'hover-minimize')
        wait(lambda: window()['minimized'], 'minimize hovered window')
        time.sleep(.25)
        assert snapshot()['overview_visible']
        capture('minimized')
        call('key', 'Escape')
        wait(lambda: not snapshot()['overview_visible'], 'overview can close after minimizing')
        call('key', 'Super_L+Shift_L+m')
        wait(lambda: not window()['minimized'], 'restore minimized window')
        call('key', 'Super_L')
        wait(lambda: snapshot()['overview_visible'], 'overview reopens')
        samples['normal'] = resources(compositor_pid)
        samples['stress_before'] = resources(compositor_pid)
        for _ in range(5):
            control(0)
            wait(lambda: window()['floating'], 'stress float')
            control(0)
            wait(lambda: not window()['floating'], 'stress tile')
        samples['stress_after'] = resources(compositor_pid)
        time.sleep(2)
        samples['recovery'] = resources(compositor_pid)
        control(3, 'hover-close')
        wait(lambda: window() is None, 'close hovered window')
        assert snapshot()['overview_visible']
        checks.append('window actions leave overview open')
        video = call('record-stop')
        hz = os.sysconf('SC_CLK_TCK')
        rates = {}
        for label, before, after in [('idle', 'idle_before', 'idle_after'),
                                      ('stress', 'stress_before', 'stress_after')]:
            a, b = samples[before], samples[after]
            rates[label] = dict(cpu_percent=100 * (b['cpu_ticks'] - a['cpu_ticks']) / hz / (b['at'] - a['at']),
                                context_switches_per_second=(b['context_switches'] - a['context_switches']) / (b['at'] - a['at']))
        for key in ('processes', 'threads', 'fds'):
            if samples['recovery'][key] > samples['normal'][key]:
                raise AssertionError(f'{key} did not recover after repeated toggles')
        checks.append('compositor process, thread and descriptor counts do not grow after stress')
        (run / 'resource-samples.json').write_text(json.dumps(dict(samples=samples, rates=rates), indent=2) + '\n')
        result = dict(passed=True, app=args.app, checks=checks, **video,
                      limits='Nested software rendering; context switches are not exact wakeup counts; no pre-change performance comparison.')
        (run / 'overview-controls-result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result, indent=2))
    finally:
        if started:
            call('stop')


if __name__ == '__main__':
    main()
