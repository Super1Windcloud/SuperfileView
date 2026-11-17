#!/usr/bin/env python3
"""Version preparation and complete release asset validation (stdlib only)."""
import argparse
import hashlib
import os
from pathlib import Path
import re
import tomllib

ROOT = Path(__file__).resolve().parents[2]
PLATFORMS = {
    'windows-x86_64': ('.zip', '-setup.exe'),
    'linux-x86_64': ('.deb', '.tar.gz'),
    'macos-aarch64': ('.dmg', '.tar.gz'),
    'macos-x86_64': ('.dmg', '.tar.gz'),
}


def version(tag):
    match = re.fullmatch(
        r'v?((0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*))'
        r'(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?', tag)
    if not match:
        raise ValueError('Use a version tag such as v1.2.3 or v1.2.3-rc.1')
    core, major, minor, patch, prerelease = match.groups()
    if any(int(part) > 65535 for part in (major, minor, patch)):
        raise ValueError('Version components must fit Windows installer fields (0..65535)')
    if prerelease and any(part.isdigit() and len(part) > 1 and part.startswith('0')
                          for part in prerelease.split('.')):
        raise ValueError('Numeric prerelease identifiers must not have leading zeros')
    return {
        'version': core + ('-' + prerelease if prerelease else ''),
        'core': core,
        'deb': core + ('~' + prerelease if prerelease else ''),
        'prerelease': str(bool(prerelease)).lower(),
    }


def prepare(tag, root=ROOT):
    """Change only workspace versions in the disposable CI checkout, never dependencies."""
    value = version(tag)['version']
    manifest = root / 'Cargo.toml'
    lock = root / 'Cargo.lock'
    source = manifest.read_text(encoding='utf-8')
    parsed = tomllib.loads(source)
    previous = parsed['workspace']['package']['version']
    source, count = re.subn(
        r'(?m)(^\[workspace\.package\]\s*\n(?:(?!^\[).)*?^version\s*=\s*)"[^"]+"',
        lambda match: match[1] + '"' + value + '"', source, flags=re.DOTALL)
    if count != 1:
        raise ValueError('Cannot locate the workspace version')
    names = set()
    for member in parsed['workspace']['members']:
        package = tomllib.loads((root / member / 'Cargo.toml').read_text(encoding='utf-8'))['package']
        if package['version'] != {'workspace': True}:
            raise ValueError('Release packages must inherit the workspace version')
        names.add(package['name'])
    locked = lock.read_text(encoding='utf-8')
    found = set()
    blocks = re.split(r'(?m)(?=^\[\[package\]\])', locked)
    for index, block in enumerate(blocks):
        name = re.search(r'(?m)^name = "([^"]+)"$', block)
        if not name or name[1] not in names:
            continue
        package = tomllib.loads(block)['package'][0]
        if 'source' in package or package['version'] != previous or name[1] in found:
            raise ValueError('Unexpected workspace package in Cargo.lock: ' + name[1])
        blocks[index] = re.sub(r'(?m)^version = "[^"]+"$', 'version = "' + value + '"', block, count=1)
        found.add(name[1])
    if found != names:
        raise ValueError('Cargo.lock is missing workspace packages')
    # Validate both documents before writing either file. Preserve all unrelated text.
    locked = ''.join(blocks)
    tomllib.loads(source)
    tomllib.loads(locked)
    manifest.write_text(source, encoding='utf-8', newline='\n')
    lock.write_text(locked, encoding='utf-8', newline='\n')


def assets(tag, platform=None):
    version(tag)
    platforms = [platform] if platform else PLATFORMS
    return {f'superfile-{tag}-{name}{suffix}'
            for name in platforms for suffix in PLATFORMS[name]}


def verify(tag, directory, platform=None, checksums=False):
    expected = assets(tag, platform)
    paths = list(directory.iterdir())
    actual = {path.name for path in paths if not (checksums and path.name == 'SHA256SUMS.txt')}
    if actual != expected:
        raise ValueError(f'Incomplete release: missing={sorted(expected - actual)}, unexpected={sorted(actual - expected)}')
    for name in sorted(expected):
        path = directory / name
        if path.is_symlink() or not path.is_file() or path.stat().st_size == 0:
            raise ValueError('Empty or invalid package: ' + name)
    if checksums:
        lines = []
        for name in sorted(expected):
            with (directory / name).open('rb') as file:
                digest = hashlib.file_digest(file, 'sha256').hexdigest()
            lines.append(f'{digest}  {name}\n')
        (directory / 'SHA256SUMS.txt').write_text(''.join(lines), encoding='utf-8')
    print(f'Validated {len(expected)} packages in {directory}')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('command', choices=['version', 'prepare', 'verify'])
    parser.add_argument('tag')
    parser.add_argument('--field', choices=['version', 'core', 'deb', 'prerelease'])
    parser.add_argument('--directory', type=Path, default=ROOT / 'package-dist')
    parser.add_argument('--platform', choices=list(PLATFORMS))
    parser.add_argument('--checksums', action='store_true')
    args = parser.parse_args()
    values = version(args.tag)
    if args.command == 'version':
        if args.field:
            print(values[args.field])
        else:
            print('\n'.join(f'{key}={value}' for key, value in values.items()))
            if output := os.environ.get('GITHUB_OUTPUT'):
                with open(output, 'a', encoding='utf-8') as file:
                    file.write('\n'.join(f'{key}={value}' for key, value in values.items()) + '\n')
    elif args.command == 'prepare':
        prepare(args.tag)
        print('Prepared workspace version ' + values['version'])
    else:
        verify(args.tag, args.directory, args.platform, args.checksums)


if __name__ == '__main__':
    main()
