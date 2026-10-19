#!/usr/bin/env python3
"""Assert synthetic swipe behavior and rendered pixels against an immutable bundle."""
import argparse
import importlib.util
import json
import os
import re
import signal
from pathlib import Path
import shlex
import subprocess
import time


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--bundle', type=Path, required=True)
    parser.add_argument('--display', default='95')
    parser.add_argument('--skip-empty', action='store_true', help='exercise optional sparse workspace cycling')
    args = parser.parse_args()
    directory = args.run.resolve()
    bundle = args.bundle.resolve()
    harness_path = Path(__file__).with_name('desktop-harness.py')
    spec = importlib.util.spec_from_file_location('gesture_harness', harness_path)
    harness = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(harness)
    checks, samples = [], []

    def call(command, *values):
        return harness.request(directory, command, list(values))

    def gesture(op, **values):
        result = call('gesture', json.dumps(dict(op=op, **values)))
        samples.append(dict(op=op, request=values, **result))
        return result

    def check(condition, label):
        if not condition:
            raise AssertionError(label)
        checks.append(label)

    def wait(predicate, label):
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            value = call('snapshot')['payload']
            if predicate(value):
                return value
            time.sleep(0.05)
        raise AssertionError(label)

    def active(number):
        wait(lambda state: any(w['workspace'] == number and w['active'] for w in state['workspaces']), f'workspace {number}')

    def settle(number):
        deadline = time.monotonic() + 2
        while time.monotonic() < deadline:
            result = gesture('query')
            if result['offset'] is None:
                check(result['active_workspace'] == number, f'settled on workspace {number}')
                return
            time.sleep(0.025)
        raise AssertionError('settle timer did not finish')

    def key(*keys):
        call('key', *keys)

    def screenshot(name):
        time.sleep(0.1)
        return Path(call('screenshot', name)['screenshot'])

    def rgb(path, geometry):
        return subprocess.check_output(['magick', str(path), '-crop', geometry, '+repage', '-depth', '8', 'rgb:-'], timeout=10)

    def begin():
        gesture('begin', fingers=3)

    def travel(delta, steps=10):
        for _ in range(steps):
            result = gesture('update', x=delta, y=0)
            time.sleep(0.055)
        return result

    def terminal(number, color):
        key(f'Super_L+{number}')
        active(number)
        key('Super_L+Return')
        wait(lambda state: any(w['workspace'] == number and w['app_id'] == 'kitty' and w['focused'] for w in state['windows']), 'Kitty launch')
        text = f'\\033[48;2;{color}m\\033[2J\\033[H\\n    WORKSPACE {number}\\n\\n    Three-finger workspace swipe\\n    Synthetic input / real compositor rendering\\n'
        call('type', 'printf ' + shlex.quote(text))
        key('Return')
        time.sleep(0.3)

    def process_sample(pid):
        base = Path('/proc') / str(pid)
        fields = (base / 'stat').read_text().split(') ', 1)[1].split()
        status = dict(line.split(':', 1) for line in (base / 'status').read_text().splitlines() if ':' in line)
        sched = (base / 'schedstat').read_text().split()
        return {'ticks': int(fields[11]) + int(fields[12]), 'rss_kib': int(status['VmRSS'].split()[0]),
                'threads': int(status['Threads']), 'fds': len(list((base / 'fd').iterdir())),
                'context_switches': int(status['voluntary_ctxt_switches']) + int(status['nonvoluntary_ctxt_switches']),
                'scheduler_runs': int(sched[2]), 'time': time.monotonic()}


    def exercise_skip_empty():
        terminal(1, '28;65;100')
        first = screenshot('workspace-one')
        terminal(4, '110;45;65')
        fourth = screenshot('workspace-four')
        terminal(7, '35;100;65')
        seventh = screenshot('workspace-seven')
        # Only kill a process launched by this private harness when testing an
        # inactive target's destruction; focusing it would cancel the gesture first.
        launched = re.findall(r'application launched pid=(\d+) workspace=4\b', (directory / 'session.log').read_text())
        check(bool(launched), 'private workspace-four terminal PID recorded')
        target_pid = int(launched[-1])
        check((Path('/proc') / str(target_pid) / 'comm').read_text().strip() == 'kitty', 'inactive target is the harness-launched Kitty')
        key('Super_L+1')
        active(1)
        call('record-start', 'workspace-skip-empty-demo')
        key('Super_L+Right')
        active(2)
        checks.append('default policy visits empty workspace two')
        key('Super_L+1')

        def policy(value, succeeds=True):
            config_path.write_text(original + f'\n[compositor.workspace]\nskip_empty = {value}\n')
            check((reload().returncode == 0) == succeeds, f'policy reload {value}')

        policy('true')
        for number in [4, 7, 1]:
            key('Super_L+Right')
            active(number)
        checks.append('next cycles sparse workspaces one-four-seven-one')
        for number in [7, 4, 1]:
            key('Super_L+Left')
            active(number)
        checks.append('previous cycles sparse workspaces one-seven-four-one')
        key('Super_L+2')
        active(2)
        key('Super_L+Right')
        active(4)
        checks.append('direct selection reaches empty slot and cycling escapes it')
        key('Super_L+m')
        wait(lambda state: any(w['workspace'] == 4 and w['minimized'] for w in state['windows']), 'minimize workspace four')
        key('Super_L+1')
        key('Super_L+Right')
        active(4)
        checks.append('minimized-only workspace remains in cycle')
        key('Super_L+Shift_L+m')
        wait(lambda state: any(w['workspace'] == 4 and not w['minimized'] for w in state['windows']), 'restore workspace four')
        key('Super_L+1')
        before = screenshot('skip-empty-before')
        begin()
        result = travel(-16, 10)
        check(result['target'] == 4 and result['offset'] == -640 and result['active_workspace'] == 1, 'sparse swipe previews four without committing')
        middle = screenshot('skip-empty-halfway-one-four')
        check(rgb(before, '100x10+10+5') == rgb(middle, '100x10+10+5'), 'panel stays stationary across skipped slots')
        check(rgb(middle, '1x1+100+400') == rgb(first, '1x1+740+400'), 'outgoing workspace one slides left')
        check(rgb(middle, '1x1+1100+400') == rgb(fourth, '1x1+460+400'), 'workspace four enters from right despite nonadjacent ID')
        time.sleep(0.4)
        gesture('end')
        settle(4)
        screenshot('skip-empty-committed-four')
        begin()
        result = travel(16, 10)
        check(result['target'] == 1 and result['active_workspace'] == 4, 'reverse sparse swipe previews workspace one')
        middle = screenshot('skip-empty-halfway-four-one')
        check(rgb(middle, '1x1+100+400') == rgb(first, '1x1+740+400'), 'previous workspace enters from left')
        check(rgb(middle, '1x1+1100+400') == rgb(fourth, '1x1+460+400'), 'outgoing workspace four slides right')
        gesture('end')
        settle(1)
        begin()
        travel(-12, 5)
        result = travel(24, 10)
        check(result['target'] == 7 and result['offset'] > 320, 'mid-swipe reversal previews wrapped workspace seven')
        middle = screenshot('skip-empty-reversal-one-seven')
        check(rgb(middle, '1x1+100+400') == rgb(seventh, '1x1+660+400'), 'wrapped previous workspace renders on left')
        gesture('end')
        settle(7)
        begin()
        travel(-16, 10)
        gesture('end')
        settle(1)
        checks.append('forward sparse swipe wraps seven to one')
        begin()
        travel(-5, 10)
        gesture('end')
        settle(1)
        checks.append('short sparse swipe snaps back')
        policy('"invalid"', succeeds=False)
        key('Super_L+Right')
        active(4)
        checks.append('invalid reload retains enabled skip policy')
        key('Super_L+1')
        begin()
        gesture('update', x=-100, y=0)
        policy('false')
        gesture('update', x=-100, y=0)
        gesture('end')
        result = gesture('query')
        check(result['offset'] is None and result['active_workspace'] == 1, 'policy reload cancels an active swipe')
        key('Super_L+Right')
        active(2)
        checks.append('disabling policy restores sequential cycling')
        policy('true')
        key('Super_L+1')
        begin()
        check(gesture('update', x=-100, y=0)['target'] == 4, 'inactive target selected before destruction')
        os.kill(target_pid, signal.SIGTERM)
        wait(lambda state: not any(w['workspace'] == 4 for w in state['windows']), 'inactive target closes')
        result = gesture('query')
        check(result['offset'] is None and result['active_workspace'] == 1, 'destroying inactive target cancels its preview')
        gesture('update', x=-100, y=0)
        gesture('end')
        check(gesture('query')['active_workspace'] == 1, 'destroyed target never retargets on release')
        key('Super_L+7')
        key('Super_L+q')
        wait(lambda state: not any(w['workspace'] == 7 for w in state['windows']), 'workspace seven closes')
        active(7)
        checks.append('closing last window leaves current empty workspace active')
        key('Super_L+Right')
        active(1)
        key('Super_L+Right')
        active(1)
        key('Super_L+Left')
        active(1)
        begin()
        result = gesture('update', x=-200, y=0)
        gesture('end')
        check(result['offset'] is None and gesture('query')['active_workspace'] == 1, 'single occupied workspace has no cycling or swipe motion')
        key('Super_L+q')
        wait(lambda state: not state['windows'], 'all managed windows close')
        active(1)
        key('Super_L+2')
        active(2)
        key('Super_L+Right')
        key('Super_L+Left')
        begin()
        result = gesture('update', x=-200, y=0)
        gesture('end')
        check(result['offset'] is None and gesture('query')['active_workspace'] == 2, 'all-empty desktop keeps selected slot without swipe motion')
        screenshot('skip-empty-all-empty')
        # Restore two occupied slots for normal/stress resource comparisons.
        terminal(1, '28;65;100')
        terminal(4, '110;45;65')
        key('Super_L+1')
        active(1)

    started = False
    try:
        subprocess.run(['python3', str(harness_path), '--run', str(directory), 'start', '--bundle', str(bundle), '--display', args.display, '--no-vnc', '--gesture-test'], check=True, capture_output=True, text=True, timeout=60)
        started = True
        manifest = json.loads((directory / 'manifest.json').read_text())
        env = dict(os.environ, XDG_RUNTIME_DIR=manifest['runtime'], KNAVE_SOCKET=str(Path(manifest['runtime']) / 'knave' / 'desktop-wayland-0.sock'))
        sockets = list(Path(manifest['runtime']).glob('knave/desktop-*.sock'))
        check(len(sockets) == 1, 'isolated compositor control socket')
        env['KNAVE_SOCKET'] = str(sockets[0])
        config_path = directory / 'config.toml'
        original = config_path.read_text()
        def reload():
            return subprocess.run([str(bundle / 'bin/knavectl'), 'reload'], env=env, capture_output=True, text=True, timeout=10)
        if args.skip_empty:
            exercise_skip_empty()
        else:
            terminal(1, '28;65;100')
            first = screenshot('workspace-one')
            terminal(2, '110;45;65')
            second = screenshot('workspace-two')
            key('Super_L+1')
            active(1)
            call('record-start', 'workspace-swipe-demo')
            time.sleep(0.4)
            before = screenshot('before-swipe')
            begin()
            travel(-5, 10)
            check(gesture('query')['active_workspace'] == 1, 'short swipe keeps workspace during travel')
            screenshot('short-swipe')
            gesture('end')
            settle(1)
            time.sleep(0.4)
            begin()
            result = travel(-16, 10)
            check(result['offset'] == -640 and result['active_workspace'] == 1 and result['target'] == 2, 'half-width live follow before release')
            middle = screenshot('halfway-live-follow')
            check(rgb(before, '100x10+10+5') == rgb(middle, '100x10+10+5'), 'panel stays stationary during swipe')
            check(rgb(middle, '1x1+100+400') == rgb(first, '1x1+740+400'), 'outgoing workspace rendered at translated position')
            check(rgb(middle, '1x1+1100+400') == rgb(second, '1x1+460+400'), 'incoming workspace rendered beside outgoing workspace')
            check(rgb(middle, '1x1+100+400') != rgb(middle, '1x1+1100+400'), 'two distinct workspace contents visible together')
            time.sleep(0.6)
            result = gesture('end')
            check(result['active_workspace'] == 2, 'release commits through workspace dispatch')
            settle(2)
            screenshot('committed-workspace-two')
            time.sleep(0.4)
            begin()
            travel(16, 10)
            gesture('end')
            settle(1)
            screenshot('returned-to-workspace-one')
            # Reverse while fingers remain down, then commit in the opposite direction.
            begin()
            travel(-12, 5)
            result = travel(24, 10)
            check(result['offset'] > 320 and result['target'] == 10, 'direction reversal changes preview and wraps backwards')
            gesture('end')
            settle(10)
            begin()
            travel(-16, 10)
            gesture('end')
            settle(1)
            checks.append('forward wrap from workspace ten to one')
            begin()
            travel(-16, 10)
            gesture('end', cancelled=True)
            settle(1)
            checks.append('backend cancellation never commits beyond threshold')
            begin()
            gesture('update', x=2, y=30)
            result = gesture('update', x=-200, y=0)
            check(result['offset'] is None, 'vertical dominance reserves stream without workspace motion')
            gesture('end')
            begin()
            gesture('update', device='unrelated', x=-200, y=0)
            gesture('end', device='unrelated')
            check(gesture('query')['offset'] is None, 'other devices cannot update or end owned stream')
            gesture('update', x=-100, y=0)
            gesture('remove')
            check(gesture('query')['offset'] is None, 'device removal cancels transition')
            begin()
            gesture('update', x=-100, y=0)
            key('Super_L+2')
            gesture('update', x=-200, y=0)
            gesture('end')
            check(gesture('query')['offset'] is None, 'explicit workspace switch consumes remainder of interrupted swipe')
            active(2)
            key('Super_L+1')
            active(1)
            begin()
            gesture('update', x=-100, y=0)
            key('Print')
            gesture('update', x=-200, y=0)
            gesture('end')
            check(gesture('query')['active_workspace'] == 1 and gesture('query')['offset'] is None, 'frozen screenshot interrupts swipe')
            key('Escape')
            gesture('begin', fingers=4)
            gesture('update', x=-200, y=0)
            gesture('end')
            check(gesture('query')['active_workspace'] == 1, 'unconfigured finger count does not switch workspaces')
            # Test successful reload cancellation, disable override, and atomic failed reload.
            begin()
            gesture('update', x=-100, y=0)
            check(reload().returncode == 0, 'valid configuration reload')
            gesture('update', x=-200, y=0)
            gesture('end')
            check(gesture('query')['offset'] is None and gesture('query')['active_workspace'] == 1, 'reload cancels active swipe')
            config_path.write_text(original + '\n[compositor.gesture]\nbind = [{ fingers = 3, direction = "horizontal", action = "none" }]\n')
            check(reload().returncode == 0, 'disable override reloads')
            begin()
            gesture('update', x=-200, y=0)
            gesture('end')
            check(gesture('query')['active_workspace'] == 1 and gesture('query')['offset'] is None, 'none override disables shipped gesture')
            config_path.write_text(original + '\n[compositor.gesture]\nbind = [{ fingers = 0, direction = "horizontal", action = "workspace" }]\n')
            check(reload().returncode != 0, 'invalid reload is rejected')
            begin()
            gesture('update', x=-200, y=0)
            gesture('end')
            check(gesture('query')['active_workspace'] == 1, 'failed reload retains disabled binding')
            config_path.write_text(original)
            check(reload().returncode == 0, 'default binding restored')
        # Bound resource behavior: no extra threads/descriptors or accumulating timers.
        session = call('status')['processes']['session']
        children = [session]
        villain = None
        while children:
            pid = children.pop()
            proc = Path('/proc') / str(pid)
            if (proc / 'comm').read_text().strip() == 'villain':
                villain = pid
                break
            children.extend(int(value) for value in (proc / 'task' / str(pid) / 'children').read_text().split())
        check(villain is not None, 'compositor process identified for resource samples')
        idle_start = process_sample(villain)
        time.sleep(1)
        idle_end = process_sample(villain)
        normal_start = process_sample(villain)
        begin()
        travel(-12, 10)
        gesture('end', cancelled=True)
        settle(1)
        normal_end = process_sample(villain)
        stress_start = process_sample(villain)
        for _ in range(30):
            begin()
            gesture('update', x=-120, y=0)
            gesture('end', cancelled=True)
        settle(1)
        stress_end = process_sample(villain)
        time.sleep(1)
        after = process_sample(villain)
        check(after['threads'] == idle_end['threads'] and after['fds'] == idle_end['fds'], 'stress leaves thread and descriptor counts unchanged')
        check(after['rss_kib'] - idle_end['rss_kib'] < 32768, 'stress resident memory growth below 32 MiB')
        screenshot('final-workspace-one')
        video = call('record-stop')
        report = {'passed': True, 'mode': 'skip_empty' if args.skip_empty else 'sequential', 'coverage': 'synthetic input, real nested Winit/software renderer; physical touchpad and DRM unverified', 'checks': checks, 'samples': samples, 'resources': {'idle_start': idle_start, 'idle_end': idle_end, 'normal_start': normal_start, 'normal_end': normal_end, 'stress_start': stress_start, 'stress_end': stress_end, 'after_idle': after}, 'bundle': str(bundle), 'bundle_sha256': manifest['bundle_sha256'], **video}
        (directory / 'gesture-result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps({key: value for key, value in report.items() if key != 'samples'}, indent=2))
    finally:
        if started:
            call('stop')


if __name__ == '__main__':
    main()
