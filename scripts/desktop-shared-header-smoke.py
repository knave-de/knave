#!/usr/bin/env python3
"""Isolated GTK 3/4 shared-header verification; never changes host GTK settings."""
import argparse
import importlib.util
import json
import os
from pathlib import Path
import signal
import subprocess
import time
import tempfile

ROOT = Path(__file__).resolve().parents[1]
spec = importlib.util.spec_from_file_location('harness', ROOT / 'scripts/desktop-harness.py')
harness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(harness)


def wait(predicate, label, seconds=20):
    deadline = time.monotonic() + seconds
    while time.monotonic() < deadline:
        value = predicate()
        if value:
            return value
        time.sleep(.1)
    raise RuntimeError('timed out: ' + label)


def sample(pid):
    base = Path('/proc') / str(pid)
    stat = (base / 'stat').read_text().split(') ')[1].split()
    status = dict(line.split(':', 1) for line in (base / 'status').read_text().splitlines() if ':' in line)
    return dict(at=time.monotonic(), ticks=int(stat[11]) + int(stat[12]),
                rss_kib=int(status['VmRSS'].split()[0]), threads=int(status['Threads']),
                fds=len(list((base / 'fd').iterdir())),
                switches=int(status['voluntary_ctxt_switches']) + int(status['nonvoluntary_ctxt_switches']))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run', type=Path, required=True)
    parser.add_argument('--display', default='96')
    parser.add_argument('--baseline', action='store_true')
    parser.add_argument('--theme', choices=('light', 'dark'), default='dark')
    args = parser.parse_args()
    run = args.run.resolve()
    env = os.environ.copy()
    apps = []
    started = False
    assertions = []
    log = None
    setup = tempfile.TemporaryDirectory(prefix='knave-header-config-')
    source_config = Path(setup.name) / 'config.toml'
    source_config.write_text('[appearance.theme]\nmode = "' + args.theme + '"\n')
    if not args.baseline:
        with source_config.open('a') as config:
            for app_id in ('org.gnome.Evince', 'com.github.johnfactotum.Foliate'):
                config.write('\n[[compositor.window_rules.application]]\napp_id = "' + app_id + '"\nshared_header = "gtk"\n')
    def call(command, *values):
        if command == 'screenshot':
            time.sleep(.3)
        result = harness.request(run, command, list(values))
        if command == 'screenshot':
            time.sleep(.9)
        return result
    def windows():
        return call('snapshot')['payload']['windows']
    def check(condition, label):
        if not condition:
            raise AssertionError(label)
        assertions.append(label)
    try:
        subprocess.run(['python3', str(ROOT / 'scripts/desktop-harness.py'), '--run', str(run),
                        'start', '--profile', 'debug', '--display', args.display, '--no-vnc', '--config', str(source_config)],
                       env=env, check=True, capture_output=True, text=True, timeout=60)
        started = True
        manifest = json.loads((run / 'manifest.json').read_text())
        runtime = manifest['runtime']
        compositor = None
        for process in Path('/proc').iterdir():
            if not process.name.isdigit():
                continue
            try:
                if b'--internal\0compositor' in (process / 'cmdline').read_bytes() and runtime.encode() in (process / 'environ').read_bytes():
                    compositor = int(process.name)
                    break
            except OSError:
                continue
        if compositor is None:
            raise RuntimeError('isolated compositor not found')
        app_env = dict(item.decode().split('=', 1) for item in Path(f'/proc/{compositor}/environ').read_bytes().split(b'\0') if b'=' in item)
        socket = next(Path(runtime).glob('knave/desktop-wayland-*.sock'))
        app_env.pop('GTK_THEME', None)
        app_env.update(WAYLAND_DISPLAY=socket.name[len('desktop-'):-len('.sock')],
                       KNAVE_SOCKET=str(socket), GDK_BACKEND='wayland', GSK_RENDERER='cairo',
                       HDY_DEBUG_COLOR_SCHEME='prefer-' + args.theme,
                       ADW_DEBUG_COLOR_SCHEME='prefer-' + args.theme)
        for version in ('gtk-3.0', 'gtk-4.0'):
            gtk = Path(app_env['XDG_CONFIG_HOME']) / version
            gtk.mkdir(exist_ok=True)
            for filename in ('gtk.css', 'settings.ini'):
                (gtk / filename).write_bytes((ROOT / 'scripts/fixtures/shared-header' / version / filename).read_bytes())
        subprocess.run(['gsettings', 'set', 'org.gnome.desktop.wm.preferences', 'button-layout', ':'], env=app_env, check=True, timeout=15)
        log = (run / 'evince.log').open('ab')
        def launch(document):
            apps.append(subprocess.Popen(['evince', document], env=app_env, stdout=log,
                                         stderr=log, start_new_session=True))
        launch('/usr/share/doc/speex/manual.pdf')
        wait(lambda: len(windows()) == 1 and windows()[0]['title'] == 'manual.pdf', 'document opens')
        time.sleep(2)
        before = sample(compositor)
        time.sleep(3)
        idle = sample(compositor)
        resources = dict(idle_before=before, idle_after=idle)
        call('screenshot', 'single-window')
        if not args.baseline:
            call('record-start', 'shared-header-demo')
            call('click', 1220, 59, 1)
            wait(lambda: windows()[0]['floating'], 'mouse float')
            check(windows()[0]['floating'], 'float button switches a tiled Evince window to floating')
            call('screenshot', 'floating')
            # Default floating geometry: 853 × 509, centered in usable output.
            call('click', 1006, 186, 1)
            wait(lambda: not windows()[0]['floating'], 'mouse tile')
            check(not windows()[0]['floating'], 'tile button returns Evince to tiled layout')
            call('click', 1244, 59, 1)
            wait(lambda: windows()[0]['maximized'], 'mouse maximize')
            call('click', 1244, 59, 1)
            wait(lambda: not windows()[0]['maximized'], 'mouse restore')
            check(not windows()[0]['maximized'], 'maximize and restore buttons update actual window state')
            call('click', 1196, 59, 1)
            wait(lambda: windows()[0]['minimized'], 'mouse minimize')
            check(windows()[0]['minimized'], 'minimize button hides Evince')
            subprocess.run([str(ROOT / 'target/debug/knave'), 'dispatch', 'restore-minimized'], env=app_env, check=True, capture_output=True)
            wait(lambda: not windows()[0]['minimized'], 'restore setup')
            call('click', 1122, 59, 1)
            call('screenshot', 'app-search')
            call('key', 'Escape')
            call('click', 1163, 59, 1)
            call('screenshot', 'app-menu')
            call('key', 'Escape')
            call('key', 'F11')
            wait(lambda: windows()[0]['fullscreen'], 'fullscreen')
            call('screenshot', 'fullscreen')
            call('key', 'F11')
            wait(lambda: not windows()[0]['fullscreen'], 'leave fullscreen')
            check(not windows()[0]['fullscreen'], 'fullscreen enter and exit preserve application operation')
            launch('/usr/share/doc/ijs/ijs_spec.pdf')
            wait(lambda: len(windows()) == 2, 'second document')
            time.sleep(1)
            call('screenshot', 'two-windows')
            check(len(windows()) == 2, 'two independently managed Evince documents coexist')
            resources['normal'] = sample(compositor)
            subprocess.run(['xdotool', 'mousemove', '1220', '59'], env=app_env, check=True)
            time.sleep(2)
            call('screenshot', 'controls-after-hover')
            for _ in range(10):
                call('click', 1220, 59, 1)
                wait(lambda: any(w['floating'] for w in windows()), 'stress float')
                call('click', 1006, 186, 1)
                wait(lambda: not any(w['floating'] for w in windows()), 'stress tile')
            resources['stress'] = sample(compositor)
            time.sleep(2)
            resources['recovery_before'] = sample(compositor)
            time.sleep(3)
            resources['recovery_after'] = sample(compositor)
            call('click', 1268, 59, 1)
            wait(lambda: len(windows()) == 1, 'close one document')
            check(len(windows()) == 1, 'close button removes only its target document')
            call('screenshot', 'remaining-window')
            # Presentation rules are live; placement remains on-open.
            config_path = run / 'config.toml'
            original_config = config_path.read_text()
            def reload_config(contents):
                replacement = config_path.with_suffix('.new')
                replacement.write_text(contents)
                replacement.replace(config_path)
                subprocess.run([str(ROOT / 'target/debug/knave'), 'reload'], env=app_env,
                               check=True, capture_output=True, timeout=15)
            reload_config(original_config.replace('"gtk"', '"off"'))
            call('screenshot', 'rules-disabled')
            reload_config(original_config)
            call('screenshot', 'rules-restored')
            check(True, 'shared-header rule changes reload successfully for existing windows')
            # GTK4/libadwaita uses the same reserved header and compositor actions.
            apps.append(subprocess.Popen(['foliate'], env=app_env, stdout=log,
                                         stderr=log, start_new_session=True))
            foliate_id = 'com.github.johnfactotum.Foliate'
            def foliate_window():
                return next((window for window in windows() if window['app_id'] == foliate_id), None)
            wait(foliate_window, 'Foliate opens')
            time.sleep(2)
            call('screenshot', 'evince-and-foliate')
            subprocess.run(['xdotool', 'mousemove', '1220', '59'], env=app_env, check=True)
            time.sleep(2)
            call('click', 1220, 59, 1)
            wait(lambda: foliate_window()['floating'], 'Foliate mouse float')
            call('screenshot', 'foliate-floating')
            call('click', 1006, 186, 1)
            wait(lambda: not foliate_window()['floating'], 'Foliate mouse tile')
            call('click', 1244, 59, 1)
            wait(lambda: foliate_window()['maximized'], 'Foliate mouse maximize')
            call('click', 1244, 59, 1)
            wait(lambda: not foliate_window()['maximized'], 'Foliate mouse restore')
            call('click', 1196, 59, 1)
            wait(lambda: foliate_window()['minimized'], 'Foliate mouse minimize')
            subprocess.run([str(ROOT / 'target/debug/knave'), 'dispatch', 'restore-minimized'], env=app_env, check=True, capture_output=True)
            wait(lambda: not foliate_window()['minimized'], 'Foliate restores')
            call('screenshot', 'foliate-controls')
            call('click', 1268, 59, 1)
            wait(lambda: foliate_window() is None, 'Foliate close')
            check(True, 'GTK4/libadwaita Foliate supports float, tile, maximize, restore, minimize and close')
            call('record-stop')
        (run / 'resources.json').write_text(json.dumps(resources, indent=2) + '\n')
        (run / 'assertions.json').write_text(json.dumps(assertions, indent=2) + '\n')
        print(json.dumps(dict(run=str(run), assertions=assertions, resources=resources), indent=2))
    finally:
        for app in apps:
            if app.poll() is None:
                os.killpg(app.pid, signal.SIGTERM)
                try:
                    app.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    os.killpg(app.pid, signal.SIGKILL)
                    app.wait(timeout=5)
        if log:
            log.close()
        if started:
            call('stop')
        setup.cleanup()


if __name__ == '__main__':
    main()
