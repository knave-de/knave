"""Real Git/Cargo workflow tests plus bundle/TTY lifecycle regressions."""

import contextlib
import io
import json
import os
import shutil
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

import knave_dev as dev
import knave_dev_tty as tty

SCRIPTS = Path(__file__).parent


class WorkspaceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='kd-test-')
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.sources = self.root / 'sources'
        self.state = self.root / 'state'
        self.sources.mkdir()
        for repo in dev.REPOSITORIES:
            path = self.sources / repo
            path.mkdir()
            dev.capture(['git', 'init', '-q', '-b', 'master', path])
            dev.git(path, 'config', 'user.name', 'Fixture Contributor')
            dev.git(path, 'config', 'user.email', 'fixture@example.invalid')
            (path / 'README').write_text(repo)
            self.commit(path)

    def commit(self, path):
        dev.git(path, 'add', '.')
        dev.git(path, 'commit', '-qm', 'test: fixture')

    def create(self, name='feature'):
        return dev.create(self.state, name, self.sources, ['knave'], {})

    def test_create_preserves_dirty_sources_and_pins_unchanged_repos(self):
        source = self.sources / 'villain'
        (source / 'README').write_text('uncommitted source')
        workspace = self.create()
        self.assertEqual((workspace / 'villain/README').read_text(), 'villain')
        self.assertEqual((source / 'README').read_text(), 'uncommitted source')
        self.assertEqual(
            dev.git(workspace / 'knave', 'branch', '--show-current'), 'dev/feature'
        )
        self.assertEqual(dev.git(workspace / 'villain', 'branch', '--show-current'), '')
        (source / 'README').write_text('later committed change')
        self.commit(source)
        self.assertNotEqual(
            dev.git(source, 'rev-parse', 'HEAD'),
            dev.git(workspace / 'villain', 'rev-parse', 'HEAD'),
        )

    def test_partial_create_rolls_back_only_new_worktrees(self):
        dev.git(self.sources / 'knaveshell', 'branch', 'dev/collision')
        with self.assertRaises(subprocess.CalledProcessError):
            dev.create(
                self.state, 'collision', self.sources, ['knave', 'knaveshell'], {}
            )
        self.assertFalse((self.state / 'workspaces/collision').exists())
        self.assertNotIn(
            'dev/collision', dev.git(self.sources / 'knave', 'branch', '--list')
        )
        self.assertIn(
            'dev/collision', dev.git(self.sources / 'knaveshell', 'branch', '--list')
        )
        self.assertEqual(
            len(dev.git(self.sources / 'villain', 'worktree', 'list').splitlines()), 1
        )

    def test_dirty_worktree_and_new_commits_prevent_removal(self):
        workspace = self.create()
        (workspace / 'knave/new').write_text('untracked')
        with self.assertRaisesRegex(ValueError, 'worktree changes'):
            dev.remove(self.state, 'feature')
        self.commit(workspace / 'knave')
        with self.assertRaisesRegex(ValueError, 'feature commits'):
            dev.remove(self.state, 'feature')

    def test_remove_leaves_source_and_feature_branch(self):
        self.create()
        dev.remove(self.state, 'feature')
        self.assertFalse((self.state / 'workspaces/feature').exists())
        self.assertIn(
            'dev/feature', dev.git(self.sources / 'knave', 'branch', '--list')
        )

    def test_build_lock_excludes_other_builds(self):
        with dev.lock(self.state / 'build.lock'):
            with self.assertRaisesRegex(RuntimeError, 'already active'):
                with dev.lock(self.state / 'build.lock'):
                    self.fail('concurrent build acquired lock')

    def test_invalid_names_cannot_escape_state(self):
        for name in ('../escape', '/absolute', '.', 'a b', '-flag', 'a' * 65):
            with self.assertRaises(ValueError):
                self.create(name)

    def prepare_cargo(self):
        knave = self.sources / 'knave'
        shared = knave / 'crates/knave-desktop-api'
        shared.mkdir(parents=True)
        (shared / 'src').mkdir()
        (shared / 'Cargo.toml').write_text(
            '[package]\nname = "knave-desktop-api"\nversion = "0.1.0"\nedition = "2024"\n'
        )
        (shared / 'src/lib.rs').write_text('pub fn value() -> u32 {\n    1\n}\n')
        for repo in dev.REPOSITORIES:
            path = self.sources / repo
            (path / '.gitignore').write_text('/target/\n')
            text = f'[package]\nname = "{repo}"\nversion = "0.1.0"\nedition = "2024"\n'
            if repo == 'knave':
                text += '[workspace]\nmembers = ["crates/knave-desktop-api"]\n'
            else:
                text += f'[dependencies]\nknave-desktop-api = {{ git = {json.dumps(knave.as_uri())} }}\n'
            (path / 'Cargo.toml').write_text(text)
            (path / 'src/bin').mkdir(parents=True)
            for binary, (owner, _) in dev.BINARIES.items():
                if owner == repo:
                    body = (
                        'println!("{}", knave_desktop_api::value());'
                        if repo != 'knave'
                        else 'println!("fixture");'
                    )
                    (path / 'src/bin' / f'{binary}.rs').write_text(
                        f'fn main() {{\n    {body}\n}}\n'
                    )
            if repo == 'knave':
                (path / 'scripts').mkdir()
                for name in (
                    'desktop-harness.py',
                    'desktop-harness-smoke.py',
                    'knave_dev.py',
                    'knave-dev.py',
                    'knave_dev_tty.py',
                ):
                    shutil.copyfile(SCRIPTS / name, path / 'scripts' / name)
                self.commit(path)
            elif repo == 'villain':
                data = path / 'share/xdg-desktop-portal'
                data.mkdir(parents=True)
                (data / 'villain-portals.conf').write_text('[preferred]\ndefault=gtk\n')
            elif repo == 'xdg-desktop-portal-knave':
                (path / 'data').mkdir()
                (path / 'data/knave.portal').write_text(
                    '[portal]\nDBusName=org.freedesktop.impl.portal.desktop.knave\n'
                )
                (path / 'data/knave-portals.conf').write_text(
                    '[preferred]\ndefault=knave\n'
                )
            dev.capture(['cargo', 'generate-lockfile'], cwd=path)
            self.commit(path)
        # Feature changes the local API; consumers' lockfiles still reference value 1.
        (shared / 'src/lib.rs').write_text('pub fn value() -> u32 {\n    7\n}\n')
        self.commit(knave)

    def test_pr_publication_checks_exact_revisions_and_reuses_prs(self):
        root = dev.create(self.state, 'publish', self.sources, ['knave', 'villain'], {})
        revisions = {}
        for repo in dev.REPOSITORIES:
            path = root / repo
            dev.git(path, 'update-ref', 'refs/remotes/origin/master', 'HEAD')
            if repo in ('knave', 'villain'):
                (path / 'feature').write_text('feature')
                self.commit(path)
            revisions[repo] = {'commit': dev.git(path, 'rev-parse', 'HEAD')}
        bundle = self.root / 'publication-bundle'
        bundle.mkdir()
        (bundle / 'bundle.json').write_text('{}')
        manifest = {'id': 'tested', 'repositories': revisions}
        invoke = dev.capture
        created = {}
        edits = []

        def capture(argv, cwd=None):
            argv = list(map(str, argv))
            if argv[0] == 'git' and 'push' in argv:
                return ''
            if argv[0] != 'gh':
                return invoke(argv, cwd=cwd)
            repo = cwd.name
            if argv[1:3] == ['repo', 'view']:
                return f'fixture/{repo}'
            if argv[1] == 'api':
                revision = argv[2].rsplit('/', 1)[-1]
                local = dev.git(
                    cwd,
                    'show',
                    '-s',
                    '--format=%an%x00%ae%x00%cn%x00%ce%x00%B',
                    revision,
                ).split('\x00', 4)
                return json.dumps(
                    {
                        'commit': {
                            'author': {'name': local[0], 'email': local[1]},
                            'committer': {'name': local[2], 'email': local[3]},
                            'message': local[4].strip(),
                        }
                    }
                )
            if argv[1:3] == ['pr', 'list']:
                return json.dumps([{'url': created[repo]}] if repo in created else [])
            if argv[1:3] == ['pr', 'create']:
                created[repo] = f'https://github.com/fixture/{repo}/pull/1'
                return created[repo]
            if argv[1:3] == ['pr', 'view']:
                return 'Human review notes.'
            if argv[1:3] == ['pr', 'edit']:
                edits.append(Path(argv[-1]).read_text())
                return ''
            self.fail(f'unexpected publication command: {argv}')

        with (
            patch.object(dev, 'tested_bundle', return_value=manifest),
            patch.object(dev, 'capture', side_effect=capture),
        ):
            urls = dev.pull_requests(self.state, 'publish', bundle)
            self.assertEqual(len(urls), 2)
            self.assertTrue(
                all(
                    'Human review notes.' in body and 'knave-dev-links' in body
                    for body in edits
                )
            )
            self.assertEqual(dev.pull_requests(self.state, 'publish', bundle), urls)
            self.assertEqual(len(created), 2)
            (root / 'knave/unverified').write_text('dirty source')
            with self.assertRaisesRegex(ValueError, 'worktree changes'):
                dev.pull_requests(self.state, 'publish', bundle)

    @unittest.skipUnless(
        shutil.which('cargo'), 'Cargo is required for the integration fixture'
    )
    def test_real_coordinated_debug_and_release_builds_preserve_locks(self):
        self.prepare_cargo()
        workspace = self.create()
        locks = {
            repo: (workspace / repo / 'Cargo.lock').read_bytes()
            for repo in dev.REPOSITORIES
        }
        for profile in ('debug', 'release'):
            with contextlib.redirect_stdout(io.StringIO()):
                output = dev.build(self.state, 'feature', profile)
            manifest = json.loads((output / 'build.json').read_text())
            self.assertEqual(len(manifest['checks']), 16)
            self.assertEqual(dev.capture([output / 'artifacts/bin/villain']), '7')
            bundle = dev.bundle(self.state, 'feature')
            self.assertEqual(dev.verify_bundle(bundle)['profile'], profile)
            self.assertEqual(bundle, dev.bundle(self.state, 'feature'))
            for repo in dev.REPOSITORIES:
                self.assertEqual(
                    (workspace / repo / 'Cargo.lock').read_bytes(), locks[repo]
                )
                self.assertEqual(dev.git(workspace / repo, 'status', '--porcelain'), '')
            # Hash inventory catches resource changes, not only executable changes.
            member = bundle / 'share/xdg-desktop-portal/knave-portals.conf'
            member.chmod(0o644)
            original = member.read_bytes()
            member.write_text('tampered')
            with self.assertRaisesRegex(ValueError, 'hash mismatch'):
                dev.verify_bundle(bundle)
            member.write_bytes(original)
            member.chmod(0o444)
        # Make test fixture cleanup possible despite intentionally immutable bundles.
        for item in self.state.rglob('*'):
            if item.is_dir():
                item.chmod(0o700)


class BundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix='kd-bundle-')
        self.addCleanup(self.temp.cleanup)
        self.state = Path(self.temp.name)
        self.bundle = self.make_bundle('one')

    def make_bundle(self, name):
        path = self.state / 'bundles' / name
        path.mkdir(parents=True)
        manifest = {
            'schema': 1,
            'id': name,
            'complete': True,
            'binaries': {},
            'files': {},
        }
        for binary, (_, relative) in dev.BINARIES.items():
            item = path / relative
            item.parent.mkdir(exist_ok=True)
            item.write_text('#!/bin/sh\nexit 0\n')
            item.chmod(0o555)
            manifest['binaries'][binary] = {
                'path': relative,
                'sha256': dev.digest(item),
            }
            manifest['files'][relative] = dev.digest(item)
        resources = {
            'share/xdg-desktop-portal/portals/knave.portal': '[portal]\n',
            'share/xdg-desktop-portal/knave-portals.conf': '[preferred]\ndefault=knave\n',
            'share/xdg-desktop-portal/villain-portals.conf': '[preferred]\ndefault=gtk\n',
            'tools/desktop-harness.py': (SCRIPTS / 'desktop-harness.py').read_text(),
        }
        for relative, text in resources.items():
            item = path / relative
            item.parent.mkdir(parents=True, exist_ok=True)
            item.write_text(text)
            manifest['files'][relative] = dev.digest(item)
        dev.atomic_json(path / 'bundle.json', manifest)
        dev.atomic_json(
            self.state / 'verification' / f'{name}.json',
            {'passed': True, 'bundle_sha256': dev.digest(path / 'bundle.json')},
        )
        return path

    def test_export_import_and_evidence_identity(self):
        archive = self.state / 'handoff.tar.gz'
        dev.export_bundle(self.state, self.bundle, archive)
        other = self.state / 'other-account'
        imported = dev.import_bundle(other, archive)
        self.assertEqual(dev.tested_bundle(other, imported)['id'], 'one')
        self.assertEqual(imported, dev.import_bundle(other, archive))
        with self.assertRaises(FileExistsError):
            dev.export_bundle(self.state, self.bundle, archive)
        (other / 'verification/one.json').write_text('{"passed":false}')
        with self.assertRaisesRegex(ValueError, 'matching successful'):
            dev.tested_bundle(other, imported)

    def test_extra_file_and_symlink_are_rejected(self):
        extra = self.bundle / 'extra'
        extra.write_text('untracked bundle member')
        with self.assertRaisesRegex(ValueError, 'inventory changed'):
            dev.verify_bundle(self.bundle)
        extra.unlink()
        item = self.bundle / 'bin/knave'
        external = self.state / 'external'
        shutil.copyfile(item, external)
        item.unlink()
        item.symlink_to(external)
        with self.assertRaisesRegex(ValueError, 'invalid bundle member'):
            dev.verify_bundle(self.bundle)

    def test_import_rejects_archive_links(self):
        archive = self.state / 'bad.tar'
        with tarfile.open(archive, 'w') as stream:
            member = tarfile.TarInfo('bundle/escape')
            member.type = tarfile.SYMTYPE
            member.linkname = '/etc/passwd'
            stream.addfile(member)
        with self.assertRaisesRegex(ValueError, 'unsupported archive member'):
            dev.import_bundle(self.state, archive)

    def test_selection_rollback_and_running_session_lock(self):
        other = self.make_bundle('two')
        with patch.object(tty, 'account'), patch.object(tty, 'no_graphical_sessions'):
            tty.select(self.state, self.bundle)
            tty.select(self.state, other)
            self.assertEqual((self.state / 'current').resolve(), other)
            self.assertEqual(tty.select(self.state, None, rollback=True), self.bundle)
            with dev.lock(self.state / 'tty.lock'):
                with self.assertRaisesRegex(RuntimeError, 'already active'):
                    tty.select(self.state, other)
            self.assertEqual((self.state / 'current').resolve(), self.bundle)

    def test_local_tty_rejects_ssh_inactive_and_missing_seat(self):
        good = {
            'User': str(os.getuid()),
            'Active': 'yes',
            'Remote': 'no',
            'Type': 'tty',
            'Seat': 'seat0',
        }
        with (
            patch.dict(os.environ, XDG_SESSION_ID='fixture'),
            patch('sys.stdin.isatty', return_value=True),
        ):
            for change in (
                {'Remote': 'yes'},
                {'Active': 'no'},
                {'Type': 'wayland'},
                {'Seat': ''},
                {'User': '-1'},
            ):
                with patch.object(tty, 'properties', return_value=good | change):
                    with self.assertRaisesRegex(ValueError, 'active local logind'):
                        tty.local_tty()

    def test_activation_exec_escaping_and_pinned_paths(self):
        run = self.state / 'session with space % and $'
        run.mkdir()
        for name in ('knave.portal', 'knave-portals.conf', 'villain-portals.conf'):
            path = (
                self.bundle
                / 'share/xdg-desktop-portal'
                / ('portals' if name == 'knave.portal' else '')
                / name
            )
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text('fixture')
        files = tty.activation_files(self.bundle, run)
        self.assertIn('%%', files['unit'])
        self.assertIn('$$', files['unit'])
        self.assertIn('SystemdService=', files['dbus'])
        self.assertIn(
            str(self.bundle / 'libexec/xdg-desktop-portal-knave'),
            (run / 'portal-launch').read_text(),
        )
        subprocess.run(['sh', '-n', str(run / 'portal-launch')], check=True)

    def test_tty_start_and_failed_activation_clean_up_owned_metadata(self):
        from types import SimpleNamespace

        paths = {
            name: self.state / 'activation' / name
            for name in ('descriptor', 'routing', 'villain-routing', 'dbus', 'unit')
        }
        (self.state / 'current').symlink_to(self.bundle)
        calls = []

        def systemctl(*args, **kwargs):
            calls.append(args)
            return SimpleNamespace(
                returncode=3 if args[0] == 'is-active' else 0,
                stdout='PATH=/previous/path\n',
                stderr='',
            )

        with (
            patch.object(tty, 'account'),
            patch.object(tty, 'local_tty', return_value=self.state),
            patch.object(tty, 'metadata_paths', return_value=paths),
            patch.object(tty, 'systemctl', side_effect=systemctl),
            patch.object(tty, 'capture'),
        ):
            self.assertEqual(tty.start(self.state), 0)
            self.assertFalse((self.state / 'tty-active.json').exists())
            self.assertTrue(all(not path.exists() for path in paths.values()))
            self.assertIn(('set-environment', 'PATH=/previous/path'), calls)
            with patch.object(
                tty,
                'reload_activation',
                side_effect=[RuntimeError('activation failed'), None],
            ):
                with self.assertRaisesRegex(RuntimeError, 'activation failed'):
                    tty.start(self.state)
            self.assertFalse((self.state / 'tty-active.json').exists())
            self.assertTrue(all(not path.exists() for path in paths.values()))

    def test_pending_recovery_blocks_selection(self):
        (self.state / 'tty-active.json').write_text('{}')
        with patch.object(tty, 'account'), patch.object(tty, 'no_graphical_sessions'):
            with self.assertRaisesRegex(ValueError, 'tty recover'):
                tty.select(self.state, self.bundle)

    def test_cleanup_keeps_recovery_journal_if_metadata_reload_fails(self):
        from types import SimpleNamespace

        marker = self.state / 'tty-active.json'
        marker.write_text('recovery required')
        journal = {'files': {}, 'systemd_environment': {}}
        with (
            patch.object(
                tty,
                'systemctl',
                return_value=SimpleNamespace(returncode=3, stderr='', stdout=''),
            ),
            patch.object(tty, 'capture'),
            patch.object(
                tty, 'reload_activation', side_effect=OSError('bus unavailable')
            ),
        ):
            with self.assertRaisesRegex(RuntimeError, 'cleanup incomplete'):
                tty.restore(self.state, journal)
        self.assertTrue(marker.exists())

    def test_cleanup_preserves_external_activation_file_edits(self):
        from types import SimpleNamespace

        path = self.state / 'activation-file'
        path.write_text('owned')
        expected = dev.digest(path)
        path.write_text('external edit')
        marker = self.state / 'tty-active.json'
        marker.write_text('recovery required')
        with (
            patch.object(
                tty,
                'systemctl',
                return_value=SimpleNamespace(returncode=3, stderr='', stdout=''),
            ),
            patch.object(tty, 'capture'),
            patch.object(tty, 'reload_activation'),
        ):
            with self.assertRaisesRegex(RuntimeError, 'externally changed'):
                tty.restore(
                    self.state,
                    {'files': {str(path): expected}, 'systemd_environment': {}},
                )
        self.assertEqual(path.read_text(), 'external edit')
        self.assertTrue(marker.exists())


if __name__ == '__main__':
    unittest.main()
