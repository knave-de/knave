"""Deterministic workspace and bundle operations for knave-dev (Python 3.12+)."""

import contextlib
import datetime
import fcntl
import hashlib
import json
import os
import re
import shutil
import socket
import subprocess
import tarfile
import tempfile
import tomllib
import uuid
from pathlib import Path

REPOSITORIES = ('knave', 'villain', 'knaveshell', 'xdg-desktop-portal-knave')
BINARIES = {
    'knave': ('knave', 'bin/knave'),
    'knave-session': ('knave', 'bin/knave-session'),
    'knavectl': ('knave', 'bin/knavectl'),
    'villain': ('villain', 'bin/villain'),
    'knave-shell': ('knaveshell', 'bin/knave-shell'),
    'xdg-desktop-portal-knave': (
        'xdg-desktop-portal-knave',
        'libexec/xdg-desktop-portal-knave',
    ),
}


def now():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def identifier(value):
    if not re.fullmatch(r'[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}', value):
        raise ValueError('name must be 1–64 letters, digits, underscores or hyphens')
    return value


def capture(argv, cwd=None):
    result = subprocess.run(
        list(map(str, argv)),
        cwd=cwd,
        check=True,
        text=True,
        capture_output=True,
        timeout=60,
    )
    return result.stdout.strip()


def git(repo, *args):
    return capture(['git', '-C', repo, *args])


def digest(path):
    with path.open('rb') as stream:
        return hashlib.file_digest(stream, 'sha256').hexdigest()


def atomic_json(path, value):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temporary = tempfile.mkstemp(dir=path.parent, prefix='.' + path.name)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(value, stream, indent=2)
            stream.write('\n')
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temporary, path)
    finally:
        Path(temporary).unlink(missing_ok=True)


@contextlib.contextmanager
def lock(path):
    path.parent.mkdir(parents=True, exist_ok=True)
    with path.open('a') as stream:
        try:
            fcntl.flock(stream, fcntl.LOCK_EX | fcntl.LOCK_NB)
        except BlockingIOError:
            raise RuntimeError(f'operation already active: {path}') from None
        yield stream


def clean_revision(repo):
    if git(repo, 'status', '--porcelain', '--untracked-files=all'):
        raise ValueError(f'commit or remove worktree changes before building: {repo}')
    files = git(repo, 'ls-files', '--stage')
    if files.startswith('160000 ') or '\n160000 ' in files:
        raise ValueError(f'submodules need an explicit snapshot policy: {repo}')
    return git(repo, 'rev-parse', 'HEAD')


def workspace(state, name):
    root = state / 'workspaces' / identifier(name)
    data = json.loads((root / 'workspace.json').read_text())
    if data['schema'] != 1 or set(data['repositories']) != set(REPOSITORIES):
        raise ValueError('unsupported workspace manifest')
    return root, data


def feature_config(state, name):
    """Seed editable feature configuration once; subsequent sessions preserve it."""
    directory = state / 'workspaces' / identifier(name) / 'config'
    with lock(state / 'config.lock'):
        defaults = state / 'config'
        defaults.mkdir(parents=True, exist_ok=True)
        default = defaults / 'config.toml'
        if not default.exists():
            default.write_text(
                'schema_version = 2\n[compositor]\nenvironment_file = "environment"\n'
            )
        environment = defaults / 'environment'
        if not environment.exists():
            environment.write_text(
                '# KEY=value settings for newly launched applications\n'
            )
        directory.mkdir(parents=True, exist_ok=True)
        target = directory / 'config.toml'
        if not target.exists():
            shutil.copyfile(default, target)
        if not (directory / 'environment').exists():
            shutil.copyfile(environment, directory / 'environment')
        return target


