"""Dedicated-account bundle selection and direct-TTY session activation."""

import hashlib
import json
import os
import shlex
import signal
import subprocess
import sys
import tempfile
import time
import tomllib
import uuid
from pathlib import Path

from knave_dev import atomic_json, capture, digest, lock, now, tested_bundle

UNITS = (
    'xdg-desktop-portal.service',
    'xdg-desktop-portal-knave.service',
    'xdg-desktop-portal-gtk.service',
)
ACTIVATION_KEYS = (
    'WAYLAND_DISPLAY',
    'DISPLAY',
    'XDG_CURRENT_DESKTOP',
    'XDG_SESSION_DESKTOP',
    'XDG_SESSION_TYPE',
    'XDG_DATA_DIRS',
    'PATH',
    'XDP_KNAVE_SHELL_BINARY',
    'XDP_KNAVE_ENABLED',
)


def account(state):
    try:
        marker = json.loads((state / 'tty-account.json').read_text())
    except FileNotFoundError:
        raise ValueError(
            'Set up the dedicated test account once with: '
            './knave-dev tty init --dedicated-test-account'
        ) from None
    if (
        marker != {'uid': os.getuid(), 'home': str(Path.home().resolve())}
        or os.getuid() == 0
    ):
        raise ValueError(
            'run tty init --dedicated-test-account as the dedicated non-root test user'
        )


def sessions():
    result = json.loads(capture(['loginctl', 'list-sessions', '--json=short']))
    return [str(row['session']) for row in result if int(row['uid']) == os.getuid()]


def properties(session):
    return dict(
        line.split('=', 1)
        for line in capture(
            [
                'loginctl',
                'show-session',
                session,
                '-p',
                'User',
                '-p',
                'Active',
                '-p',
                'Remote',
                '-p',
                'Type',
                '-p',
                'Seat',
            ]
        ).splitlines()
    )


def no_graphical_sessions():
    for session in sessions():
        if properties(session).get('Type') in ('wayland', 'x11'):
            raise ValueError('test account has a graphical login; log it out first')


def init(state):
    if os.getuid() == 0:
        raise ValueError('TTY testing requires a dedicated non-root login account')
    no_graphical_sessions()
    with lock(state / 'tty.lock'):
        atomic_json(
            state / 'tty-account.json',
            {'uid': os.getuid(), 'home': str(Path.home().resolve())},
        )


def symlink(path, target):
    temporary = path.with_name('.' + path.name + '-' + uuid.uuid4().hex)
    try:
        temporary.symlink_to(target)
        temporary.replace(path)
    finally:
        temporary.unlink(missing_ok=True)


def select(state, bundle, rollback=False):
    account(state)
    with lock(state / 'tty.lock'):
        no_graphical_sessions()
        return select_locked(state, bundle, rollback)


def select_locked(state, bundle, rollback=False):
    # Caller holds tty.lock through selection and, for session, through teardown.
    if (state / 'tty-active.json').exists():
        raise ValueError(
            'interrupted session requires tty recover before selecting a bundle'
        )
    if rollback:
        bundle = (state / 'previous').resolve(strict=True)
    bundle = bundle.resolve(strict=True)
    tested_bundle(state, bundle)
    current = state / 'current'
    if current.is_symlink():
        old = current.resolve(strict=True)
        if old == bundle:
            return bundle
        symlink(state / 'previous', old)
    elif current.exists():
        raise ValueError('current must be a symlink managed by knave-dev')
    symlink(current, bundle)
    return bundle


def local_tty():
    session = os.environ.get('XDG_SESSION_ID')
    if not session or not sys.stdin.isatty():
        raise ValueError('start from the dedicated user’s active local TTY login')
    props = properties(session)
    if (
        props.get('User') != str(os.getuid())
        or props.get('Active') != 'yes'
        or props.get('Remote') != 'no'
        or props.get('Type') != 'tty'
        or not props.get('Seat')
    ):
        raise ValueError(
            'an active local logind TTY session with a physical seat is required'
        )
    no_graphical_sessions()
    runtime = Path(os.environ['XDG_RUNTIME_DIR'])
    if runtime.stat().st_uid != os.getuid() or not (runtime / 'bus').exists():
        raise ValueError(
            'the login must provide the test user’s runtime directory and D-Bus'
        )
    return runtime


