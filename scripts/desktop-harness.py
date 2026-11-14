#!/usr/bin/env python3
"""Isolated, recordable Knave Winit sessions. Python 3.11+, Linux/X11 tools."""

import argparse
import datetime
import hashlib
import json
import os
import re
import secrets
import shutil
import signal
import socket
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAX_REQUEST = 65536


def run(command, **kwargs):
    return subprocess.run(command, check=True, timeout=15, capture_output=True,
                          text=True, **kwargs).stdout.strip()


def stamp():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def toml_value(value):
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, bool):
        return str(value).lower()
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, (datetime.datetime, datetime.date, datetime.time)):
        return value.isoformat()
    if isinstance(value, list):
        return '[' + ', '.join(toml_value(v) for v in value) + ']'
    if isinstance(value, dict):
        return '{' + ', '.join(json.dumps(k) + ' = ' + toml_value(v)
                               for k, v in value.items()) + '}'
    raise ValueError(f'unsupported TOML value: {type(value)}')


def write_config(path, args, binaries):
    # Normalize a private copy, preserving values while forcing test lifecycle.
    config = tomllib.loads(args.config.read_text()) if args.config else {}
    if args.config:
        projection = config.get('compositor', {})
        configured = projection.get('environment_file')
        if configured is not None:
            source = Path(configured).expanduser()
            if not source.is_absolute():
                source = args.config.parent / source
            shutil.copyfile(source, path.parent / 'environment')
            projection['environment_file'] = 'environment'
    config['session'] = dict(config.get('session', {}), backend='winit',
                             compositor_binary=str(binaries['villain']),
                             shell_binary=str(binaries['knave-shell']),
                             restart_on_failure=False)
    config['portal'] = dict(config.get('portal', {}), enabled=False)
    path.write_text('\n'.join(json.dumps(k) + ' = ' + toml_value(v)
                              for k, v in config.items()) + '\n')


def request(directory, command, values=None, timeout=40):
    with socket.socket(socket.AF_UNIX) as client:
        client.settimeout(timeout)
        client.connect(str(directory / 'control.sock'))
        client.sendall(json.dumps({'command': command, 'values': values or []}).encode() + b'\n')
        response = b''
        while b'\n' not in response:
            chunk = client.recv(4096)
            if not chunk:
                raise RuntimeError('harness disconnected')
            response += chunk
            if len(response) > MAX_REQUEST:
                raise RuntimeError('oversized harness response')
    result = json.loads(response)
    if 'error' in result:
        raise RuntimeError(result['error'])
    return result


def start_daemon(command, directory, timeout=45):
    with (directory / 'harness.log').open('ab') as log:
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL, stdout=log,
                                   stderr=log, start_new_session=True)
    ready = False
    try:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if process.poll() is not None:
                raise RuntimeError(f'startup failed; see {directory / "harness.log"}')
            try:
                response = request(directory, 'status', timeout=max(0.01, min(1, deadline - time.monotonic())))
            except (FileNotFoundError, ConnectionRefusedError, socket.timeout):
                time.sleep(0.1)
                continue
            ready = True
            return response
        raise RuntimeError(f'startup timed out; see {directory / "harness.log"}')
    finally:
        if not ready:
            if process.poll() is None:
                process.terminate()
            try:
                process.wait(timeout=25)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
                raise RuntimeError('daemon cleanup stalled; inspect harness.log for remaining children')


def shell_services_ready(directory, roles):
    for role in roles:
        try:
            if (directory / f'{role}.ready').read_text() != 'ready\n':
                return False
        except FileNotFoundError:
            return False
    return True


def create_vnc_auth(path, password):
    run(['x11vnc', '-storepasswd', str(path)],
        input=f'{password}\n{password}\ny\n', start_new_session=True)