def create(state, name, sources, changed, revisions):
    identifier(name)
    if not changed or not set(changed) <= set(REPOSITORIES):
        raise ValueError('select at least one affected repository with --change')
    root = state / 'workspaces' / name
    existing_config = root.exists()
    if existing_config and any(child.name != 'config' for child in root.iterdir()):
        raise FileExistsError(f'workspace already exists: {root}')
    root.mkdir(parents=True, exist_ok=True)
    created = []
    data = {'schema': 1, 'name': name, 'created_at': now(), 'repositories': {}}
    try:
        for repo in REPOSITORIES:
            source = sources / repo
            # Resolve first, so moving source branches cannot alter the selection.
            revision = git(
                source,
                'rev-parse',
                '--verify',
                revisions.get(repo, 'HEAD') + '^{commit}',
            )
            path = root / repo
            branch = f'dev/{name}' if repo in changed else None
            args = ['worktree', 'add'] + (['-b', branch] if branch else ['--detach'])
            git(source, *args, str(path), revision)
            created.append((source, path, branch))
            data['repositories'][repo] = {
                'source': str(source),
                'base': revision,
                'branch': branch,
            }
        feature_config(state, name)
        atomic_json(root / 'workspace.json', data)
    except BaseException:
        # Only remove worktrees/branches created by this invocation.
        for source, path, branch in reversed(created):
            git(source, 'worktree', 'remove', str(path))
            if branch:
                git(source, 'branch', '-D', branch)
        if not existing_config:
            shutil.rmtree(root)
        raise
    return root


def remove(state, name):
    root, data = workspace(state, name)
    with lock(root / 'operation.lock'):
        for repo, entry in data['repositories'].items():
            path = root / repo
            clean_revision(path)
            if git(path, 'rev-parse', 'HEAD') != entry['base']:
                raise ValueError(
                    f'{repo} has feature commits; retain it or remove its worktree manually'
                )
        for repo, entry in data['repositories'].items():
            git(Path(entry['source']), 'worktree', 'remove', str(root / repo))
        # Keep branches, even empty ones: this command never deletes feature history.
    for child in root.iterdir():
        if child.name == 'config':
            continue
        if child.is_dir() and not child.is_symlink():
            shutil.rmtree(child)
        else:
            child.unlink()
    if not any(root.iterdir()):
        root.rmdir()


def command(argv, cwd, env, log):
    print(' '.join(map(str, argv)), flush=True)
    with log.open('a') as stream:
        stream.write('\n$ ' + ' '.join(map(str, argv)) + '\n')
        stream.flush()
        result = subprocess.run(
            list(map(str, argv)),
            cwd=cwd,
            env=env,
            stdout=stream,
            stderr=subprocess.STDOUT,
        )
    if result.returncode:
        raise RuntimeError(f'command failed ({result.returncode}); see {log}')


def snapshot(repo, revision, target):
    target.mkdir()
    with tempfile.TemporaryFile() as stream:
        subprocess.run(
            ['git', '-C', str(repo), 'archive', revision], stdout=stream, check=True
        )
        stream.seek(0)
        with tarfile.open(fileobj=stream) as archive:
            archive.extractall(target, filter='data')


def patch_config(sources):
    patches = {}
    shared = {}
    for manifest in sorted((sources / 'knave' / 'crates').glob('*/Cargo.toml')):
        name = tomllib.loads(manifest.read_text())['package']['name']
        shared[name] = manifest.parent
    for repo in REPOSITORIES:
        for manifest in (sources / repo).rglob('Cargo.toml'):
            data = tomllib.loads(manifest.read_text())
            tables = [data, data.get('workspace', {})]
            tables.extend(data.get('target', {}).values())
            for table in tables:
                for kind in ('dependencies', 'build-dependencies', 'dev-dependencies'):
                    for name, dep in table.get(kind, {}).items():
                        if (
                            isinstance(dep, dict)
                            and dep.get('package', name) in shared
                            and 'git' in dep
                        ):
                            patches.setdefault(dep['git'], {})[
                                dep.get('package', name)
                            ] = shared[dep.get('package', name)]
    lines = []
    for url, packages in sorted(patches.items()):
        lines.append(f'[patch.{json.dumps(url)}]')
        lines.extend(
            f'{json.dumps(name)} = {{ path = {json.dumps(str(path))} }}'
            for name, path in sorted(packages.items())
        )
    return '\n'.join(lines) + '\n', shared


