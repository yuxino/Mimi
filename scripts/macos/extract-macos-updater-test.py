import importlib.util
import io
from pathlib import Path
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location('extractor', Path(__file__).with_name('extract-macos-updater.py'))
extractor = importlib.util.module_from_spec(spec)
spec.loader.exec_module(extractor)


class ArchiveSafety(unittest.TestCase):
    def attempt(self, entries, rejects=False):
        with tempfile.TemporaryDirectory() as root:
            archive = Path(root) / 'app.tar.gz'
            output = Path(root) / 'out'
            with tarfile.open(archive, 'w:gz') as tar:
                for name, link in entries:
                    entry = tarfile.TarInfo(name)
                    if link:
                        entry.type = tarfile.SYMTYPE
                        entry.linkname = link
                        tar.addfile(entry)
                    else:
                        entry.size = 1
                        tar.addfile(entry, io.BytesIO(b'x'))
            if rejects:
                with self.assertRaises(ValueError):
                    extractor.extract(archive, output)
            else:
                extractor.extract(archive, output)
                self.assertEqual((output / 'mimi.app/Contents/MacOS/mimi').read_bytes(), b'x')

    def test_valid_app(self):
        self.attempt([('mimi.app/Contents/MacOS/mimi', None)])

    def test_parent_traversal(self):
        self.attempt([('mimi.app/../../outside', None)], True)

    def test_absolute_path(self):
        self.attempt([('/tmp/outside', None)], True)

    def test_other_app(self):
        self.attempt([('other.app/Contents/MacOS/other', None)], True)

    def test_escaping_symlink(self):
        self.attempt([('mimi.app/Contents/link', '../../../outside')], True)

    def test_existing_destination(self):
        with tempfile.TemporaryDirectory() as root:
            with self.assertRaises(ValueError):
                extractor.extract('unused', root)


if __name__ == '__main__':
    unittest.main()
