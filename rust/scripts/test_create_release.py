import importlib.util
import json
import os
import subprocess
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
from urllib.error import HTTPError

spec = importlib.util.spec_from_file_location("release", Path(__file__).with_name("create-release.py"))
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
        self.changelog = Path(self.temp.name) / "CHANGELOG.md"
        self.write_notes("v2.0.0-alpha.14")
        ancestry_patch = patch.object(release, "validate_main_ancestry")
        self.ancestry = ancestry_patch.start()
        self.addCleanup(ancestry_patch.stop)
        self.calls = []
        self.existing = False
        self.bad_digest = False
        self.bad_upload_number = None
        self.upload_number = 0
        self.prerelease = True
        self.wrong_draft_status = False
        self.wrong_published_status = False
        self.upload_url = "https://uploads.github.com/repos/owner/repo/releases/4/assets{?name,label}"

    def write_notes(self, tag):
        self.changelog.write_text(
            f"# Changes\n\n## Version {tag[1:]}\n\n"
            "### Änderungen\n- Version-specific fixture change (#71).\n\n"
            "### Bekannte Fehler\n- Fixture limitation (#99).\n\n"
            "## Version 1.2.0\n\nDO NOT INCLUDE PREVIOUS NOTES.\n",
            encoding="utf-8",
        )

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
            return release.publish(self.exe, tag, "commit", "owner/repo", "token", "https://example.com/run", self.manifest, self.changelog, Path(self.temp.name))

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
                self.write_notes(tag)
                self.publish(tag)
                self.assertIs(json.loads(self.calls[2][2])["prerelease"], True)
                self.assertIn("Version-specific fixture change (#71).", json.loads(self.calls[2][2])["body"])
                self.assertIn("Fixture limitation (#99).", json.loads(self.calls[2][2])["body"])
                self.assertNotIn("DO NOT INCLUDE PREVIOUS NOTES", json.loads(self.calls[2][2])["body"])
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
        self.write_notes("v2.0.0-beta.1")
        self.assertEqual(self.publish("v2.0.0-beta.1"), "https://example.com/release")
        create = json.loads(self.calls[2][2])
        self.assertTrue(create["draft"] and create["prerelease"])
        self.assertIn(b"shotkit-v2.0.0-beta.1-windows-x64.exe", self.calls[4][2])

    def stable(self):
        self.write_notes("v2.0.1")
        self.manifest.write_text('[package]\nversion = "2.0.1"\n', encoding="utf-8")
        self.prerelease = False
        return self.publish("v2.0.1")

    def test_stable_becomes_latest_only_after_two_verified_uploads(self):
        from hashlib import sha256
        self.assertEqual(self.stable(), "https://example.com/release")
        self.assertEqual([c[1] for c in self.calls], ["GET", "GET", "POST", "POST", "POST", "PATCH"])
        create = json.loads(self.calls[2][2])
        self.assertIs(create["draft"], True)
        self.assertIs(create["prerelease"], False)
        self.assertEqual(create["make_latest"], "false")
        self.assertIn("Version-specific fixture change (#71).", create["body"])
        self.assertIn("Fixture limitation (#99).", create["body"])
        self.assertNotIn("DO NOT INCLUDE PREVIOUS NOTES", create["body"])
        filename = "shotkit-v2.0.1-windows-x64.exe"
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
            self.publish("v2.0.1")
        self.assertEqual(self.calls, [])

    def test_malformed_or_unsupported_tags_make_no_api_calls(self):
        for tag in ("v3.0.0", "v2.0.0-rc.1", "v2.0.0-extra", "v2.0.0\n", "v2.0.1+build.1", "v2.0.1-beta.01", "v2.00.1", "v2.0.1-beta.1\n"):
            with self.subTest(tag=tag):
                # Keep the other gates valid so rejection proves tag validation.
                self.manifest.write_text(
                    "[package]\nversion = " + json.dumps(tag[1:]) + "\n", encoding="utf-8"
                )
                self.write_notes(tag)
                with self.assertRaises(ValueError):
                    self.publish(tag)
                self.assertEqual(self.calls, [])

    def test_moved_stable_tag_is_rejected_before_release_creation(self):
        self.manifest.write_text('[package]\nversion = "2.0.1"\n', encoding="utf-8")
        self.write_notes("v2.0.1")
        with patch.object(release, "api", return_value={"sha": "other-commit"}) as api:
            with self.assertRaisesRegex(ValueError, "tested commit"):
                release.publish(self.exe, "v2.0.1", "commit", "owner/repo", "token", "https://example.com/run", self.manifest, self.changelog, Path(self.temp.name))
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

    def test_missing_or_ambiguous_notes_make_no_api_calls(self):
        for notes in (
            "# Changelog\n\n## Unreleased\nMissing target.\n",
            "## Version 2.0.0-alpha.14\n\n## Version 1.0.0\nOlder.\n",
            "## Version 2.0.0-alpha.14\nFirst.\n## Version 2.0.0-alpha.14\nSecond.\n",
        ):
            with self.subTest(notes=notes):
                self.changelog.write_text(notes, encoding="utf-8")
                with self.assertRaises(ValueError):
                    self.publish()
                self.assertEqual(self.calls, [])

    def test_off_main_commit_makes_no_api_calls(self):
        self.ancestry.side_effect = ValueError("Commit is not on main")
        with self.assertRaises(ValueError):
            self.publish()
        self.assertEqual(self.calls, [])

    def test_publish_checks_tested_commit_in_supplied_repository(self):
        self.publish()
        args, kwargs = self.ancestry.call_args
        self.assertEqual(args[:2], ("commit", Path(self.temp.name)))

    def test_other_tag_is_rejected_before_api_calls(self):
        with self.assertRaises(ValueError):
            self.publish("v2.0.0-rc.1")
        self.assertEqual(self.calls, [])


