"""No-network integrity/publication regressions for the build-time downloader."""
import hashlib
import importlib.util
import io
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("onnx_build", Path(__file__).with_name("build.py"))
build = importlib.util.module_from_spec(spec)
spec.loader.exec_module(build)


class Response(io.BytesIO):
    url = "https://example.test/pinned-header"


class DownloadTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.destination = self.root / "header.h"
        self.raw = b"// Exact upstream bytes\n#endif\n"
        self.digest = hashlib.sha256(self.raw).hexdigest()

    def download(self, data, *, size=None, url=None):
        response = Response(data)
        if url is not None:
            response.url = url
        with patch.object(build.urllib.request, "urlopen", return_value=response) as request:
            build.verified_download("https://example.test/pinned-header", self.destination,
                                    self.digest, size)
        return request

    def test_exact_bytes_are_published_and_valid_cache_needs_no_network(self):
        self.download(self.raw, size=len(self.raw))
        self.assertEqual(self.destination.read_bytes(), self.raw)
        with patch.object(build.urllib.request, "urlopen") as request:
            build.verified_download("https://example.test/pinned-header", self.destination,
                                    self.digest, len(self.raw))
        request.assert_not_called()
        self.assertEqual(list(self.root.iterdir()), [self.destination])

    def test_extra_newline_is_rejected_without_publication_or_temporary_files(self):
        with self.assertRaisesRegex(RuntimeError, "integrity"):
            self.download(self.raw + b"\n")
        self.assertEqual(list(self.root.iterdir()), [])

    def test_corrupted_cache_is_replaced_only_after_a_valid_download(self):
        self.destination.write_bytes(b"corrupted cache")
        with self.assertRaisesRegex(RuntimeError, "integrity"):
            self.download(self.raw + b"\n")
        self.assertEqual(self.destination.read_bytes(), b"corrupted cache")
        self.assertEqual(list(self.root.iterdir()), [self.destination])
        self.download(self.raw)
        self.assertEqual(self.destination.read_bytes(), self.raw)

    def test_truncated_download_is_rejected_and_cleaned(self):
        with self.assertRaisesRegex(RuntimeError, "integrity"):
            self.download(self.raw[:-1], size=len(self.raw))
        self.assertEqual(list(self.root.iterdir()), [])

    def test_oversized_download_is_rejected_and_cleaned(self):
        with self.assertRaisesRegex(RuntimeError, "size"):
            self.download(self.raw + b"x", size=len(self.raw))
        self.assertEqual(list(self.root.iterdir()), [])

    def test_invalid_target_argument_cannot_silently_disable_the_worker(self):
        for target in ["build-test.py", "../runtime", "", "x86_64"]:
            with self.assertRaisesRegex(ValueError, "target triple"):
                build.prepare(target)

    def test_non_https_redirect_cannot_publish(self):
        with self.assertRaisesRegex(RuntimeError, "HTTPS"):
            self.download(self.raw, url="http://example.test/header")
        self.assertEqual(list(self.root.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
