#!/usr/bin/env python3
"""Exercise workspace presentation against frozen binaries in the private Winit harness."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import shlex
import subprocess
import time



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='106')
    parser.add_argument('--baseline', action='store_true', help='measure the preceding build with identical workloads')
    args = parser.parse_args()
    bundle, directory = args.bundle.resolve(), args.run.resolve()
    harness_path = bundle / 'tools/desktop-harness.py'
    spec = importlib.util.spec_from_file_location('motion_harness', harness_path)
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    checks, samples = [], []

    def call(command, *values):
        return harness.request(directory, command, list(values))

    def check(condition, label):
        if not condition:
            raise AssertionError(label)
        checks.append(label)

    def query():
        value = call('gesture', '{"op":"query"}')
        samples.append(value)
        return value

    def wait(predicate, label):
        end = time.monotonic() + 5
        while time.monotonic() < end:
            value = query()
            if predicate(value):
                return value
            time.sleep(.01)
        raise AssertionError(label)

    def settle(workspace):
        return wait(lambda value: value['active_workspace'] == workspace
                    and not value.get('motion') and value['offset'] is None
                    and not value.get('motion_timer', False), 'motion did not settle')

    def control(*values):
        return subprocess.run([str(bundle / 'bin/knavectl'), *values], env=env,
                              capture_output=True, text=True, check=True, timeout=10)

    def select(workspace):
        control('dispatch', 'workspace', str(workspace))

    def screenshot(name):
        return Path(call('screenshot', name)['screenshot'])

    def proc(pid):
        base = Path('/proc') / str(pid)
        fields = (base / 'stat').read_text().split(') ', 1)[1].split()
        status = dict(line.split(':', 1) for line in (base / 'status').read_text().splitlines() if ':' in line)
        return dict(ticks=int(fields[11]) + int(fields[12]), rss_kib=int(status['VmRSS'].split()[0]),
                    threads=int(status['Threads']), fds=len(list((base / 'fd').iterdir())),
                    context_switches=int(status['voluntary_ctxt_switches']) + int(status['nonvoluntary_ctxt_switches']),
                    time=time.monotonic())

    started = False
    try:
        subprocess.run(['python3', str(harness_path), '--run', str(directory), 'start',
                        '--bundle', str(bundle), '--display', args.display, '--no-vnc', '--gesture-test'],
                       capture_output=True, text=True, check=True, timeout=60)
        started = True
        manifest = json.loads((directory / 'manifest.json').read_text())
        runtime = Path(manifest['runtime'])
        socket = next(runtime.glob('knave/desktop-*.sock'))
        env = dict(os.environ, XDG_RUNTIME_DIR=str(runtime), KNAVE_SOCKET=str(socket))
        colors = [(28, 65, 100), (110, 45, 65)]
        for workspace, color in enumerate(colors, 1):
            select(workspace)
            settle(workspace)
            call('key', 'Super_L+Return')
            deadline = time.monotonic() + 10
            while time.monotonic() < deadline:
                state = call('snapshot')['payload']
                if any(w['workspace'] == workspace and w['app_id'] == 'kitty' and w['focused'] for w in state['windows']):
                    break
                time.sleep(.05)
            else:
                raise AssertionError('terminal did not launch')
            rgb = '/'.join(f'{value:02x}' for value in color)
            text = f'\\033]11;rgb:{rgb}\\007\\033[?25l\\033[2J\\033[H\\n    WORKSPACE {workspace}\\n\\n    Workspace motion primitives\\n    Live compositor scenes / stationary panels\\n'
            call('type', 'printf ' + shlex.quote(text))
            call('key', 'Return')
            time.sleep(.3)
        select(1)
        settle(1)
        first = screenshot('workspace-one')
        call('record-start', 'workspace-motion-demo')
        time.sleep(.3)

        # Real keyboard path, with state committed before presentation completes.
        call('key', 'Super_L+2')
        middle = query()
        intermediate = screenshot('keyboard-mid-transition')
        check(middle['active_workspace'] == 2, 'keyboard changes authoritative workspace immediately')
        if not args.baseline:
            incoming = next(p['x'] for p in middle['motion'] if p['workspace'] == 2)
            check(0 < incoming < 1280 and middle['motion_timer'], 'keyboard switch has intermediate displacement')
            def pixels(path, geometry):
                return subprocess.check_output(['magick', str(path), '-crop', geometry,
                                                '+repage', '-depth', '8', 'rgb:-'], timeout=10)
            row = pixels(intermediate, '1240x1+20+450')
            counts = [sum(max(abs(a-b) for a, b in zip(row[x:x+3], color)) < 4
                          for x in range(0, len(row), 3)) for color in colors]
            check(min(counts) > 20, 'intermediate pixels contain both source and destination windows')
            check(pixels(first, '400x36+600+0') == pixels(intermediate, '400x36+600+0'),
                  'panel remains stationary during command animation')
        state = call('snapshot')['payload']
        check(any(w['focused'] and w['workspace'] == 2 for w in state['windows']), 'keyboard focus follows logical destination')
        settle(2)
        screenshot('workspace-two')
        time.sleep(.3)

        # Direct commands, reversal and a third target while motion is in flight.
        select(1)
        initial = query()
        if not args.baseline:
            check(next(p['x'] for p in initial['motion'] if p['workspace'] == 1) < 0, 'direct selection slides in numeric direction')
        time.sleep(.035)
        select(2)
        reverse = query()
        if not args.baseline:
            check(len(reverse['motion']) == 2 and reverse['active_workspace'] == 2, 'reversal reuses the two live tracks')
        select(3)
        third = query()
        if not args.baseline:
            check(len(third['motion']) == 3 and third['active_workspace'] == 3, 'rapid third target preserves outgoing tracks')
        settle(3)
        for workspace in range(4, 11):
            select(workspace)
            settle(workspace)
        call('key', 'Super_L+Right')
        advanced = query()
        if not args.baseline:
            check(next(p['x'] for p in advanced['motion'] if p['workspace'] == 11) > 0, 'forward navigation creates workspace eleven')
        settle(11)
        call('key', 'Super_L+Left')
        returned = query()
        if not args.baseline:
            check(next(p['x'] for p in returned['motion'] if p['workspace'] == 10) < 0, 'backward navigation returns to workspace ten')
        settle(10)
        select(1)
        settle(1)
        call('key', 'Super_L+Left')
        check(query()['active_workspace'] == 1 and not query().get('motion_timer', False), 'backward navigation stops at workspace one')

        # Click the actual bar's workspace chip, then interrupt with a window click.
        call('click', '192', '18', '1')
        check(query()['active_workspace'] == 2, 'bar click switches workspace')
        settle(2)
        select(1)
        call('click', '640', '450', '1')
        check(not query().get('motion_timer', False), 'pointer click finishes motion before client delivery')
        select(2)
        call('click', '640', '450', '5')
        check(not query().get('motion_timer', False), 'scroll finishes motion before client delivery')
        select(1)
        call('key', 'Super_L')
        check(not query().get('motion_timer', False), 'overview entry cancels desktop animation')
        deadline = time.monotonic() + 5
        while not call('snapshot')['payload']['overview_visible']:
            if time.monotonic() > deadline:
                raise AssertionError('overview did not open')
            time.sleep(.02)
        time.sleep(.3)  # Wait for the layer buffer and keyboard focus, beyond the logical toggle.
        call('key', 'Escape')
        deadline = time.monotonic() + 5
        while call('snapshot')['payload']['overview_visible']:
            if time.monotonic() > deadline:
                raise AssertionError('overview did not close')
            time.sleep(.02)
        time.sleep(.2)
        select(2)
        control('reload')
        check(not query().get('motion_timer', False), 'reload cancels timed presentation')
        settle(2)
        select(1)
        settle(1)

        session = call('status')['processes']['session']
        children, villain = [session], None
        while children:
            pid = children.pop()
            base = Path('/proc') / str(pid)
            if (base / 'comm').read_text().strip() == 'villain':
                villain = pid
                break
            children.extend(map(int, (base / 'task' / str(pid) / 'children').read_text().split()))
        check(villain is not None, 'isolated compositor identified')
        # Warm all workspace caches before measuring growth.
        for workspace in range(1, 11):
            select(workspace)
            settle(workspace)
        select(1)
        settle(1)
        time.sleep(.5)
        idle_start = proc(villain)
        time.sleep(2)
        idle_end = proc(villain)
        normal_start = proc(villain)
        for workspace in [2, 1, 2, 1]:
            select(workspace)
            if not args.baseline:
                check(query()['motion_timer'], f'normal workload animates switch to {workspace}')
            settle(workspace)
            time.sleep(.05)
        normal_end = proc(villain)
        stress_start = proc(villain)
        for index in range(100):
            select((index + 1) % 10 + 1)
            if not args.baseline:
                value = query()
                check(0 < len(value['motion']) <= 10 and value['motion_timer'], f'track bound after stress switch {index + 1}')
        settle(1)
        stress_end = proc(villain)
        time.sleep(2)
        after = proc(villain)
        check((after['threads'], after['fds']) == (idle_end['threads'], idle_end['fds']), 'stress leaves threads and descriptors unchanged')
        check(after['rss_kib'] - idle_end['rss_kib'] < 32768, 'stress memory growth below 32 MiB after cache warmup')
        if not args.baseline:
            check(not query()['motion_timer'], 'no animation timer remains at idle')
        screenshot('settled-after-stress')
        video = call('record-stop')
        resources = dict(idle_start=idle_start, idle_end=idle_end, normal_start=normal_start, normal_end=normal_end,
                         stress_start=stress_start, stress_end=stress_end, after_idle=after)
        report = dict(passed=True, baseline=args.baseline, checks=checks, samples=samples, resources=resources,
                      bundle=str(bundle), bundle_sha256=manifest['bundle_sha256'], **video)
        (directory / 'workspace-motion-result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({key: value for key, value in report.items() if key not in ('samples', 'checks')}, indent=2))
    finally:
        if started:
            call('stop')


if __name__ == '__main__':
    main()