def build(state, name, profile='debug', jobs=1):
    if not 1 <= jobs <= 8:
        raise ValueError('--jobs must be between 1 and 8 (default: 1)')
    root, data = workspace(state, name)
    with lock(state / 'build.lock'), lock(root / 'operation.lock'):
        revisions = {repo: clean_revision(root / repo) for repo in REPOSITORIES}
        for repo, entry in data['repositories'].items():
            if not entry['branch'] and revisions[repo] != entry['base']:
                raise ValueError(
                    f'unchanged repository moved from its fixed revision: {repo}'
                )
        build_id = (
            datetime.datetime.now(datetime.timezone.utc).strftime('%Y%m%dT%H%M%S')
            + '-'
            + uuid.uuid4().hex[:8]
        )
        output = root / 'builds' / build_id
        sources = output / 'sources'
        sources.mkdir(parents=True)
        manifest = {
            'schema': 1,
            'id': f'{name[:32]}-{build_id}',
            'feature': name,
            'created_at': now(),
            'profile': profile,
            'jobs': jobs,
            'repositories': {},
            'checks': [],
            'binaries': {},
            'complete': False,
        }
        atomic_json(output / 'build.json', manifest)
        # A failed latest attempt must not silently hand back an older build.
        atomic_json(root / 'latest-build.json', {'path': str(output)})
        for repo, revision in revisions.items():
            snapshot(root / repo, revision, sources / repo)
            manifest['repositories'][repo] = {
                'commit': revision,
                'base': data['repositories'][repo]['base'],
                'source_lock_sha256': digest(sources / repo / 'Cargo.lock'),
            }
        config, shared = patch_config(sources)
        config_path = output / 'cargo-config.toml'
        config_path.write_text(config)
        manifest['cargo_config'] = config
        manifest['rustc'] = capture(['rustc', '-Vv'])
        manifest['cargo'] = capture(['cargo', '-V'])
        # Avoid an ambient target/profile override silently changing artifact paths.
        env = {
            k: v
            for k, v in os.environ.items()
            if not k.startswith(('CARGO_', 'RUSTFLAGS', 'RUSTDOCFLAGS'))
        }
        env.update(
            CARGO_BUILD_JOBS=str(jobs),
            CARGO_PROFILE_DEV_DEBUG='0',
            CARGO_PROFILE_TEST_DEBUG='0',
        )
        manifest['build_environment'] = {
            k: env[k]
            for k in (
                'CARGO_BUILD_JOBS',
                'CARGO_PROFILE_DEV_DEBUG',
                'CARGO_PROFILE_TEST_DEBUG',
            )
        }
        for repo in REPOSITORIES:
            directory = sources / repo
            env['CARGO_TARGET_DIR'] = str(state / 'targets' / repo)
            cargo = ['cargo']
            # Resolve the local patches in snapshot locks only. Subsequent steps are locked.
            metadata = subprocess.run(
                cargo
                + ['metadata', '--config', str(config_path), '--format-version', '1'],
                cwd=directory,
                env=env,
                capture_output=True,
                text=True,
                check=False,
            )
            (output / f'{repo}-metadata.json').write_text(metadata.stdout)
            (output / f'{repo}-resolution.log').write_text(metadata.stderr)
            if metadata.returncode:
                raise RuntimeError(
                    f'dependency resolution failed; see {output / (repo + "-resolution.log")}'
                )
            resolved = json.loads(metadata.stdout)
            for package in resolved['packages']:
                if package['source'] is None and not Path(
                    package['manifest_path']
                ).resolve().is_relative_to(sources):
                    raise ValueError(
                        f'{repo} resolved an unrecorded local dependency: {package["name"]}'
                    )
                if (
                    package['name'] in shared
                    and Path(package['manifest_path']).resolve()
                    != shared[package['name']] / 'Cargo.toml'
                ):
                    raise ValueError(
                        f'{repo} resolved {package["name"]} outside the coordinated snapshot'
                    )
            manifest['repositories'][repo]['resolved_lock_sha256'] = digest(
                directory / 'Cargo.lock'
            )
            log = output / f'{repo}.log'
            for operation in (
                ['fmt', '--all', '--', '--check'],
                ['test', '--workspace', '--all-targets', '--locked'],
                [
                    'clippy',
                    '--workspace',
                    '--all-targets',
                    '--locked',
                    '--',
                    '-D',
                    'warnings',
                ],
                ['build', '--workspace', '--bins', '--locked']
                + (['--release'] if profile == 'release' else []),
            ):
                # External Cargo subcommands (notably Clippy) must receive the
                # config after their name; Cargo does not forward global flags.
                configured = (
                    operation
                    if operation[0] == 'fmt'
                    else [operation[0], '--config', str(config_path), *operation[1:]]
                )
                command(cargo + configured, directory, env, log)
                manifest['checks'].append({'repository': repo, 'command': operation})
                atomic_json(output / 'build.json', manifest)
        artifacts = output / 'artifacts'
        for binary, (repo, destination) in BINARIES.items():
            source = state / 'targets' / repo / profile / binary
            target = artifacts / destination
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(source, target)
            target.chmod(0o555)
            manifest['binaries'][binary] = {
                'path': destination,
                'sha256': digest(target),
            }
        for repo in REPOSITORIES:
            shutil.copyfile(
                sources / repo / 'Cargo.lock', output / f'{repo}.Cargo.lock'
            )
        manifest['complete'] = True
        atomic_json(output / 'build.json', manifest)
        atomic_json(root / 'latest-build.json', {'path': str(output)})
        return output