class ReleaseNotesTests(unittest.TestCase):
    def test_relative_release_links_use_versioned_repository_paths(self):
        notes = "[Anleitung](README.md) [Root](../README.md) [Web](https://example.com/x)"
        resolved = release.resolve_release_links(notes, "owner/repo", "v2.0.1")
        self.assertIn("https://github.com/owner/repo/blob/v2.0.1/rust/README.md", resolved)
        self.assertIn("https://github.com/owner/repo/blob/v2.0.1/README.md", resolved)
        self.assertIn("https://example.com/x", resolved)

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.path = Path(self.temp.name) / "CHANGELOG.md"

    def extract(self, text, tag="v2.0.1"):
        self.path.write_text(text, encoding="utf-8")
        return release.extract_release_notes(self.path, tag)

    def test_exact_section_for_stable_alpha_and_beta(self):
        for tag in ("v2.0.1", "v2.1.0-alpha.2", "v2.1.0-beta.3"):
            with self.subTest(tag=tag):
                body = "### Changes\n\n- Target fix (#71).\n\n### Known bugs\n\n- Target limitation."
                result = self.extract(
                    "# Changelog\n\n## Unreleased\nFuture changes.\n\n"
                    f"## Version {tag[1:]}\n\n{body}\n\n"
                    "## Version 1.2.0\nOld changes.\n", tag,
                )
                self.assertEqual(result.strip(), body)

    def test_version_heading_accepts_existing_description_suffix(self):
        for separator in ("–", "—", "-"):
            with self.subTest(separator=separator):
                self.assertEqual(self.extract(
                    f"## Version 2.0.1 {separator} Wartungsrelease\nTarget fix.\n"
                ).strip(), "Target fix.")

    def test_duplicate_version_with_description_is_rejected(self):
        with self.assertRaises(ValueError):
            self.extract("## Version 2.0.1\nFirst.\n## Version 2.0.1 – Wartung\nSecond.\n")

    def test_target_section_at_end_of_file(self):
        self.assertEqual(self.extract("## Version 2.0.1\n\nFinal fix.\n").strip(), "Final fix.")

    def test_rejects_missing_malformed_duplicate_or_empty_section(self):
        for text in (
            "## Unreleased\nTarget absent.\n",
            "## Version 2.0.10\nPrefix collision.\n",
            "## Version 2.0.1-beta.1\nPrerelease collision.\n",
            "## Version v2.0.1\nWrong leading v.\n",
            "## Version 2.0.1 extra\nNot a permitted suffix.\n",
            "### Version 2.0.1\nWrong heading level.\n",
            "## Version 2.0.1\n  \n\t\n",
            "## Version 2.0.1\n\n## Version 1.2.0\nOld content.\n",
            "## Version 2.0.1\nFirst.\n## Version 2.0.1\nSecond.\n",
            "## Version 2.0.1\n\n## Version 2.0.1\nSecond.\n",
        ):
            with self.subTest(text=text):
                with self.assertRaises(ValueError):
                    self.extract(text)

    def test_h2_without_version_also_ends_section(self):
        self.assertEqual(self.extract(
            "## Version 2.0.1\nTarget fix.\n## Maintenance\nDo not include.\n"
        ).strip(), "Target fix.")


