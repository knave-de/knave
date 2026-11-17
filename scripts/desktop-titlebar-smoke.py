#!/usr/bin/env python3
"""Verify server titlebars and save input, screenshots, video and resource evidence."""
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
    sequence = 0

    def call(op, *values):
        return harness.request(run, op, list(values))

    def windows():
        return call('snapshot')['payload']['windows']

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
        subprocess.run(['xdotool', 'mousemove', '640', '780'], env=input_env, check=True)
        time.sleep(.15)
        return Path(call('screenshot', name)['screenshot'])

    def pixels(path):
        return subprocess.check_output(['magick', str(path), '-alpha', 'off', '-depth', '8', 'rgb:-'])

    def control(slot):
        nonlocal sequence
        sequence += 1
        image = pixels(capture(f'control-{sequence:02d}'))
        rows = []
        for y in range(36, 800):
            row = image[y * 1280 * 3:(y + 1) * 1280 * 3]
            if row.count(background) >= 200:
                rows.append(y)
        groups = []
        for y in rows:
            if not groups or y != groups[-1][-1] + 1:
                groups.append([])
            groups[-1].append(y)
        bars = [group for group in groups if 15 <= len(group) <= 35]
        if len(bars) != 1:
            raise AssertionError(f'expected one server titlebar, found {len(bars)}')
        y = bars[0][len(bars[0]) // 2]
        row = image[y * 1280 * 3:(y + 1) * 1280 * 3]
        right = row.rfind(background) // 3 + 1
        call('click', right - 12 - slot * 24, y, 1)

    try:
        with tempfile.TemporaryDirectory(prefix='knave-titlebar-config-') as directory:
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
        initial = pixels(capture('initial'))
        background = initial[(66 * 1280 + 500) * 3:(66 * 1280 + 500) * 3 + 3]
        samples['idle_before'] = resources(compositor_pid)
        time.sleep(2)
        samples['idle_after'] = resources(compositor_pid)
        call('record-start', 'titlebar-actions')
        control(1)
        wait(lambda: window()['maximized'], 'maximize control')
        capture('maximized')
        control(1)
        wait(lambda: not window()['maximized'], 'restore control')
        control(2)
        wait(lambda: window()['floating'], 'float control')
        capture('floating')
        control(2)
        wait(lambda: not window()['floating'], 'tile control')
        control(3)
        wait(lambda: window()['minimized'], 'minimize control')
        call('key', 'Super_L+Shift_L+m')
        wait(lambda: not window()['minimized'], 'restore minimized window')
        call('key', 'F11')
        wait(lambda: window()['fullscreen'], 'fullscreen entry')
        capture('fullscreen')
        call('key', 'F11')
        wait(lambda: not window()['fullscreen'], 'fullscreen exit')
        samples['normal'] = resources(compositor_pid)
        samples['stress_before'] = resources(compositor_pid)
        for _ in range(5):
            control(2)
            wait(lambda: window()['floating'], 'stress float')
            control(2)
            wait(lambda: not window()['floating'], 'stress tile')
        samples['stress_after'] = resources(compositor_pid)
        time.sleep(2)
        samples['recovery'] = resources(compositor_pid)
        capture('restored')
        control(0)
        wait(lambda: window() is None, 'close control')
        video = call('record-stop')
        hz = os.sysconf('SC_CLK_TCK')
        rates = {}
        for label, before, after in [('idle', 'idle_before', 'idle_after'),
                                      ('stress', 'stress_before', 'stress_after')]:
            a, b = samples[before], samples[after]
            rates[label] = dict(cpu_percent=100 * (b['cpu_ticks'] - a['cpu_ticks']) / hz / (b['at'] - a['at']),
                                context_switches_per_second=(b['context_switches'] - a['context_switches']) / (b['at'] - a['at']))
        (run / 'resource-samples.json').write_text(json.dumps(dict(samples=samples, rates=rates), indent=2) + '\n')
        for key in ('processes', 'threads', 'fds'):
            if samples['recovery'][key] > samples['normal'][key]:
                raise AssertionError(f'{key} did not recover after repeated toggles')
        checks.append('compositor process, thread and descriptor counts do not grow after stress')
        (run / 'resource-samples.json').write_text(json.dumps(dict(samples=samples, rates=rates), indent=2) + '\n')
        result = dict(passed=True, app=args.app, checks=checks, **video,
                      limits='Nested software rendering; context switches are not exact wakeup counts; no pre-change performance comparison.')
        (run / 'titlebar-result.json').write_text(json.dumps(result, indent=2) + '\n')
        print(json.dumps(result, indent=2))
    finally:
        if started:
            call('stop')


if __name__ == '__main__':
    main()