def bundle(state, name):
    root, _ = workspace(state, name)
    with lock(root / 'operation.lock'):
        output = Path(json.loads((root / 'latest-build.json').read_text())['path'])
        manifest = json.loads((output / 'build.json').read_text())
        if not manifest['complete']:
            raise ValueError('build did not complete')
        parent = state / 'bundles'
        parent.mkdir(parents=True, exist_ok=True)
        target = parent / identifier(manifest['id'])
        if target.exists():
            verify_bundle(target)
            return target
        staging = Path(tempfile.mkdtemp(prefix='.bundle-', dir=parent))
        try:
            shutil.copytree(output / 'artifacts', staging, dirs_exist_ok=True)
            for binary, entry in manifest['binaries'].items():
                if digest(staging / entry['path']) != entry['sha256']:
                    raise ValueError(f'build artifact changed: {binary}')
            sources = output / 'sources'
            for source, destination in (
                (sources / 'villain/share', 'share'),
                (
                    sources / 'xdg-desktop-portal-knave/data/knave.portal',
                    'share/xdg-desktop-portal/portals/knave.portal',
                ),
                (
                    sources / 'xdg-desktop-portal-knave/data/knave-portals.conf',
                    'share/xdg-desktop-portal/knave-portals.conf',
                ),
            ):
                if source.is_dir():
                    shutil.copytree(source, staging / destination, dirs_exist_ok=True)
                else:
                    path = staging / destination
                    path.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copyfile(source, path)
            provenance = staging / 'provenance'
            provenance.mkdir()
            for repo in REPOSITORIES:
                shutil.copyfile(
                    output / f'{repo}.Cargo.lock', provenance / f'{repo}.Cargo.lock'
                )
            shutil.copyfile(
                output / 'cargo-config.toml', provenance / 'cargo-config.toml'
            )
            # Keep the test runner version with the binaries it will exercise.
            for script in (
                'desktop-harness.py',
                'desktop-harness-smoke.py',
                'knave_dev.py',
                'knave-dev.py',
                'knave_dev_tty.py',
            ):
                path = staging / 'tools' / script
                path.parent.mkdir(exist_ok=True)
                shutil.copyfile(sources / 'knave/scripts' / script, path)
            launcher = sources / 'knave/knave-dev'
            if launcher.is_file():
                shutil.copyfile(launcher, staging / 'knave-dev')
                (staging / 'knave-dev').chmod(0o555)
            manifest['files'] = {
                str(path.relative_to(staging)): digest(path)
                for path in sorted(staging.rglob('*'))
                if path.is_file()
            }
            (staging / 'bundle.json').write_text(json.dumps(manifest, indent=2) + '\n')
            for path in staging.rglob('*'):
                if path.is_file():
                    path.chmod(0o555 if os.access(path, os.X_OK) else 0o444)
            staging.rename(target)
            for path in target.rglob('*'):
                if path.is_dir():
                    path.chmod(0o555)
            target.chmod(0o555)
        finally:
            if staging.exists():
                shutil.rmtree(staging)
        return target


