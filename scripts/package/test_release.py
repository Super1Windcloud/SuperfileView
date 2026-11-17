"""Regression checks for tag preparation and publication completeness."""
import hashlib
import os
from pathlib import Path
import subprocess
import tempfile
import tomllib
import unittest

from release import ROOT, assets, prepare, verify, version


class ReleaseTests(unittest.TestCase):
    def test_shell_packagers_reject_missing_release_instead_of_falling_back(self):
        with tempfile.TemporaryDirectory() as temporary:
            environment = {**os.environ, 'SUPERFILE_PACKAGE_BINARY': str(Path(temporary) / 'missing')}
            for script, arguments in [
                ('macos/package-dmg.sh', ['aarch64-apple-darwin', 'aarch64', 'v1.2.3']),
                ('linux/package-deb.sh', ['x86_64-unknown-linux-gnu', 'v1.2.3']),
            ]:
                with self.subTest(script=script):
                    result = subprocess.run(['bash', str(ROOT / 'scripts/package' / script), *arguments],
                                            env=environment, capture_output=True, text=True)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn('Missing release binary:', result.stderr)

    def test_versions_and_prerelease_order(self):
        self.assertEqual(version('v1.2.3')['version'], '1.2.3')
        self.assertEqual(version('1.2.3-rc.1')['deb'], '1.2.3~rc.1')
        self.assertEqual(version('v1.2.3-rc.1')['prerelease'], 'true')
        for tag in ['v1', 'nightly', 'v01.2.3', 'v1.2.3-01', 'v1.2.3/evil',
                    'v1.2.3\nfoo=bar', 'v1.2.3+metadata', 'v65536.0.0']:
            with self.subTest(tag=tag), self.assertRaises(ValueError):
                version(tag)

    def test_ci_preparation_changes_only_workspace_versions(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'Cargo.toml').write_text('[workspace]\nmembers = ["app", "core"]\n\n[workspace.package]\n# keep comment\nversion = "0.0.2"\nedition = "2024"\n\n[workspace.dependencies]\nserde = "1"\n')
            for name in ['app', 'core']:
                (root / name).mkdir()
                (root / name / 'Cargo.toml').write_text(f'[package]\nname = "{name}"\nversion.workspace = true\n')
            external = '[[package]]\nname = "serde"\nversion = "1.0.0"\nsource = "registry+example"\nchecksum = "keep"\n'
            (root / 'Cargo.lock').write_text('version = 4\n\n' + ''.join(
                f'[[package]]\nname = "{name}"\nversion = "0.0.2"\n\n' for name in ['app', 'core']) + external)
            prepare('v2.3.4-rc.1', root)
            manifest = (root / 'Cargo.toml').read_text()
            locked = (root / 'Cargo.lock').read_text()
            self.assertIn('# keep comment', manifest)
            self.assertIn(external, locked)
            self.assertEqual(tomllib.loads(manifest)['workspace']['package']['version'], '2.3.4-rc.1')
            self.assertEqual([package['version'] for package in tomllib.loads(locked)['package'][:2]], ['2.3.4-rc.1'] * 2)
            prepare('v2.3.4-rc.1', root)  # retry is idempotent
            self.assertEqual((root / 'Cargo.lock').read_text(), locked)
            with self.assertRaises(ValueError):
                prepare('v2.3.4;bad', root)
            self.assertEqual((root / 'Cargo.toml').read_text(), manifest)

    def test_publication_rejects_missing_empty_and_unexpected_assets(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            names = assets('v1.2.3')
            self.assertEqual(len(names), 8)
            for name in names:
                (root / name).write_bytes(name.encode())
            verify('v1.2.3', root, checksums=True)
            checksums = (root / 'SHA256SUMS.txt').read_text()
            self.assertEqual(len(checksums.splitlines()), 8)
            for name in names:
                self.assertIn(hashlib.sha256(name.encode()).hexdigest() + '  ' + name, checksums)
            verify('v1.2.3', root, checksums=True)  # stale checksum file is excluded
            (root / 'SHA256SUMS.txt').unlink()
            missing = root / next(iter(names))
            missing.unlink()
            with self.assertRaises(ValueError):
                verify('v1.2.3', root)
            missing.touch()
            with self.assertRaises(ValueError):
                verify('v1.2.3', root)
            missing.write_bytes(b'package')
            (root / 'old-version.zip').write_bytes(b'stale')
            with self.assertRaises(ValueError):
                verify('v1.2.3', root)


if __name__ == '__main__':
    unittest.main()