class Harness:
    def __init__(self, args):
        self.args = args
        self.directory = args.run
        self.processes = {}
        self.logs = []
        self.recording = None
        self.finished = False
        self.runtime = Path(tempfile.mkdtemp(prefix='knave-test-'))
        self.ready_directory = self.runtime / 'shell-ready'
        self.ready_directory.mkdir(mode=0o700)
        self.env = os.environ.copy()
        for key in ('WAYLAND_DISPLAY', 'KNAVE_SOCKET', 'KNAVE_SESSION',
                    'KNAVE_OVERVIEW_SERVICE', 'DBUS_SESSION_BUS_ADDRESS', 'VILLAIN_GESTURE_TEST'):
            self.env.pop(key, None)
        self.env.update(DISPLAY=f':{args.display}', XDG_RUNTIME_DIR=str(self.runtime),
                        XAUTHORITY=str(self.directory / 'Xauthority'),
                        KNAVE_CONFIG=str(self.directory / 'config.toml'),
                        KNAVE_HARNESS_READY_DIR=str(self.ready_directory),
                        LIBGL_ALWAYS_SOFTWARE='1', LP_NUM_THREADS='4', WGPU_BACKEND='gl')
        for key, directory in (('XDG_CONFIG_HOME', 'config-home'), ('XDG_DATA_HOME', 'data'),
                               ('XDG_CACHE_HOME', 'cache'), ('XDG_STATE_HOME', 'state')):
            (self.directory / directory).mkdir(mode=0o700)
            self.env[key] = str(self.directory / directory)
        if args.gesture_test:
            self.env['VILLAIN_GESTURE_TEST'] = '1'
        self.binaries = {
            'knave': ROOT / 'target' / args.profile / 'knave',
            'knavectl': ROOT / 'target' / args.profile / 'knavectl',
            'villain': ROOT.parent / 'villain' / 'target' / args.profile / 'villain',
            'knave-shell': ROOT.parent / 'knaveshell' / 'target' / args.profile / 'knave-shell',
        }
        self.bundle_manifest = None
        if args.bundle:
            from knave_dev import BINARIES, verify_bundle
            self.bundle_manifest = verify_bundle(args.bundle)
            self.binaries = {name: args.bundle / destination
                             for name, (_, destination) in BINARIES.items()}
            self.env['PATH'] = str(args.bundle / 'bin') + ':' + self.env.get('PATH', '/usr/bin')
            self.env['XDG_DATA_DIRS'] = str(args.bundle / 'share') + ':' + self.env.get('XDG_DATA_DIRS', '/usr/local/share:/usr/share')

    def launch(self, name, command, env=None):
        log = (self.directory / f'{name}.log').open('ab')
        self.logs.append(log)
        process = subprocess.Popen(command, env=env or self.env, stdout=log, stderr=log,
                                   stdin=subprocess.DEVNULL, start_new_session=True)
        self.processes[name] = process
        return process

    def wait_for(self, predicate, label, seconds=20):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            if self.finished:
                raise RuntimeError('startup interrupted')
            self.check_processes()
            if predicate():
                return
            time.sleep(0.1)
        raise RuntimeError(f'timed out waiting for {label}; see logs in {self.directory}')

    def check_processes(self):
        for name, process in self.processes.items():
            if process.poll() is not None:
                raise RuntimeError(f'{name} exited ({process.returncode}); see {name}.log')
        if self.recording and self.recording[0].poll() is not None:
            raise RuntimeError('recorder exited unexpectedly; see recorder.log')

    def startup(self):
        for tool in ('Xvfb', 'xauth', 'xdotool', 'import', 'ffmpeg', 'ffprobe', 'dbus-run-session'):
            if not shutil.which(tool):
                raise RuntimeError(f'missing dependency: {tool}')
        if self.args.vnc_port and not shutil.which('x11vnc'):
            raise RuntimeError('missing dependency: x11vnc (or use --no-vnc)')
        for name, binary in self.binaries.items():
            if not binary.is_file() or not os.access(binary, os.X_OK):
                raise RuntimeError(f'build {name} first: {binary}')
        if Path(f'/tmp/.X11-unix/X{self.args.display}').exists():
            raise RuntimeError('display already exists; choose another --display')
        write_config(self.directory / 'config.toml', self.args, self.binaries)
        run([str(self.binaries['knave']), 'config', 'check'], env=self.env)
        auth = self.directory / 'Xauthority'
        auth.touch(mode=0o600)
        run(['xauth', '-f', str(auth), 'add', self.env['DISPLAY'], '.', secrets.token_hex(16)])
        self.launch('xvfb', ['Xvfb', self.env['DISPLAY'], '-screen', '0',
                            f'{self.args.size}x24', '-nolisten', 'tcp', '-auth', str(auth)])

        def display_ready():
            try:
                run(['xdotool', 'getdisplaygeometry'], env=self.env)
                return True
            except subprocess.CalledProcessError:
                return False

        self.wait_for(display_ready, 'X display')
        if self.bundle_manifest:
            for tool in ('dbus-daemon', 'dbus-update-activation-environment', 'gdbus', 'pipewire'):
                if not shutil.which(tool):
                    raise RuntimeError(f'missing bundle-test dependency: {tool}')
            self.env['PIPEWIRE_RUNTIME_DIR'] = str(self.runtime)
            self.env['PIPEWIRE_REMOTE'] = 'pipewire-0'
            self.start_bundle_bus()
            self.launch('pipewire', ['pipewire'])
            self.wait_for(lambda: (self.runtime / 'pipewire-0').exists(), 'private PipeWire')
            self.launch('session', [str(self.binaries['knave']), 'session', 'start'])
        else:
            self.launch('session', ['dbus-run-session', '--', str(self.binaries['knave']),
                                    'session', 'start'])
        self.wait_for(lambda: any(self.runtime.glob('knave/desktop-wayland-*.sock')), 'desktop socket')
        desktop_socket = next(self.runtime.glob('knave/desktop-wayland-*.sock'))
        self.env['KNAVE_SOCKET'] = str(desktop_socket)
        self.env['WAYLAND_DISPLAY'] = desktop_socket.name[len('desktop-'):-len('.sock')]
        def window_ready():
            try:
                return bool(run(['xdotool', 'search', '--name', '.'], env=self.env))
            except subprocess.CalledProcessError:
                return False

        self.wait_for(window_ready, 'Winit window')
        # A nonempty name excludes Xvfb's unnamed root window.
        windows = run(['xdotool', 'search', '--name', '.'], env=self.env).splitlines()
        run(['xdotool', 'windowsize', windows[0], *self.args.size.split('x')], env=self.env)
        run(['xdotool', 'windowfocus', windows[0]], env=self.env)
        run([str(self.binaries['knavectl']), 'snapshot'], env=self.env)
        effective = tomllib.loads((self.directory / 'config.toml').read_text())
        shell_config = effective.get('shell', {})
        roles = {'bar'} if shell_config.get('start_bar', True) else set()
        if effective.get('notifications', {}).get('enabled', True):
            roles.add('notifications')
        if shell_config.get('start_overview_service', True):
            roles.add('overview')

        if shell_config.get('wallpaper', {}).get('image'):
            roles.add('wallpaper')
        self.wait_for(lambda: shell_services_ready(self.ready_directory, roles),
                      'configured shell services')
        if self.bundle_manifest:
            # Update only this private bus, without --systemd or host imports.
            run(['dbus-update-activation-environment', 'WAYLAND_DISPLAY', 'KNAVE_CONFIG',
                 'XDP_KNAVE_SHELL_BINARY=' + str(self.binaries['knave-shell']),
                 'XDP_KNAVE_ENABLED=1'], env=self.env)
            self.portal_check()
        if self.args.vnc_port:
            with socket.socket() as reservation:
                reservation.bind(('127.0.0.1', self.args.vnc_port))
            # VNC uses only eight characters; six random bytes encode to exactly
            # eight URL-safe characters, retaining 48 bits instead of hex's 32.
            password = secrets.token_urlsafe(6)
            (self.directory / 'vnc-password.txt').write_text(password + '\n')
            create_vnc_auth(self.directory / 'vnc.auth', password)
            # VNC captures the outer X display, not Villain's Wayland socket.
            vnc_env = dict(self.env, XDG_SESSION_TYPE='x11')
            vnc_env.pop('WAYLAND_DISPLAY', None)
            self.launch('vnc', ['x11vnc', '-display', self.env['DISPLAY'], '-auth', str(auth),
                                '-listen', '127.0.0.1', '-no6', '-rfbport', str(self.args.vnc_port),
                                '-rfbauth', str(self.directory / 'vnc.auth'), '-forever', '-shared',
                                '-noxdamage', '-cursor', 'arrow'], env=vnc_env)

            def vnc_ready():
                try:
                    with socket.create_connection(('127.0.0.1', self.args.vnc_port), timeout=1) as client:
                        return client.recv(12).startswith(b'RFB ')
                except OSError:
                    return False

            self.wait_for(vnc_ready, 'VNC listener')
        manifest = {'started_at': stamp(), 'display': self.env['DISPLAY'],
                    'size': self.args.size, 'profile': self.args.profile,
                    'runtime': str(self.runtime), 'vnc_port': self.args.vnc_port,
                    'rendering': 'software; nested Winit; direct TTY untested',
                    'binaries': {}, 'repositories': {}}
        for name, path in self.binaries.items():
            with path.open('rb') as source:
                digest = hashlib.file_digest(source, 'sha256').hexdigest()
            manifest['binaries'][name] = {'path': str(path), 'sha256': digest}
        if self.bundle_manifest:
            manifest['repositories'] = self.bundle_manifest['repositories']
            manifest['bundle'] = str(self.args.bundle)
            manifest['bundle_sha256'] = hashlib.sha256((self.args.bundle / 'bundle.json').read_bytes()).hexdigest()
        else:
            for repo in (ROOT, ROOT.parent / 'villain', ROOT.parent / 'knaveshell'):
                manifest['repositories'][repo.name] = {
                    'commit': run(['git', '-C', str(repo), 'rev-parse', 'HEAD']),
                    'status': run(['git', '-C', str(repo), 'status', '--short'])}
        (self.directory / 'manifest.json').write_text(json.dumps(manifest, indent=2) + '\n')

    def start_bundle_bus(self):
        from xml.sax.saxutils import escape
        services = self.directory / 'portal-services'
        services.mkdir()
        executable = str(self.binaries['xdg-desktop-portal-knave']).replace('\\', '\\\\').replace('"', '\\"')
        (services / 'org.freedesktop.impl.portal.desktop.knave.service').write_text(
            '[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.knave\n'
            f'Exec="{executable}"\n')
        address = 'unix:path=' + str(self.runtime / 'bus')
        config = self.directory / 'bus.conf'
        config.write_text('<busconfig><type>session</type><listen>' + escape(address) + '</listen>'
                          '<servicedir>' + escape(str(services)) + '</servicedir>'
                          '<policy context="default"><allow send_destination="*"/>'
                          '<allow receive_sender="*"/><allow own="*"/></policy></busconfig>')
        self.env['DBUS_SESSION_BUS_ADDRESS'] = address
        self.launch('dbus', ['dbus-daemon', '--nofork', '--config-file=' + str(config)])
        self.wait_for(lambda: (self.runtime / 'bus').exists(), 'private session bus')

    def portal_check(self):
        if not self.bundle_manifest:
            raise ValueError('portal activation checks require --bundle')
        def call(destination, path, method, *args):
            return run(['gdbus', 'call', '--session', '--dest', destination, '--object-path', path,
                        '--method', method, *args], env=self.env)
        settings = call('org.freedesktop.impl.portal.desktop.knave', '/org/freedesktop/portal/desktop',
                        'org.freedesktop.impl.portal.Settings.Read', 'org.freedesktop.appearance', 'color-scheme')
        response = call('org.freedesktop.DBus', '/org/freedesktop/DBus',
                        'org.freedesktop.DBus.GetConnectionUnixProcessID', 'org.freedesktop.impl.portal.desktop.knave')
        match = re.fullmatch(r'\(uint32 (\d+),\)', response)
        if not match:
            raise RuntimeError(f'unexpected portal PID response: {response}')
        pid = int(match.group(1))
        if Path(f'/proc/{pid}/exe').resolve() != self.binaries['xdg-desktop-portal-knave']:
            raise RuntimeError('D-Bus activated a backend outside the selected bundle')
        if not re.fullmatch(r'\(<uint32 [012]>,\)', settings):
            raise RuntimeError(f'unexpected portal color-scheme response: {settings}')
        result = {'settings': settings, 'pid': pid, 'executable': str(self.binaries['xdg-desktop-portal-knave'])}
        (self.directory / 'portal-result.json').write_text(json.dumps(result, indent=2) + '\n')
        return result

    def artifact(self, name, extension):
        if not re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]{0,80}', name):
            raise ValueError('artifact name must contain only letters, digits, dots, underscores or hyphens')
        path = self.directory / (name if name.endswith(extension) else name + extension)
        if path.exists():
            raise ValueError(f'artifact already exists: {path}')
        return path

    def stop_recording(self):
        if self.recording is None:
            raise ValueError('no recording is running')
        process, path = self.recording
        self.recording = None
        if process.poll() is None:
            process.send_signal(signal.SIGINT)
        try:
            process.wait(timeout=15)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
            raise RuntimeError('recorder did not finalize; MP4 may be incomplete')
        if process.returncode not in (0, 255):
            raise RuntimeError(f'recorder failed ({process.returncode}); see recorder.log')
        metadata = json.loads(run(['ffprobe', '-v', 'error', '-show_format', '-show_streams',
                                  '-of', 'json', str(path)]))
        if float(metadata['format'].get('duration', 0)) <= 0:
            raise RuntimeError('recording has no duration')
        path.with_suffix('.json').write_text(json.dumps(metadata, indent=2) + '\n')
        return {'video': str(path), 'duration': metadata['format']['duration']}

    def dispatch(self, command, values):
        if command == 'status':
            return {'run': str(self.directory), 'display': self.env['DISPLAY'],
                    'vnc_port': self.args.vnc_port,
                    'password_file': str(self.directory / 'vnc-password.txt') if self.args.vnc_port else None,
                    'recording': str(self.recording[1]) if self.recording else None,
                    'processes': {name: p.pid for name, p in self.processes.items()}}
        if command == 'stop':
            self.finished = True
            result = self.stop_recording() if self.recording else {}
            return dict(result, stopped=True)
        if command == 'screenshot':
            path = self.artifact(values[0], '.png')
            run(['import', '-window', 'root', str(path)], env=self.env)
            return {'screenshot': str(path)}
        if command == 'key':
            run(['xdotool', 'key', '--clearmodifiers', *values], env=self.env)
        elif command == 'type':
            run(['xdotool', 'type', '--clearmodifiers', '--delay', '20', '--', values[0]], env=self.env)
        elif command == 'click':
            x, y, button = map(int, values)
            width, height = map(int, self.args.size.split('x'))
            if not (0 <= x < width and 0 <= y < height and 1 <= button <= 9):
                raise ValueError('click outside display or unsupported button')
            run(['xdotool', 'mousemove', str(x), str(y), 'click', str(button)], env=self.env)
        elif command == 'gesture':
            if not self.args.gesture_test:
                raise ValueError('start with --gesture-test to enable synthetic swipes')
            message = json.loads(values[0])
            if not isinstance(message, dict):
                raise ValueError('gesture request must be a JSON object')
            payload = json.dumps(message).encode()
            if len(payload) > 512:
                raise ValueError('gesture request exceeds 512 bytes')
            client_path = self.runtime / 'gesture-driver.sock'
            with socket.socket(socket.AF_UNIX, socket.SOCK_DGRAM) as client:
                try:
                    client.bind(str(client_path))
                    client.settimeout(2)
                    client.sendto(payload, str(self.runtime / 'villain-gesture-test.sock'))
                    response = json.loads(client.recv(4096))
                finally:
                    client_path.unlink(missing_ok=True)
            if 'error' in response:
                raise RuntimeError(response['error'])
            return response
        elif command == 'snapshot':
            return json.loads(run([str(self.binaries['knavectl']), 'snapshot'], env=self.env))
        elif command == 'portal-check':
            return self.portal_check()
        elif command == 'record-start':
            if self.recording:
                raise ValueError('a recording is already running')
            path = self.artifact(values[0], '.mp4')
            if path.with_suffix('.json').exists():
                raise ValueError('recording metadata already exists; choose another name')
            log = (self.directory / 'recorder.log').open('ab')
            self.logs.append(log)
            process = subprocess.Popen(
                ['ffmpeg', '-nostdin', '-hide_banner', '-loglevel', 'warning', '-n',
                 '-filter_threads', '1',
                 '-f', 'x11grab', '-framerate', '20', '-video_size', self.args.size,
                 '-draw_mouse', '1', '-i', self.env['DISPLAY'], '-an', '-c:v', 'libx264',
                 '-preset', 'ultrafast', '-threads', '2', '-crf', '23', '-pix_fmt', 'yuv420p',
                 '-movflags', '+faststart', str(path)], env=self.env,
                stdin=subprocess.DEVNULL, stdout=log, stderr=log, start_new_session=True)
            self.recording = (process, path)
            time.sleep(0.3)
            if process.poll() is not None:
                self.recording = None
                raise RuntimeError('recorder failed to start; see recorder.log')
            return {'recording': str(path)}
        elif command == 'record-stop':
            return self.stop_recording()
        else:
            raise ValueError(f'unknown command: {command}')
        return {'ok': True}

    def serve(self):
        def interrupted(_signum, _frame):
            self.finished = True

        signal.signal(signal.SIGTERM, interrupted)
        signal.signal(signal.SIGINT, interrupted)
        try:
            self.startup()
            with socket.socket(socket.AF_UNIX) as server:
                server.bind(str(self.directory / 'control.sock'))
                server.listen(4)
                server.settimeout(0.5)
                while not self.finished:
                    self.check_processes()
                    try:
                        client, _ = server.accept()
                    except socket.timeout:
                        continue
                    with client:
                        client.settimeout(2)
                        data = b''
                        try:
                            while b'\n' not in data:
                                chunk = client.recv(4096)
                                if not chunk or len(data) + len(chunk) > MAX_REQUEST:
                                    raise ValueError('invalid or oversized request')
                                data += chunk
                            payload = json.loads(data)
                            result = self.dispatch(payload['command'], payload.get('values', []))
                            with (self.directory / 'actions.jsonl').open('a') as actions:
                                actions.write(json.dumps(dict(payload, at=stamp(), result=result)) + '\n')
                        except Exception as error:
                            result = {'error': str(error)}
                        try:
                            client.sendall(json.dumps(result).encode() + b'\n')
                        except OSError:
                            pass
        finally:
            self.cleanup()

    def cleanup(self):
        if self.recording:
            try:
                self.stop_recording()
            except Exception as error:
                print(f'recording cleanup: {error}', file=sys.stderr)
        # Each launched component has its own process group, including apps that
        # outlive the session supervisor's direct children.
        for process in reversed(list(self.processes.values())):
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
        deadline = time.monotonic() + 4
        for process in self.processes.values():
            try:
                process.wait(timeout=max(0.01, deadline - time.monotonic()))
            except subprocess.TimeoutExpired:
                pass
        for process in self.processes.values():
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()
        for log in self.logs:
            log.close()
        (self.directory / 'control.sock').unlink(missing_ok=True)
        shutil.rmtree(self.runtime)
        (self.directory / 'stopped.json').write_text(json.dumps({'stopped_at': stamp()}) + '\n')


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument('--run', type=Path, required=True, help='artifact directory (new for each start)')
    commands = result.add_subparsers(dest='command', required=True)
    for name in ('start', '_serve'):
        command = commands.add_parser(name)
        command.add_argument('--gesture-test', action='store_true', help='enable private synthetic swipe input (Winit only)')
        command.add_argument('--profile', choices=('debug', 'release'), default='release')
        command.add_argument('--bundle', type=Path, help='verified immutable bundle instead of workspace artifacts')
        command.add_argument('--display', type=int, default=92)
        command.add_argument('--size', default='1280x800')
        command.add_argument('--config', type=Path, help='copy of a config to test; source is never modified')
        command.add_argument('--vnc-port', type=int, default=5992)
        command.add_argument('--no-vnc', action='store_const', const=0, dest='vnc_port')
    for name in ('status', 'stop', 'snapshot', 'record-stop', 'portal-check'):
        commands.add_parser(name)
    for name in ('screenshot', 'record-start'):
        commands.add_parser(name).add_argument('name')
    commands.add_parser('key').add_argument('keys', nargs='+')
    commands.add_parser('type').add_argument('text')
    commands.add_parser('gesture').add_argument('json_request')
    click = commands.add_parser('click')
    click.add_argument('x', type=int)
    click.add_argument('y', type=int)
    click.add_argument('--button', type=int, default=1)
    return result