def verify_bundle(path):
    path = path.resolve(strict=True)
    manifest = json.loads((path / 'bundle.json').read_text())
    if manifest.get('schema') != 1 or not manifest.get('complete'):
        raise ValueError('unsupported or incomplete bundle')
    files = manifest['files']
    members = list(path.rglob('*'))
    if any(
        item.is_symlink() or not (item.is_file() or item.is_dir()) for item in members
    ):
        raise ValueError('invalid bundle member: links and special files are forbidden')
    actual = {str(item.relative_to(path)) for item in members if item.is_file()}
    if actual != set(files) | {'bundle.json'}:
        raise ValueError('bundle file inventory changed')
    for name, expected in files.items():
        item = path / name
        if (
            Path(name).is_absolute()
            or '..' in Path(name).parts
            or item.is_symlink()
            or not item.resolve().is_relative_to(path)
        ):
            raise ValueError(f'invalid bundle member: {name}')
        if digest(item) != expected:
            raise ValueError(f'bundle hash mismatch: {name}')
    for binary, (_, destination) in BINARIES.items():
        entry = manifest['binaries'][binary]
        if (
            entry['path'] != destination
            or files[destination] != entry['sha256']
            or not os.access(path / destination, os.X_OK)
        ):
            raise ValueError(f'invalid bundle binary: {binary}')
    return manifest


def tested_bundle(state, path):
    manifest = verify_bundle(path)
    evidence = state / 'verification' / (manifest['id'] + '.json')
    result = json.loads(evidence.read_text())
    if not result.get('passed') or result['bundle_sha256'] != digest(
        path / 'bundle.json'
    ):
        raise ValueError('bundle has no matching successful nested test')
    return manifest


def session_bundle(state, feature):
    archive = Path(feature).expanduser()
    if archive.is_file():
        return import_bundle(state, archive.resolve())
    if (
        archive.is_absolute()
        or len(archive.parts) > 1
        or feature.endswith(('.tar', '.tar.gz', '.tgz'))
    ):
        raise ValueError(f'Feature archive not found: {archive}')
    identifier(feature)
    candidates = []
    for path in (state / 'bundles').glob('*/bundle.json'):
        if path.parent.name.startswith('.'):
            continue
        manifest = json.loads(path.read_text())
        if manifest.get('feature') != feature:
            continue
        evidence = state / 'verification' / (path.parent.name + '.json')
        if (
            not evidence.is_file()
            or json.loads(evidence.read_text()).get('passed') is not True
        ):
            continue
        candidates.append((manifest['created_at'], path.parent.name, path.parent))
    if not candidates:
        raise ValueError(
            f'Feature {feature!r} is not ready for testing: no verified bundle. '
            'Pass its exported archive to session to import and start it.'
        )
    selected = max(candidates)[2]
    # Reject a damaged newest verified build instead of silently launching an older one.
    tested_bundle(state, selected)
    return selected