def systemctl(*args, check=True):
    return subprocess.run(
        ['systemctl', '--user', *args],
        check=check,
        capture_output=True,
        text=True,
        timeout=30,
    )


def metadata_paths():
    data = Path(os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
    config = Path(os.environ.get('XDG_CONFIG_HOME', str(Path.home() / '.config')))
    if not data.is_absolute() or not config.is_absolute():
        raise ValueError('XDG config/data paths must be absolute')
    if data != Path.home() / '.local/share' or config != Path.home() / '.config':
        raise ValueError(
            'TTY test accounts must use default XDG data/config locations for login-bus discovery'
        )
    return {
        'descriptor': data / 'xdg-desktop-portal/portals/knave.portal',
        'routing': data / 'xdg-desktop-portal/knave-portals.conf',
        'villain-routing': data / 'xdg-desktop-portal/villain-portals.conf',
        'dbus': data
        / 'dbus-1/services/org.freedesktop.impl.portal.desktop.knave.service',
        'unit': config / 'systemd/user/xdg-desktop-portal-knave.service',
    }


def reload_activation():
    systemctl('daemon-reload')
    capture(
        [
            'busctl',
            '--user',
            'call',
            'org.freedesktop.DBus',
            '/org/freedesktop/DBus',
            'org.freedesktop.DBus',
            'ReloadConfig',
        ]
    )


def quote_exec(path, systemd=False):
    value = str(path).replace('\\', '\\\\').replace('"', '\\"')
    if any(c in value for c in '\n\r\x00'):
        raise ValueError('activation paths cannot contain newlines or NUL')
    if systemd:
        value = value.replace('%', '%%').replace('$', '$$')
    return '"' + value + '"'


def activation_files(bundle, run):
    launcher = run / 'portal-launch'
    launcher.write_text(
        '#!/bin/sh\nexport KNAVE_CONFIG='
        + shlex.quote(str(run / 'config.toml'))
        + '\nexec '
        + shlex.quote(str(bundle / 'libexec/xdg-desktop-portal-knave'))
        + '\n'
    )
    launcher.chmod(0o700)
    return {
        'descriptor': (
            bundle / 'share/xdg-desktop-portal/portals/knave.portal'
        ).read_text(),
        'routing': (bundle / 'share/xdg-desktop-portal/knave-portals.conf').read_text(),
        'villain-routing': (
            bundle / 'share/xdg-desktop-portal/villain-portals.conf'
        ).read_text(),
        'dbus': '[D-BUS Service]\nName=org.freedesktop.impl.portal.desktop.knave\n'
        f'Exec={quote_exec(launcher)}\nSystemdService=xdg-desktop-portal-knave.service\n',
        'unit': '[Unit]\nDescription=Knave development bundle portal\n'
        '[Service]\nType=dbus\nBusName=org.freedesktop.impl.portal.desktop.knave\n'
        f'ExecStart={quote_exec(launcher, systemd=True)}\nRestart=no\n',
    }


def configure(bundle, run, source):
    # Reuse the same normalizer as nested testing, then set the TTY differences.
    import importlib.util
    from types import SimpleNamespace

    spec = importlib.util.spec_from_file_location(
        'bundle_harness', bundle / 'tools/desktop-harness.py'
    )
    harness = importlib.util.module_from_spec(spec)
    previous_bytecode = sys.dont_write_bytecode
    try:
        sys.dont_write_bytecode = True
        spec.loader.exec_module(harness)
    finally:
        sys.dont_write_bytecode = previous_bytecode
    path = run / 'config.toml'
    harness.write_config(
        path,
        SimpleNamespace(config=source),
        {'villain': bundle / 'bin/villain', 'knave-shell': bundle / 'bin/knave-shell'},
    )
    config = tomllib.loads(path.read_text())
    config['session']['backend'] = 'tty'
    # Explicitly enabled for the direct desktop; do not inherit disabled nested settings.
    config['portal']['enabled'] = True
    path.write_text(
        '\n'.join(
            json.dumps(k) + ' = ' + harness.toml_value(v) for k, v in config.items()
        )
        + '\n'
    )


def restore(state, journal):
    errors = []
    failures = []
    result = systemctl('stop', *UNITS, check=False)
    if result.returncode:
        # Missing optional GTK units must not prevent cleanup of files/environment.
        errors.append(result.stderr.strip())
    for name, expected in journal['files'].items():
        path = Path(name)
        if not path.exists() and not path.is_symlink():
            continue
        if path.is_symlink() or not path.is_file() or digest(path) != expected:
            failures.append(f'preserved externally changed activation file: {path}')
        else:
            path.unlink()
    try:
        reload_activation()
    except (OSError, subprocess.SubprocessError) as error:
        failures.append(str(error))
    old = journal['systemd_environment']
    missing = [key for key in ACTIVATION_KEYS if key not in old]
    if missing:
        systemctl('unset-environment', *missing)
    present = [f'{key}={old[key]}' for key in ACTIVATION_KEYS if key in old]
    if present:
        systemctl('set-environment', *present)
    # D-Bus has no unset operation: empty values clear previously absent keys.
    capture(
        [
            'dbus-update-activation-environment',
            *[f'{key}={old.get(key, "")}' for key in ACTIVATION_KEYS],
        ]
    )
    active = [
        unit
        for unit in UNITS
        if systemctl('is-active', '--quiet', unit, check=False).returncode == 0
    ]
    if active or failures:
        raise RuntimeError(
            'TTY cleanup incomplete; run tty recover: ' + '; '.join(active + failures)
        )
    (state / 'tty-active.json').unlink(missing_ok=True)
    if errors:
        print('TTY cleanup diagnostics: ' + '; '.join(errors), file=sys.stderr)


def recover(state):
    account(state)
    with lock(state / 'tty.lock'):
        no_graphical_sessions()
        journal = json.loads((state / 'tty-active.json').read_text())
        if 'pid' in journal:
            pid = journal['pid']
            stat = Path(f'/proc/{pid}/stat')
            if stat.exists():
                ticks = stat.read_text().rsplit(')', 1)[1].split()[19]
                if ticks != journal['start_ticks'] or stat.stat().st_uid != os.getuid():
                    raise ValueError(
                        'recorded session PID was reused; inspect the session manually'
                    )
                os.killpg(pid, signal.SIGTERM)
                deadline = time.monotonic() + 10
                while stat.exists() and time.monotonic() < deadline:
                    time.sleep(0.1)
                try:
                    os.killpg(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        restore(state, journal)


def start(state, source=None, bundle=None):
    account(state)
    with lock(state / 'tty.lock'):
        runtime = local_tty()
        if (state / 'tty-active.json').exists():
            raise ValueError('interrupted session requires tty recover before starting')
        bundle = (
            select_locked(state, bundle)
            if bundle is not None
            else (state / 'current').resolve(strict=True)
        )
        manifest = tested_bundle(state, bundle)
        for unit in UNITS:
            if systemctl('is-active', '--quiet', unit, check=False).returncode == 0:
                raise ValueError(
                    f'{unit} is active on this account’s shared user bus; '
                    'keep development sessions running with '
                    './knave-dev session FEATURE --nested, or use a separate '
                    'Unix test account for TTY testing'
                )
        paths = metadata_paths()
        for path in paths.values():
            if path.exists() or path.is_symlink():
                raise ValueError(
                    f'test account already has portal metadata; move it aside explicitly: {path}'
                )
        run = Path(tempfile.mkdtemp(prefix='knave-tty-', dir=state))
        configure(bundle, run, source.resolve(strict=True) if source else None)
        env = os.environ.copy()
        for key in (
            'WAYLAND_DISPLAY',
            'DISPLAY',
            'KNAVE_SOCKET',
            'KNAVE_SESSION',
            'KNAVE_HARNESS_READY_DIR',
            'LIBGL_ALWAYS_SOFTWARE',
            'WGPU_BACKEND',
            'LP_NUM_THREADS',
        ):
            env.pop(key, None)
        env.update(
            KNAVE_CONFIG=str(run / 'config.toml'),
            XDG_SESSION_TYPE='wayland',
            XDG_CURRENT_DESKTOP='Knave:Villain',
            XDG_SESSION_DESKTOP='knave',
            PATH=str(bundle / 'bin') + ':' + os.environ.get('PATH', '/usr/bin'),
            XDG_DATA_DIRS=str(bundle / 'share')
            + ':'
            + os.environ.get('XDG_DATA_DIRS', '/usr/local/share:/usr/share'),
            DBUS_SESSION_BUS_ADDRESS='unix:path=' + str(runtime / 'bus'),
        )
        subprocess.run(
            [str(bundle / 'bin/knave'), 'config', 'check'],
            env=env,
            check=True,
            timeout=15,
        )
        old_environment = dict(
            line.split('=', 1)
            for line in shlex.split(systemctl('show-environment').stdout)
            if '=' in line
        )
        contents = activation_files(bundle, run)
        journal = {
            'bundle': str(bundle),
            'bundle_sha256': digest(bundle / 'bundle.json'),
            'started_at': now(),
            'run': str(run),
            'files': {
                str(paths[name]): hashlib.sha256(value.encode()).hexdigest()
                for name, value in contents.items()
            },
            'systemd_environment': {
                key: old_environment[key]
                for key in ACTIVATION_KEYS
                if key in old_environment
            },
        }
        atomic_json(state / 'tty-active.json', journal)
        child = None
        previous_handlers = {}
        try:
            for name, text in contents.items():
                path = paths[name]
                path.parent.mkdir(parents=True, exist_ok=True)
                with path.open('x') as stream:
                    stream.write(text)
            reload_activation()
            # All desktop descendants have one owned process group. The parent
            # retains tty.lock through service teardown, including signal paths.
            print(
                f'Starting {manifest["id"]}; session log: {run / "session.log"}',
                flush=True,
            )
            with (run / 'session.log').open('ab') as log:
                child = subprocess.Popen(
                    [str(bundle / 'bin/knave'), 'session', 'start'],
                    env=env,
                    stdout=log,
                    stderr=subprocess.STDOUT,
                    start_new_session=True,
                )
            journal['pid'] = child.pid
            journal['start_ticks'] = (
                Path(f'/proc/{child.pid}/stat')
                .read_text()
                .rsplit(')', 1)[1]
                .split()[19]
            )
            atomic_json(state / 'tty-active.json', journal)

            def interrupted(signum, _frame):
                if child.poll() is None:
                    os.killpg(child.pid, signum)

            for signum in (signal.SIGINT, signal.SIGTERM, signal.SIGHUP):
                previous_handlers[signum] = signal.signal(signum, interrupted)
            code = child.wait()
            atomic_json(
                run / 'result.json',
                {'exit_code': code, 'bundle': manifest['id'], 'ended_at': now()},
            )
            return code
        finally:
            if child:
                try:
                    os.killpg(child.pid, signal.SIGTERM)
                    deadline = time.monotonic() + 5
                    while child.poll() is None and time.monotonic() < deadline:
                        time.sleep(0.1)
                    os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                child.wait()
            try:
                restore(state, journal)
            finally:
                for signum, handler in previous_handlers.items():
                    signal.signal(signum, handler)
