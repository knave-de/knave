#!/usr/bin/env python3
"""Exercise real desktop input and save a demonstration video. Run outside sandbox."""

import argparse
import json
import subprocess
import time
from pathlib import Path

HARNESS = Path(__file__).with_name('desktop-harness.py')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='93')
    parser.add_argument('--vnc-port', default='5993')
    parser.add_argument('--bundle', type=Path)
    args = parser.parse_args()
    directory = args.run.resolve()
    command = ['python3', str(HARNESS), '--run', str(directory)]

    def call(*values):
        result = subprocess.run(command + list(values), check=True, text=True,
                                capture_output=True, timeout=60)
        return json.loads(result.stdout)

    def wait_snapshot(predicate):
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            snapshot = call('snapshot')['payload']
            if predicate(snapshot):
                return snapshot
            time.sleep(0.2)
        raise AssertionError('desktop state did not reach expected result')

    started = False
    try:
        options = ['--bundle', str(args.bundle.resolve())] if args.bundle else []
        call('start', '--display', args.display, '--vnc-port', args.vnc_port, *options)
        started = True
        call('record-start', 'computer-use-demo')
        call('key', 'Super_L')
        wait_snapshot(lambda s: s['overview_visible'])
        # The overview projects the desktop bar, so the search field starts
        # below it. Click the field itself to focus it.
        call('click', '460', '90')
        call('type', 'kitty')
        # Give the asynchronous app catalog time to filter before taking evidence.
        time.sleep(1)
        call('screenshot', 'launcher-search')
        call('key', 'Return')
        wait_snapshot(lambda s: any(w['app_id'] == 'kitty' and w['focused'] for w in s['windows']))
        proof = directory / 'input-proof.txt'
        # Generated paths are shell-quoted because this text is typed into a shell.
        import shlex
        call('type', f'echo KNAVE_INPUT_OK | tee {shlex.quote(str(proof))}')
        call('key', 'Return')
        deadline = time.monotonic() + 10
        while not proof.exists() and time.monotonic() < deadline:
            time.sleep(0.1)
        assert proof.read_text().strip() == 'KNAVE_INPUT_OK', 'application input failed'
        call('screenshot', 'application-input')
        call('click', '192', '18')
        wait_snapshot(lambda s: any(w['workspace'] == 2 and w['active'] for w in s['workspaces']))
        call('screenshot', 'workspace-2')
        call('click', '153', '18')
        wait_snapshot(lambda s: any(w['workspace'] == 1 and w['active'] for w in s['workspaces']))
        time.sleep(1)
        video = call('record-stop')
        report = {'passed': True, 'checks': ['overview shortcut', 'launcher click and search',
                  'Kitty launch and focus', 'application keyboard input', 'workspace mouse navigation',
                  'MP4 finalization and ffprobe validation'], **video}
        if args.bundle:
            report['portal'] = call('portal-check')
            report['checks'].append('private D-Bus activation, Settings.Read and backend executable identity')
        (directory / 'smoke-result.json').write_text(json.dumps(report, indent=2) + '\n')
        print(json.dumps(report, indent=2))
    finally:
        if started:
            call('stop')


if __name__ == '__main__':
    try:
        main()
    except subprocess.CalledProcessError as error:
        print(error.stderr, end='')
        raise SystemExit(error.returncode)