def preview_bundle(state, path, config=None):
    manifest = tested_bundle(state, path)
    feature = identifier(manifest['feature'])
    record = state / 'previews' / (feature + '.json')
    with lock(state / 'preview.lock'):
        if record.exists():
            previous = json.loads(record.read_text())
            run = Path(previous['run'])
            if not (run / 'stopped.json').exists():
                # Reuse the live preview; an unreachable daemon is reported rather
                # than abandoned while launching another desktop.
                result = json.loads(capture(previous['command'] + ['status']))
                return dict(result, feature=feature)
        display = next(
            (
                number
                for number in range(92, 192)
                if not Path(f'/tmp/.X11-unix/X{number}').exists()
                and not Path(f'/tmp/.X{number}-lock').exists()
            ),
            None,
        )
        if display is None:
            raise RuntimeError('no free nested display')
        with socket.socket() as reservation:
            reservation.bind(('127.0.0.1', 0))
            port = reservation.getsockname()[1]
        run = Path(tempfile.mkdtemp(prefix='knave-preview-')) / 'run'
        command = ['python3', str(path / 'tools/desktop-harness.py'), '--run', str(run)]
        options = ['--config', str(config.resolve(strict=True))] if config else []
        result = json.loads(
            capture(
                command
                + [
                    'start',
                    '--bundle',
                    str(path),
                    '--display',
                    str(display),
                    '--vnc-port',
                    str(port),
                    *options,
                ]
            )
        )
        try:
            atomic_json(
                record, {'run': str(run), 'bundle': str(path), 'command': command}
            )
        except BaseException:
            capture(command + ['stop'])
            raise
        return dict(result, feature=feature)


def stop_preview(state, feature):
    record = state / 'previews' / (identifier(feature) + '.json')
    with lock(state / 'preview.lock'):
        if not record.is_file():
            raise ValueError(f'No nested preview for {feature!r}')
        previous = json.loads(record.read_text())
        if not (Path(previous['run']) / 'stopped.json').exists():
            capture(previous['command'] + ['stop'])


def test_bundle(state, path, display=93, vnc_port=0):
    manifest = verify_bundle(path)
    # Short socket paths also work when the workspace has a long feature name.
    directory = Path(tempfile.mkdtemp(prefix='knave-check-')) / 'run'
    log = directory.parent / 'test.log'
    argv = [
        'python3',
        path / 'tools/desktop-harness-smoke.py',
        '--run',
        directory,
        '--bundle',
        path,
        '--display',
        str(display),
        '--vnc-port',
        str(vnc_port),
    ]
    command(argv, path, os.environ.copy(), log)
    verify_bundle(path)
    report = json.loads((directory / 'smoke-result.json').read_text())
    if report.get('passed') is not True:
        raise RuntimeError(f'smoke test failed: {directory}')
    saved = state / 'runs' / (manifest['id'] + '-' + uuid.uuid4().hex[:8])
    saved.parent.mkdir(parents=True, exist_ok=True)
    shutil.move(str(directory.parent), saved)
    directory = saved / 'run'
    report['video'] = str(directory / Path(report['video']).name)
    result = dict(
        report,
        bundle=str(path),
        bundle_sha256=digest(path / 'bundle.json'),
        run=str(directory),
        tested_at=now(),
        coverage='nested software rendering; TTY/hardware untested',
    )
    atomic_json(state / 'verification' / (manifest['id'] + '.json'), result)
    return result


def export_bundle(state, path, destination):
    manifest = tested_bundle(state, path)
    evidence = state / 'verification' / (manifest['id'] + '.json')
    # Exclusive creation: never overwrite an earlier handoff.
    with (
        destination.open('xb') as stream,
        tarfile.open(fileobj=stream, mode='w:gz') as archive,
    ):
        archive.add(path, arcname='bundle')
        archive.add(evidence, arcname='verification.json')
        result = json.loads(evidence.read_text())
        if result.get('run'):
            archive.add(Path(result['run']), arcname='evidence')
    return destination