class MainAncestryTests(unittest.TestCase):
    """Exercise Git itself using only disposable local repositories."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.repo = self.root / "work"
        self.remote = self.root / "origin.git"
        self.repo.mkdir()
        self.env = dict(os.environ, GIT_CONFIG_NOSYSTEM="1", GIT_CONFIG_GLOBAL=os.devnull)
        self.git("init", "--bare", str(self.remote))
        self.git("init", "--initial-branch=main")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release-test@example.invalid")
        self.git("commit", "--allow-empty", "-m", "main ancestor")
        self.ancestor = self.git("rev-parse", "HEAD")
        self.git("commit", "--allow-empty", "-m", "main tip")
        self.tip = self.git("rev-parse", "HEAD")
        self.git("remote", "add", "origin", str(self.remote))
        self.git("push", "origin", "main")
        self.git("checkout", "--orphan", "unrelated")
        self.git("commit", "--allow-empty", "-m", "unrelated history")
        self.unrelated = self.git("rev-parse", "HEAD")

    def git(self, *args):
        return subprocess.run(
            ["git", "-c", "core.hooksPath=/dev/null", "-c", "commit.gpgsign=false", *args],
            cwd=self.repo, env=self.env, check=True, text=True,
            stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        ).stdout.strip()

    def test_accepts_main_tip_and_ancestor(self):
        self.git("checkout", "--detach", self.tip)
        release.validate_main_ancestry(self.tip, self.repo)
        self.git("checkout", "--detach", self.ancestor)
        release.validate_main_ancestry(self.ancestor, self.repo)

    def test_rejects_mismatched_checkout(self):
        with self.assertRaises(ValueError):
            release.validate_main_ancestry(self.tip, self.repo)

    def test_rejects_unrelated_history(self):
        with self.assertRaises(ValueError):
            release.validate_main_ancestry(self.unrelated, self.repo)

    def test_rejects_missing_remote_main_even_if_local_main_exists(self):
        self.git("checkout", "--detach", self.ancestor)
        self.git("update-ref", "-d", "refs/remotes/origin/main")
        with self.assertRaises(ValueError):
            release.validate_main_ancestry(self.ancestor, self.repo)

    def test_rejects_unknown_commit(self):
        with self.assertRaises(ValueError):
            release.validate_main_ancestry("0" * 40, self.repo)

    def test_accepts_explicit_main_ref(self):
        self.git("checkout", "--detach", self.ancestor)
        self.git("update-ref", "refs/remotes/verification/main", self.tip)
        release.validate_main_ancestry(
            self.ancestor, self.repo, main_ref="refs/remotes/verification/main"
        )


if __name__ == "__main__":
    unittest.main()
