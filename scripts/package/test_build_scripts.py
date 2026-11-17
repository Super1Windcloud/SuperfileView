"""Exercise build generators with Windows-style default text encoding."""
from pathlib import Path
import runpy
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
READ_TEXT = Path.read_text
WRITE_TEXT = Path.write_text


def ansi_read(path, *args, **kwargs):
    if not args and kwargs.get('encoding') is None:
        kwargs['encoding'] = 'cp1252'
    return READ_TEXT(path, *args, **kwargs)


def ansi_write(path, data, *args, **kwargs):
    if not args and kwargs.get('encoding') is None:
        kwargs['encoding'] = 'cp1252'
    return WRITE_TEXT(path, data, *args, **kwargs)


class BuildScriptEncodingTests(unittest.TestCase):
    def test_reader_generation_and_docs_ignore_windows_default_encoding(self):
        namespace = runpy.run_path(str(ROOT / 'scripts/generate-reader-limits.py'))
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / 'assets').mkdir()
            (root / 'docs').mkdir()
            (root / 'assets/reader-limits.json').write_bytes((ROOT / 'assets/reader-limits.json').read_bytes())
            with patch.dict(namespace['generate'].__globals__, ROOT=root), \
                 patch.object(Path, 'read_text', ansi_read), \
                 patch.object(Path, 'write_text', ansi_write):
                namespace['generate'](root / 'out')
                constants = root / 'out/reader_limits.rs'
                before = constants.stat().st_mtime_ns
                namespace['generate'](root / 'out')
                self.assertEqual(constants.stat().st_mtime_ns, before)
                namespace['document']()
            self.assertIn('pub const JSON:', constants.read_text(encoding='utf-8'))
            self.assertIn('阅读器', (root / 'docs/reader-limits.zh-CN.md').read_text(encoding='utf-8'))
            self.assertNotIn(b'\r\n', constants.read_bytes())

    def test_locale_catalog_and_architecture_checks_ignore_windows_default_encoding(self):
        for name, arguments in [
            ('check-locales.py', []),
            ('generate-format-catalog.py', ['--check']),
            ('check-architecture.py', []),
        ]:
            with self.subTest(script=name), \
                 patch.object(Path, 'read_text', ansi_read), \
                 patch.object(Path, 'write_text', ansi_write), \
                 patch.object(sys, 'argv', [name, *arguments]):
                runpy.run_path(str(ROOT / 'scripts' / name), run_name='__main__')


if __name__ == '__main__':
    unittest.main()