def import_bundle(state, archive_path):
    parent = state / 'bundles'
    parent.mkdir(parents=True, exist_ok=True)
    staging = Path(tempfile.mkdtemp(prefix='.import-', dir=parent))
    try:
        with tarfile.open(archive_path) as archive:
            # Bundles contain regular files/directories only, never devices or links.
            for member in archive.getmembers():
                if not (member.isfile() or member.isdir()):
                    raise ValueError(f'unsupported archive member: {member.name}')
            archive.extractall(staging, filter='data')
        source = staging / 'bundle'
        manifest = verify_bundle(source)
        identifier(manifest['id'])
        evidence = json.loads((staging / 'verification.json').read_text())
        if not evidence.get('passed') or evidence['bundle_sha256'] != digest(
            source / 'bundle.json'
        ):
            raise ValueError('handoff has no matching successful nested verification')
        target = parent / manifest['id']
        if target.exists():
            verify_bundle(target)
            if digest(target / 'bundle.json') != digest(source / 'bundle.json'):
                raise ValueError('bundle ID already exists with different contents')
        else:
            source.rename(target)
        evidence['bundle'] = str(target)
        if (staging / 'evidence').is_dir():
            saved = (
                state / 'runs' / (manifest['id'] + '-import-' + uuid.uuid4().hex[:8])
            )
            saved.parent.mkdir(parents=True, exist_ok=True)
            (staging / 'evidence').rename(saved)
            evidence['run'] = str(saved)
            if 'video' in evidence:
                evidence['video'] = str(saved / Path(evidence['video']).name)
        atomic_json(state / 'verification' / (manifest['id'] + '.json'), evidence)
        return target
    finally:
        # Imported directories can already be read-only.
        for item in staging.rglob('*'):
            if item.is_dir():
                item.chmod(0o700)
        staging.chmod(0o700)
        shutil.rmtree(staging)


