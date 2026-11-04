import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("create-alpha-release.py"))
release = importlib.util.module_from_spec(spec)
spec.loader.exec_module(release)


class ReleaseTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.exe = Path(self.temp.name) / "shotkit.exe"
        self.exe.write_bytes(b"MZ" + b"x" * 100_000)
        self.manifest = Path(self.temp.name) / "Cargo.toml"
        self.manifest.write_text('[package]\nversion = "2.0.0-alpha.14"\n', encoding="utf-8")
        self.calls = []
        self.existing = False
        self.bad_digest = False

    def api(self, url, token, method="GET", data=None, content_type=None):
        self.calls.append((url, method, data))
        if "/commits/" in url:
            return {"sha": "commit"}
        if "/releases/tags/" in url:
            if self.existing:
                return {"id": 4}
            raise HTTPError(url, 404, "Not Found", {}, None)
        if url.endswith("/releases"):
            return {"id": 4, "draft": True, "prerelease": True, "upload_url": "https://uploads.example/assets{?name}"}
        if url.startswith("https://uploads.example/"):
            from hashlib import sha256
            from urllib.parse import parse_qs, urlsplit
            name = parse_qs(urlsplit(url).query)["name"][0]
            digest = "sha256:" + sha256(data).hexdigest()
            return {"name": name, "size": len(data), "digest": "wrong" if self.bad_digest else digest}
        if url.endswith("/releases/4"):
            return {"draft": False, "prerelease": True, "html_url": "https://example.com/release"}
        self.fail(f"unexpected request: {method} {url}")

    def publish(self, tag="v2.0.0-alpha.14"):
        with patch.object(release, "api", side_effect=self.api):
            return release.publish(self.exe, tag, "commit", "owner/repo", "token", "https://example.com/run", self.manifest)

    def test_publish_only_after_two_verified_uploads(self):
        self.assertEqual(self.publish(), "https://example.com/release")
        self.assertEqual([c[1] for c in self.calls], ["GET", "GET", "POST", "POST", "POST", "PATCH"])
        create = json.loads(self.calls[2][2])
        self.assertTrue(create["draft"] and create["prerelease"])
        self.assertIn(b"shotkit-v2.0.0-alpha.14-windows-x64.exe", self.calls[4][2])

    def test_existing_release_is_never_modified(self):
        self.existing = True
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.publish()
        self.assertTrue(all(method == "GET" for _, method, _ in self.calls))

    def test_bad_upload_keeps_draft_unpublished(self):
        self.bad_digest = True
        with self.assertRaisesRegex(ValueError, "draft retained"):
            self.publish()
        self.assertNotIn("PATCH", [c[1] for c in self.calls])

    def test_mismatched_version_is_rejected_before_api_calls(self):
        self.manifest.write_text('[package]\nversion = "2.0.0-alpha.13"\n', encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "does not match"):
            self.publish()
        self.assertEqual(self.calls, [])

    def test_beta_prerelease_uses_matching_manifest_and_beta_description(self):
        self.manifest.write_text('[package]\nversion = "2.0.0-beta.1"\n', encoding="utf-8")
        self.assertEqual(self.publish("v2.0.0-beta.1"), "https://example.com/release")
        create = json.loads(self.calls[2][2])
        self.assertTrue(create["draft"] and create["prerelease"])
        self.assertIn("Beta", create["body"])
        self.assertIn(b"shotkit-v2.0.0-beta.1-windows-x64.exe", self.calls[4][2])

    def test_other_tag_is_rejected_before_api_calls(self):
        with self.assertRaisesRegex(ValueError, "alpha or beta tags"):
            self.publish("v2.0.0-rc.1")
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
