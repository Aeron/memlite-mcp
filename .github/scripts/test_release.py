import unittest
from contextlib import redirect_stderr, redirect_stdout
from email.message import Message
from io import StringIO
from unittest.mock import patch
from urllib.error import HTTPError, URLError

import release


class ReleaseTests(unittest.TestCase):
    def run_release(self, *args):
        output = StringIO()
        manifest = '[package]\nname = "memlite-mcp"\nversion = "0.1.0"\n'
        with (
            patch("sys.argv", ["release.py", *args]),
            patch.object(release.Path, "read_text", return_value=manifest),
            redirect_stdout(output),
            redirect_stderr(StringIO()),
        ):
            release.main()
        return output.getvalue()

    def test_matching_tag(self):
        self.assertEqual(self.run_release("0.1.0"), "version=0.1.0\n")

    def test_mismatched_tag_fails_before_registry_lookup(self):
        with patch.object(release, "is_published") as lookup:
            for tag in ("0.2.0", "v0.1.0", "main"):
                with self.subTest(tag=tag), self.assertRaises(SystemExit) as error:
                    self.run_release(tag, "--check-published")
                self.assertEqual(error.exception.code, 2)
            lookup.assert_not_called()

    def test_published_and_unpublished_outputs(self):
        for published in (True, False):
            with (
                self.subTest(published=published),
                patch.object(release, "is_published", return_value=published) as lookup,
            ):
                self.assertEqual(
                    self.run_release("0.1.0", "--check-published"),
                    f"version=0.1.0\npublished={str(published).lower()}\n",
                )
                lookup.assert_called_once_with("memlite-mcp", "0.1.0")

    def test_existing_version(self):
        with patch.object(release, "urlopen") as request:
            self.assertTrue(release.is_published("memlite-mcp", "0.1.0"))
            args, kwargs = request.call_args
            self.assertEqual(
                args[0].full_url,
                "https://crates.io/api/v1/crates/memlite-mcp/0.1.0",
            )
            self.assertEqual(kwargs["timeout"], 30)

    def test_missing_version(self):
        error = HTTPError("url", 404, "Not Found", Message(), None)
        with patch.object(release, "urlopen", side_effect=error):
            self.assertFalse(release.is_published("memlite-mcp", "0.1.0"))

    def test_registry_errors_are_not_missing_versions(self):
        errors: list[URLError] = [
            HTTPError("url", code, "Registry error", Message(), None)
            for code in (401, 403, 429, 500, 503)
        ]
        errors.append(URLError("Connection failed"))
        for error in errors:
            with (
                self.subTest(error=error),
                patch.object(release, "urlopen", side_effect=error),
                self.assertRaises(type(error)),
            ):
                release.is_published("memlite-mcp", "0.1.0")


if __name__ == "__main__":
    unittest.main()
