"""Assemble a signed Superfile bundle; --debug builds the local Cargo profile."""
import argparse
import json
from pathlib import Path
import plistlib
import shutil
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[3]
DEBUG_ID = 'app.superfile.desktop.debug'


def assemble(binary, bundle, version, debug=False):
    contents = bundle / 'Contents'
    (contents / 'MacOS').mkdir(parents=True)
    (contents / 'Resources').mkdir()
    shutil.copy2(binary, contents / 'MacOS/superfile')
    for asset in ('assets/macos/Superfile.icns', 'LICENSE', 'README.md'):
        shutil.copy2(ROOT / asset, contents / 'Resources')
    info = plistlib.loads((ROOT / 'assets/macos/Info.plist').read_bytes())
    info.update(CFBundleShortVersionString=version, CFBundleVersion=version)
    if debug:
        info.update(CFBundleIdentifier=DEBUG_ID, CFBundleName='Superfile Debug',
                    CFBundleDisplayName='Superfile Debug')
        info.pop('CFBundleURLTypes', None)
    (contents / 'Info.plist').write_bytes(plistlib.dumps(info, sort_keys=False))
    subprocess.run(['codesign', '--force', '--sign', '-', bundle], check=True)
    subprocess.run(['codesign', '--verify', '--deep', '--strict', bundle], check=True)


def debug_bundle():
    subprocess.run(['cargo', 'build', '--locked', '-p', 'superfile'], cwd=ROOT, check=True)
    metadata = json.loads(subprocess.check_output(
        ['cargo', 'metadata', '--locked', '--no-deps', '--format-version', '1'], cwd=ROOT))
    version = next(p['version'] for p in metadata['packages'] if p['name'] == 'superfile')
    directory = Path(metadata['target_directory']) / 'debug'
    bundle = directory / 'Superfile Debug.app'
    with tempfile.TemporaryDirectory(dir=directory) as temporary:
        staged = Path(temporary) / bundle.name
        assemble(directory / 'superfile', staged, version, debug=True)
        backup = Path(temporary) / 'previous'
        if bundle.exists():
            info = plistlib.loads((bundle / 'Contents/Info.plist').read_bytes())
            if info.get('CFBundleIdentifier') != DEBUG_ID or bundle.is_symlink():
                raise ValueError(f'Unexpected bundle at {bundle}')
            bundle.rename(backup)
        try:
            staged.rename(bundle)
        except OSError:
            if backup.exists():
                backup.rename(bundle)
            raise
    print(bundle)


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--debug', action='store_true')
    parser.add_argument('release', nargs='*', metavar='ARG')
    args = parser.parse_args()
    if args.debug and not args.release:
        debug_bundle()
    elif not args.debug and len(args.release) == 3:
        binary, bundle, version = args.release
        assemble(Path(binary), Path(bundle), version)
    else:
        parser.error('use --debug or supply BINARY BUNDLE VERSION')
