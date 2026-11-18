"""Tests for published.py: `python3 -m unittest test_published` in scripts/ (dry-run.sh runs them)."""

import json
import subprocess
import tempfile
import unittest
from pathlib import Path

import published


class FakeGh:
    """`gh release view/download` against a fake `index` release."""

    def __init__(self, assets=None, view_error=None, download_error=None):
        self.assets = assets
        self.view_error = view_error
        self.download_error = download_error
        self.downloads = []

    def __call__(self, argv, capture_output, text):
        if argv[1:3] == ["release", "view"]:
            if self.view_error:
                return subprocess.CompletedProcess(argv, 1, "", self.view_error)
            if self.assets is None:
                return subprocess.CompletedProcess(argv, 1, "", "release not found\n")
            body = json.dumps({"assets": [{"name": name} for name in self.assets]})
            return subprocess.CompletedProcess(argv, 0, body, "")
        name = argv[argv.index("--pattern") + 1]
        out = Path(argv[argv.index("--dir") + 1])
        self.downloads.append(name)
        if self.download_error:
            return subprocess.CompletedProcess(argv, 1, "", self.download_error)
        (out / name).write_text("{}", encoding="utf-8")
        return subprocess.CompletedProcess(argv, 0, "", "")


class Published(unittest.TestCase):
    def setUp(self):
        self.dir = Path(tempfile.mkdtemp())

    def download(self, gh, names, first_run=False):
        return published.download("o/r", names, self.dir, first_run, run=gh)

    def test_downloads_what_is_published(self):
        gh = FakeGh(assets=["index.json", "snapshot.json", "blocklist.json"])
        self.assertEqual(self.download(gh, ["index.json", "snapshot.json"]), [])
        self.assertEqual(gh.downloads, ["index.json", "snapshot.json"])
        self.assertTrue((self.dir / "snapshot.json").is_file())

    def test_nothing_published_needs_first_run(self):
        with self.assertRaisesRegex(published.Failed, "first_run"):
            self.download(FakeGh(), ["snapshot.json"])
        self.assertEqual(self.download(FakeGh(), ["snapshot.json"], first_run=True), ["snapshot.json"])
        gh = FakeGh(assets=["index.json"])
        with self.assertRaisesRegex(published.Failed, "no snapshot.json"):
            self.download(gh, ["index.json", "snapshot.json"])
        self.assertEqual(
            self.download(gh, ["index.json", "snapshot.json"], first_run=True), ["snapshot.json"]
        )
        self.assertTrue((self.dir / "index.json").is_file())
        self.assertFalse((self.dir / "snapshot.json").exists())

    def test_gh_errors_fail_even_on_a_first_run(self):
        """A network or API error is never "nothing published": that would restart sequence 1."""
        for first_run in (False, True):
            with self.assertRaisesRegex(published.Failed, "HTTP 502"):
                self.download(FakeGh(view_error="HTTP 502: Bad Gateway"), ["snapshot.json"], first_run)
            with self.assertRaisesRegex(published.Failed, "timeout"):
                self.download(
                    FakeGh(assets=["snapshot.json"], download_error="timeout"), ["snapshot.json"], first_run
                )


if __name__ == "__main__":
    unittest.main()
