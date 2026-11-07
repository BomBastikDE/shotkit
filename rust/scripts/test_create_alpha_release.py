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
        self.bad_upload_number = None
        self.upload_number = 0
        self.prerelease = True
        self.wrong_draft_status = False
        self.wrong_published_status = False
        self.upload_url = "https://uploads.github.com/repos/owner/repo/releases/4/assets{?name,label}"

    def api(self, url, token, method="GET", data=None, content_type=None):
        self.calls.append((url, method, data))
        if "/commits/" in url:
            return {"sha": "commit"}
        if "/releases/tags/" in url:
            if self.existing:
                return {"id": 4}
            raise HTTPError(url, 404, "Not Found", {}, None)
        if url.endswith("/releases"):
            return {"id": 4, "draft": True, "prerelease": not self.prerelease if self.wrong_draft_status else self.prerelease, "upload_url": self.upload_url}
        if url.startswith("https://uploads.github.com/"):
            from hashlib import sha256
            from urllib.parse import parse_qs, urlsplit
            name = parse_qs(urlsplit(url).query)["name"][0]
            self.upload_number += 1
            digest = "sha256:" + sha256(data).hexdigest()
            return {"name": name, "size": len(data), "digest": "wrong" if self.bad_digest or self.upload_number == self.bad_upload_number else digest}
        if url.endswith("/releases/4"):
            return {"draft": False, "prerelease": not self.prerelease if self.wrong_published_status else self.prerelease, "html_url": "https://example.com/release"}
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

    def test_alpha_and_beta_publication_explicitly_preserve_latest(self):
        for tag in ("v2.0.0-alpha.14", "v2.0.0-beta.1"):
            with self.subTest(tag=tag):
                self.calls.clear()
                self.manifest.write_text(f'[package]\nversion = "{tag[1:]}"\n', encoding="utf-8")
                self.publish(tag)
                self.assertIs(json.loads(self.calls[2][2])["prerelease"], True)
                self.assertNotIn("C# v1.2.0", json.loads(self.calls[2][2])["body"])
                self.assertNotIn("Mixed-DPI", json.loads(self.calls[2][2])["body"])
                self.assertEqual(json.loads(self.calls[2][2])["make_latest"], "false")
                self.assertEqual(json.loads(self.calls[-1][2]), {"draft": False, "make_latest": "false"})

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

    def stable(self):
        self.manifest.write_text('[package]\nversion = "2.0.0"\n', encoding="utf-8")
        self.prerelease = False
        return self.publish("v2.0.0")

    def test_stable_publishes_only_after_two_verified_uploads_and_becomes_latest(self):
        from hashlib import sha256
        self.assertEqual(self.stable(), "https://example.com/release")
        self.assertEqual([c[1] for c in self.calls], ["GET", "GET", "POST", "POST", "POST", "PATCH"])
        create = json.loads(self.calls[2][2])
        self.assertIs(create["draft"], True)
        self.assertIs(create["prerelease"], False)
        self.assertEqual(create["make_latest"], "true")
        self.assertIn("stabile Version", create["body"])
        self.assertIn("nicht signiert", create["body"])
        self.assertIn("SmartScreen", create["body"])
        self.assertIn("SHA256", create["body"])
        self.assertIn("Rust ist die neue maßgebliche Version", create["body"])
        self.assertIn("C# v1.2.0", create["body"])
        self.assertIn("Mixed-DPI- und Mehrmonitor-Tests stehen noch aus", create["body"])
        self.assertIn("Verpixelung ist keine sichere Schwärzung", create["body"])
        self.assertIn("kein vollständiges Security-Audit", create["body"])
        self.assertIn("https://github.com/owner/repo/blob/v2.0.0/rust/CHANGELOG.md", create["body"])
        filename = "shotkit-v2.0.0-windows-x64.exe"
        self.assertIn(filename, self.calls[3][0])
        self.assertIn(filename + ".sha256", self.calls[4][0])
        self.assertEqual(self.calls[3][2], self.exe.read_bytes())
        self.assertEqual(self.calls[4][2], f"{sha256(self.exe.read_bytes()).hexdigest()}  {filename}\n".encode())
        self.assertEqual(json.loads(self.calls[5][2]), {"draft": False, "make_latest": "true"})

    def test_existing_stable_release_is_never_modified(self):
        self.existing = True
        with self.assertRaisesRegex(ValueError, "already exists"):
            self.stable()
        self.assertEqual([c[1] for c in self.calls], ["GET", "GET"])

    def test_stable_second_upload_failure_keeps_draft(self):
        self.bad_upload_number = 2
        with self.assertRaisesRegex(ValueError, "draft retained"):
            self.stable()
        self.assertEqual(self.upload_number, 2)
        self.assertNotIn("PATCH", [c[1] for c in self.calls])

    def test_stable_rejects_wrong_draft_status_before_uploads(self):
        self.wrong_draft_status = True
        with self.assertRaisesRegex(ValueError, "expected draft release"):
            self.stable()
        self.assertEqual([c[1] for c in self.calls], ["GET", "GET", "POST"])

    def test_stable_rejects_wrong_final_prerelease_status(self):
        self.wrong_published_status = True
        with self.assertRaisesRegex(ValueError, "expected prerelease status"):
            self.stable()

    def test_stable_manifest_mismatch_makes_no_api_calls(self):
        with self.assertRaisesRegex(ValueError, "does not match"):
            self.publish("v2.0.0")
        self.assertEqual(self.calls, [])

    def test_unsupported_stable_and_malformed_tags_make_no_api_calls(self):
        for tag in ("v2.0.1", "v2.1.0", "v3.0.0", "v2.0.0-rc.1", "v2.0.0-extra", "v2.0.0\n"):
            with self.subTest(tag=tag):
                with self.assertRaises(ValueError):
                    self.publish(tag)
                self.assertEqual(self.calls, [])

    def test_moved_stable_tag_is_rejected_before_release_creation(self):
        self.manifest.write_text('[package]\nversion = "2.0.0"\n', encoding="utf-8")
        with patch.object(release, "api", return_value={"sha": "other-commit"}) as api:
            with self.assertRaisesRegex(ValueError, "tested commit"):
                release.publish(self.exe, "v2.0.0", "commit", "owner/repo", "token", "https://example.com/run", self.manifest)
        api.assert_called_once()

    def test_rejects_untrusted_upload_target_before_sending_assets(self):
        bad = [
            "http://uploads.github.com/repos/owner/repo/releases/4/assets{?name,label}",
            "https://evil.example/repos/owner/repo/releases/4/assets{?name,label}",
            "https://uploads.github.com.evil.example/repos/owner/repo/releases/4/assets{?name,label}",
            "https://uploads.github.com@evil.example/repos/owner/repo/releases/4/assets{?name,label}",
            "https://uploads.github.com:443/repos/owner/repo/releases/4/assets{?name,label}",
            "https://uploads.github.com/repos/owner/other/releases/4/assets{?name,label}",
            "https://uploads.github.com/repos/owner/repo/releases/5/assets{?name,label}",
            "https://uploads.github.com/repos/owner/repo/releases/4/assets?x=1{?name,label}",
            "https://uploads.github.com/repos/owner/repo/releases/4/assets?{?name,label}",
            "https://uploads.github.com/repos/owner/repo/releases/4/assets#{?name,label}",
            "https://uploads.github.com/repos/owner/repo/releases/4/assets#fragment{?name,label}",
            "https://uploads.github.com/repos/owner/repo/releases/4/assets{?name,label}extra",
        ]
        for target in bad:
            with self.subTest(target=target):
                self.calls.clear()
                self.upload_url = target
                with self.assertRaises(ValueError):
                    self.publish()
                self.assertEqual([method for _, method, _ in self.calls], ["GET", "GET", "POST"])
                self.assertNotIn("PATCH", [method for _, method, _ in self.calls])

    def test_accepts_documented_url_case_insensitively_for_repository(self):
        self.upload_url = "https://uploads.github.com/repos/Owner/Repo/releases/4/assets{?name,label}"
        self.assertEqual(self.publish(), "https://example.com/release")

    def test_authenticated_redirect_handler_rejects_every_redirect(self):
        handler = release.NoAuthenticatedRedirect()
        request = release.urllib.request.Request(
            "https://api.github.com/repos/owner/repo/releases", headers={"Authorization": "Bearer dummy"})
        for code in (301, 302, 303, 307, 308):
            with self.subTest(code=code):
                self.assertIsNone(handler.redirect_request(request, None, code, "redirect", {}, "https://evil.example/"))

    def test_real_opener_rejects_redirects_without_second_request(self):
        from email.message import Message
        from io import BytesIO
        from urllib.response import addinfourl

        real_build = release.urllib.request.build_opener
        for method in ("GET", "POST", "PATCH"):
            for code in (301, 302, 303, 307, 308):
                with self.subTest(method=method, code=code):
                    class FakeHttps(release.urllib.request.BaseHandler):
                        handler_order = 100
                        def __init__(self): self.seen = []
                        def https_open(self, request):
                            self.seen.append(request.full_url)
                            headers = Message()
                            headers["Location"] = "https://evil.example/collect"
                            result = addinfourl(BytesIO(b""), headers, request.full_url, code)
                            result.msg = "redirect"
                            return result
                    transport = FakeHttps()
                    def build(*handlers):
                        return real_build(*handlers, transport)
                    with patch.object(release.urllib.request, "build_opener", side_effect=build):
                        with self.assertRaises(HTTPError) as caught:
                            release.api("https://api.github.com/example", "dummy", method, b"data" if method != "GET" else None)
                    self.assertEqual(caught.exception.code, code)
                    self.assertEqual(transport.seen, ["https://api.github.com/example"])

    def test_api_installs_no_redirect_handler_and_preserves_request(self):
        class Response:
            def __enter__(self): return self
            def __exit__(self, *_): pass
            def read(self): return b'{"ok": true}'
        class Opener:
            def open(self, request, timeout):
                self.request, self.timeout = request, timeout
                return Response()
        opener = Opener()
        with patch.object(release.urllib.request, "build_opener", return_value=opener) as build:
            self.assertEqual(release.api("https://api.github.com/example", "dummy"), {"ok": True})
        self.assertIsInstance(build.call_args.args[0], release.NoAuthenticatedRedirect)
        self.assertEqual(opener.request.get_header("Authorization"), "Bearer dummy")
        self.assertEqual(opener.timeout, 60)

    def test_other_tag_is_rejected_before_api_calls(self):
        with self.assertRaisesRegex(ValueError, "alpha, beta, or v2.0.0 stable tags"):
            self.publish("v2.0.0-rc.1")
        self.assertEqual(self.calls, [])


if __name__ == "__main__":
    unittest.main()
