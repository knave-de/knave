#!/usr/bin/env python3
"""Create isolated Knave feature workspaces and test exact desktop bundles."""

import argparse
import json
import os
import subprocess
import sys
from pathlib import Path

import knave_dev as dev


def parser():
    result = argparse.ArgumentParser(description=__doc__)
    result.add_argument(
        '--state',
        type=Path,
        default=Path(os.environ.get('XDG_DATA_HOME', str(Path.home() / '.local/share')))
        / 'knave-dev',
    )
    commands = result.add_subparsers(dest='command', required=True)
    create = commands.add_parser(
        'create', help='create matching branches and detached dependency worktrees'
    )
    create.add_argument('name')
    create.add_argument(
        '--sources', type=Path, default=Path(__file__).resolve().parents[2]
    )
    create.add_argument(
        '--change', action='append', choices=dev.REPOSITORIES, required=True
    )
    create.add_argument('--rev', action='append', default=[], metavar='REPO=REV')
    build = commands.add_parser(
        'build',
        help='snapshot committed sources; format, test, lint and build every component',
    )
    build.add_argument('name')
    build.add_argument('--profile', choices=('debug', 'release'), default='debug')
    build.add_argument('--jobs', type=int, default=1)
    for name in ('bundle', 'remove', 'status'):
        commands.add_parser(name).add_argument('name')
    test = commands.add_parser(
        'test', help='exercise and record a frozen bundle in nested Winit'
    )
    test.add_argument('bundle', type=Path)
    test.add_argument('--display', type=int, default=93)
    test.add_argument('--vnc-port', type=int, default=0)
    commands.add_parser('verify').add_argument('bundle', type=Path)
    export = commands.add_parser(
        'export',
        help='package a tested bundle and its verification for another account',
    )
    export.add_argument('bundle', type=Path)
    export.add_argument('--output', type=Path, required=True)
    commands.add_parser(
        'import', help='verify and import an exported test bundle'
    ).add_argument('archive', type=Path)
    pr = commands.add_parser(
        'pr',
        help='explicitly push committed changes and open linked draft PRs; never merges',
    )
    pr.add_argument('name')
    pr.add_argument('--bundle', type=Path, required=True)
    pr.add_argument('--base', default='master')
    tty = commands.add_parser(
        'tty',
        help='dedicated test-account operations; never installs production binaries',
    )
    operations = tty.add_subparsers(dest='operation', required=True)
    operations.add_parser('init').add_argument(
        '--dedicated-test-account', action='store_true', required=True
    )
    operations.add_parser('use').add_argument('bundle', type=Path)
    operations.add_parser('start').add_argument('--config', type=Path)
    for name in ('rollback', 'status', 'recover'):
        operations.add_parser(name)
    return result


def main():
    args = parser().parse_args()
    os.umask(0o077)
    state = args.state.expanduser().resolve()
    if args.command == 'create':
        revisions = {}
        for value in args.rev:
            repo, separator, revision = value.partition('=')
            if (
                not separator
                or repo not in dev.REPOSITORIES
                or not revision
                or repo in revisions
            ):
                raise ValueError('--rev requires one REPO=REV per known repository')
            revisions[repo] = revision
        print(
            dev.create(state, args.name, args.sources.resolve(), args.change, revisions)
        )
    elif args.command == 'build':
        print(dev.build(state, args.name, args.profile, args.jobs))
    elif args.command == 'bundle':
        print(dev.bundle(state, args.name))
    elif args.command == 'remove':
        dev.remove(state, args.name)
    elif args.command == 'status':
        root, data = dev.workspace(state, args.name)
        for repo, entry in data['repositories'].items():
            entry['head'] = dev.git(root / repo, 'rev-parse', 'HEAD')
            entry['status'] = dev.git(root / repo, 'status', '--short')
        print(json.dumps(data, indent=2))
    elif args.command == 'test':
        print(
            json.dumps(
                dev.test_bundle(
                    state, args.bundle.resolve(), args.display, args.vnc_port
                ),
                indent=2,
            )
        )
    elif args.command == 'verify':
        print(json.dumps(dev.verify_bundle(args.bundle.resolve()), indent=2))
    elif args.command == 'export':
        print(dev.export_bundle(state, args.bundle.resolve(), args.output.resolve()))
    elif args.command == 'import':
        print(dev.import_bundle(state, args.archive.resolve()))
    elif args.command == 'pr':
        print(
            json.dumps(
                dev.pull_requests(state, args.name, args.bundle.resolve(), args.base),
                indent=2,
            )
        )
    elif args.command == 'tty':
        import knave_dev_tty as tty

        if args.operation == 'init':
            tty.init(state)
        elif args.operation == 'use':
            print(tty.select(state, args.bundle))
        elif args.operation == 'rollback':
            print(tty.select(state, None, rollback=True))
        elif args.operation == 'recover':
            tty.recover(state)
        elif args.operation == 'start':
            return tty.start(state, args.config)
        else:
            tty.account(state)
            print(
                json.dumps(
                    {
                        name: str((state / name).resolve())
                        if (state / name).is_symlink()
                        else None
                        for name in ('current', 'previous')
                    },
                    indent=2,
                )
            )
    return 0


if __name__ == '__main__':
    try:
        sys.exit(main())
    except (
        OSError,
        ValueError,
        KeyError,
        RuntimeError,
        subprocess.SubprocessError,
    ) as error:
        print(f'knave-dev: {error}', file=sys.stderr)
        if isinstance(error, subprocess.CalledProcessError) and error.stderr:
            print(error.stderr, file=sys.stderr)
        sys.exit(1)