def main():
    args = parser().parse_args()
    args.run = args.run.resolve()
    if len(os.fsencode(args.run / 'control.sock')) > 100:
        raise ValueError('run path is too long for a Unix socket; choose a shorter path')
    if args.command in ('start', '_serve'):
        dimensions = re.fullmatch(r'(\d+)x(\d+)', args.size)
        if not dimensions or any(int(v) < 200 or int(v) > 4096 or int(v) % 2
                                 for v in dimensions.groups()):
            raise ValueError('size must have even dimensions between 200 and 4096')
        if not 1 <= args.display <= 999 or not (args.vnc_port == 0 or 1024 <= args.vnc_port <= 65535):
            raise ValueError('invalid display or VNC port')
        if args.config:
            args.config = args.config.resolve()
        if args.bundle:
            args.bundle = args.bundle.resolve(strict=True)
    if args.command == '_serve':
        os.umask(0o077)
        Harness(args).serve()
    elif args.command == 'start':
        os.umask(0o077)
        args.run.mkdir(parents=True, mode=0o700, exist_ok=False)
        command = [sys.executable, str(Path(__file__).resolve()), '--run', str(args.run),
                   '_serve', '--profile', args.profile, '--display', str(args.display),
                   '--size', args.size, '--vnc-port', str(args.vnc_port)]
        if args.gesture_test:
            command += ['--gesture-test']
        if args.config:
            command += ['--config', str(args.config)]
        if args.bundle:
            command += ['--bundle', str(args.bundle)]
        print(json.dumps(start_daemon(command, args.run), indent=2))
    else:
        values = {'gesture': [getattr(args, 'json_request', '')], 'key': getattr(args, 'keys', []), 'type': [getattr(args, 'text', '')],
                  'click': [getattr(args, 'x', 0), getattr(args, 'y', 0), getattr(args, 'button', 1)],
                  'screenshot': [getattr(args, 'name', '')],
                  'record-start': [getattr(args, 'name', '')]}.get(args.command, [])
        response = request(args.run, args.command, values)
        if args.command == 'stop':
            deadline = time.monotonic() + 10
            while not (args.run / 'stopped.json').exists():
                if time.monotonic() > deadline:
                    raise RuntimeError('shutdown cleanup timed out; inspect harness.log')
                time.sleep(0.1)
        print(json.dumps(response, indent=2))


if __name__ == '__main__':
    try:
        main()
    except (OSError, ValueError, RuntimeError, subprocess.SubprocessError) as error:
        print(f'desktop-harness: {error}', file=sys.stderr)
        sys.exit(1)