def pull_requests(state, name, bundle_path, base='master'):
    root, data = workspace(state, name)
    manifest = tested_bundle(state, bundle_path)
    with lock(root / 'operation.lock'):
        changed = []
        for repo, entry in data['repositories'].items():
            revision = clean_revision(root / repo)
            if revision != manifest['repositories'][repo]['commit']:
                raise ValueError(
                    f'{repo} moved since bundle verification; build and test again'
                )
            if entry['branch'] and revision != entry['base']:
                if git(root / repo, 'branch', '--show-current') != entry['branch']:
                    raise ValueError(f'{repo} is not on its managed feature branch')
                changed.append(repo)
        if not changed:
            raise ValueError('no committed feature changes to publish')
        for repo in changed:
            directory = root / repo
            author = git(directory, 'var', 'GIT_AUTHOR_IDENT').split('>')[0]
            committer = git(directory, 'var', 'GIT_COMMITTER_IDENT').split('>')[0]
            if author != committer:
                raise ValueError(f'{repo}: author and committer identities differ')
            publication_base = git(
                directory, 'rev-parse', '--verify', 'origin/' + base + '^{commit}'
            )
            outgoing = git(
                directory, 'log', '--format=fuller', publication_base + '..HEAD'
            )
            (root / f'{repo}-commit-audit.txt').write_text(outgoing + '\n')
            if re.search(
                r'(?im)^\s*(?:Author|Commit|Co-authored-by|Signed-off-by):.*\b(?:codex|claude|openai|chatgpt)\b',
                outgoing,
            ):
                raise ValueError(
                    f'{repo}: assistant attribution in outgoing commits; inspect the commit audit'
                )
        urls = {}
        for repo in changed:
            directory = root / repo
            entry = data['repositories'][repo]
            repository = capture(
                [
                    'gh',
                    'repo',
                    'view',
                    '--json',
                    'nameWithOwner',
                    '--jq',
                    '.nameWithOwner',
                ],
                cwd=directory,
            )
            git(directory, 'push', '--set-upstream', 'origin', entry['branch'])
            # Check GitHub's published attribution, not only local Git config.
            for revision in git(
                directory, 'rev-list', 'origin/' + base + '..HEAD'
            ).splitlines():
                local = git(
                    directory,
                    'show',
                    '-s',
                    '--format=%an%x00%ae%x00%cn%x00%ce%x00%B',
                    revision,
                ).split('\x00', 4)
                remote = json.loads(
                    capture(
                        ['gh', 'api', f'repos/{repository}/commits/{revision}'],
                        cwd=directory,
                    )
                )['commit']
                published = [
                    remote['author']['name'],
                    remote['author']['email'],
                    remote['committer']['name'],
                    remote['committer']['email'],
                    remote['message'].strip(),
                ]
                if [*local[:4], local[4].strip()] != published:
                    raise ValueError(
                        f'{repo}: published commit attribution/message differs: {revision}'
                    )
            found = json.loads(
                capture(
                    [
                        'gh',
                        'pr',
                        'list',
                        '--repo',
                        repository,
                        '--head',
                        entry['branch'],
                        '--base',
                        base,
                        '--state',
                        'open',
                        '--json',
                        'url',
                    ],
                    cwd=directory,
                )
            )
            body = (
                f'Coordinated feature: {name}.\n\n'
                f'Tested bundle: `{manifest["id"]}`\n'
                f'Bundle manifest SHA-256: `{digest(bundle_path / "bundle.json")}`\n\n'
                'All four repositories passed formatting, tests, strict Clippy and builds. '
                'Nested Winit verification covers launcher/input/workspaces and private portal activation. '
                'Direct TTY/hardware verification remains a human review step.\n\n'
                'Component revisions:\n'
                + '\n'.join(
                    f'- {key}: `{value["commit"]}`'
                    for key, value in manifest['repositories'].items()
                )
                + '\n\nRollout: test the exact bundle in the dedicated account. Rollback: select the previous tested bundle and start a fresh session. '
                'This tooling does not merge pull requests.\n'
            )
            body_path = root / f'{repo}-pr.md'
            body_path.write_text(body)
            if found:
                url = found[0]['url']
            else:
                url = capture(
                    [
                        'gh',
                        'pr',
                        'create',
                        '--repo',
                        repository,
                        '--head',
                        entry['branch'],
                        '--base',
                        base,
                        '--draft',
                        '--title',
                        f'feat: {name.replace("-", " ")}',
                        '--body-file',
                        str(body_path),
                    ],
                    cwd=directory,
                )
            urls[repo] = url
            atomic_json(root / 'pull-requests.json', urls)
        # Keep cross-links in descriptions and preserve existing human text.
        if len(urls) > 1:
            links_path = root / 'pr-links.md'
            links_path.write_text(
                'Coordinated pull requests:\n\n'
                + '\n'.join(f'- {repo}: {url}' for repo, url in urls.items())
                + '\n'
            )
            for repo, url in urls.items():
                current_body = capture(
                    ['gh', 'pr', 'view', url, '--json', 'body', '--jq', '.body'],
                    cwd=root / repo,
                )
                begin, end = '<!-- knave-dev-links -->', '<!-- /knave-dev-links -->'
                section = begin + '\n' + links_path.read_text() + end
                if begin in current_body and end in current_body:
                    current_body = re.sub(
                        re.escape(begin) + '.*?' + re.escape(end),
                        lambda _: section,
                        current_body,
                        flags=re.DOTALL,
                    )
                else:
                    current_body += '\n\n' + section + '\n'
                body_path = root / f'{repo}-linked-pr.md'
                body_path.write_text(current_body)
                capture(
                    ['gh', 'pr', 'edit', url, '--body-file', str(body_path)],
                    cwd=root / repo,
                )
        return urls
