#!/usr/bin/env python3
"""Publish an alpha, beta, or v2.0.0 stable release only after the workflow's blocking checks pass."""
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import tomllib
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.github.com"
TAG_PATTERN = re.compile(r"(?:v2\.0\.0|v2\.\d+\.\d+-(?:alpha|beta)\.\d+)\Z")


class NoAuthenticatedRedirect(urllib.request.HTTPRedirectHandler):
    """Do not forward an Authorization header to a redirect target."""

    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


def validated_upload_url(template, repository, release_id):
    """Bind the GitHub upload URL to this repository and release before using a token."""
    if not isinstance(template, str) or not isinstance(repository, str):
        raise ValueError("Invalid upload URL or repository")
    if type(release_id) is not int or release_id <= 0:
        raise ValueError("Invalid release ID")
    parts = repository.split("/")
    if len(parts) != 2 or any(not re.fullmatch(r"[A-Za-z0-9_.-]+", part) for part in parts):
        raise ValueError("Invalid repository")
    suffix = "{?name,label}"
    if not template.endswith(suffix):
        raise ValueError("Unexpected upload URL template")
    raw = template[:-len(suffix)]
    expected_path = f"/repos/{parts[0]}/{parts[1]}/releases/{release_id}/assets"
    parsed = urllib.parse.urlsplit(raw)
    if (not raw.isascii() or any(ord(c) <= 32 or ord(c) == 127 for c in raw)
            or parsed.scheme != "https" or parsed.netloc != "uploads.github.com"
            or parsed.path.casefold() != expected_path.casefold()
            or "?" in raw or "#" in raw):
        raise ValueError("Unexpected GitHub upload URL")
    return raw


def api(url, token, method="GET", data=None, content_type="application/json"):
    request = urllib.request.Request(
        url,
        data=data,
        method=method,
        headers={
            "Authorization": f"Bearer {token}",
            "Accept": "application/vnd.github+json",
            "Content-Type": content_type,
            "User-Agent": "shotkit-release-workflow",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    opener = urllib.request.build_opener(NoAuthenticatedRedirect())
    with opener.open(request, timeout=60) as response:
        return json.load(response)


def publish(exe_path, tag, commit, repository, token, run_url, manifest_path=None):
    if not TAG_PATTERN.fullmatch(tag):
        raise ValueError("Only v2 alpha, beta, or v2.0.0 stable tags may create a release")
    manifest_path = manifest_path or Path(__file__).resolve().parent.parent / "Cargo.toml"
    with Path(manifest_path).open("rb") as manifest:
        version = tomllib.load(manifest).get("package", {}).get("version")
    if version != tag.removeprefix("v"):
        raise ValueError("Release tag does not match Cargo package version")
    exe = Path(exe_path).read_bytes()
    if len(exe) < 100_000 or exe[:2] != b"MZ":
        raise ValueError("Tested artifact is not a valid-sized Windows executable")
    base = f"{API}/repos/{repository}"
    quoted_tag = urllib.parse.quote(tag, safe="")
    tagged_commit = api(f"{base}/commits/{quoted_tag}", token)
    if tagged_commit["sha"] != commit:
        raise ValueError("Tag does not resolve to the tested commit")
    try:
        api(f"{base}/releases/tags/{quoted_tag}", token)
    except urllib.error.HTTPError as error:
        if error.code != 404:
            raise
    else:
        raise ValueError("Release already exists; refusing to change it")
    filename = f"shotkit-{tag}-windows-x64.exe"
    checksum = f"{hashlib.sha256(exe).hexdigest()}  {filename}\n".encode()
    prerelease = tag != "v2.0.0"
    make_latest = "false" if prerelease else "true"
    stage = ("Beta" if "-beta." in tag else "Alpha") if prerelease else "stabile Version"
    body = (
        f"Automatisch getestete {stage}. CI: {run_url}\n\n"
        "Die EXE ist nicht signiert. Windows SmartScreen kann eine Warnung zeigen; "
        "nach eigener Prüfung kann die Ausführung über ‚Weitere Informationen‘ und "
        "‚Trotzdem ausführen‘ gestartet werden. Die SHA256-Datei dient zur "
        "Integritätsprüfung."
    )
    if not prerelease:
        body += (
            "\n\nRust ist die neue maßgebliche Version von ShotKit. "
            "C# v1.2.0 bleibt als historische, abgeschlossene Version erhalten. "
            "Echte Mixed-DPI- und Mehrmonitor-Tests stehen noch aus. "
            "Verpixelung ist keine sichere Schwärzung. "
            "Bei abruptem Abbruch beim Speichern können temporäre Dateien mit Bilddaten zurückbleiben. "
            "Diese Veröffentlichung ist kein vollständiges Security-Audit.\n\n"
            f"[Änderungen und bekannte Grenzen](https://github.com/{repository}/blob/{tag}/rust/CHANGELOG.md)"
        )
    payload = json.dumps({"tag_name": tag, "name": f"ShotKit {tag}", "body": body,
                          "draft": True, "prerelease": prerelease, "make_latest": make_latest}).encode()
    release = api(f"{base}/releases", token, "POST", payload)
    if release["draft"] is not True or release["prerelease"] is not prerelease:
        raise ValueError("GitHub did not create the expected draft release")
    upload_url = validated_upload_url(release["upload_url"], repository, release["id"])
    for name, content, content_type in (
        (filename, exe, "application/vnd.microsoft.portable-executable"),
        (filename + ".sha256", checksum, "text/plain"),
    ):
        asset = api(f"{upload_url}?name={urllib.parse.quote(name)}", token, "POST", content, content_type)
        expected = "sha256:" + hashlib.sha256(content).hexdigest()
        if asset["name"] != name or asset["size"] != len(content) or asset.get("digest") != expected:
            raise ValueError(f"Upload verification failed for {name}; draft retained")
    publish_payload = json.dumps({"draft": False, "make_latest": make_latest}).encode()
    result = api(f"{base}/releases/{release['id']}", token, "PATCH", publish_payload)
    if result["draft"] is not False or result["prerelease"] is not prerelease:
        raise ValueError("Release was not published with the expected prerelease status")
    return result["html_url"]


def main():
    if len(sys.argv) != 2:
        raise ValueError("Expected path to the tested executable")
    print(publish(sys.argv[1], os.environ["RELEASE_TAG"], os.environ["RELEASE_COMMIT"],
                  os.environ["REPOSITORY"], os.environ["GH_TOKEN"], os.environ["CI_RUN_URL"]))


if __name__ == "__main__":
    main()
