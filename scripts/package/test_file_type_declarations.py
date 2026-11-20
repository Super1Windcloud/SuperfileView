"""Regression checks for associations where filename alone is ambiguous."""
import plistlib
import unittest
import xml.etree.ElementTree as ET
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]


class FileTypeDeclarationTests(unittest.TestCase):
    def test_typescript_uses_source_uti_for_ambiguous_extensions(self):
        bundle = plistlib.loads((ROOT / 'assets/macos/Info.plist').read_bytes())
        source = next(t for t in bundle['UTImportedTypeDeclarations']
                      if t['UTTypeIdentifier'] == 'com.microsoft.typescript')
        self.assertIn('public.source-code', source['UTTypeConformsTo'])
        suffixes = {'ts', 'tsx', 'cts', 'mts'}
        self.assertEqual(suffixes, set(source['UTTypeTagSpecification']['public.filename-extension']))
        for ext in suffixes:
            matches = [g for g in bundle['CFBundleDocumentTypes'] if ext in g['CFBundleTypeExtensions']]
            self.assertEqual([g.get('LSItemContentTypes') for g in matches], [['com.microsoft.typescript']])

    def test_linux_associations_cover_specialized_readers_and_avoid_unrelated_types(self):
        desktop = (ROOT / 'assets/linux/superfile.desktop').read_text(encoding='utf-8')
        handlers = set(next(s for s in desktop.splitlines() if s.startswith('MimeType='))[9:].split(';'))
        self.assertTrue({'text/csv', 'text/x-opml+xml', 'application/vnd.jgraph.mxfile',
                         'application/jsonc', 'image/vnd.microsoft.icon', 'image/x-icns',
                         'image/x-exr', 'image/vnd.radiance', 'application/x-zip-compressed-fb2'} <= handlers)
        self.assertFalse({'application/octet-stream', 'video/mp2t', 'application/x-tiled-tsx'} & handlers)
        definitions = ET.parse(ROOT / 'assets/linux/superfile-mime.xml')
        globs = {g.get('pattern') for g in definitions.findall('.//{*}glob')}
        self.assertTrue({'*.hdr', '*.drawio', '*.jsonc'} <= globs)
        self.assertFalse({'*.ts', '*.tsx', '*.mts', '*.db'} & globs)
