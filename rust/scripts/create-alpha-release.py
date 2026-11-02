#!/usr/bin/env python3
"""Publish a new alpha only after the calling workflow's blocking checks pass."""
import hashlib
import json
import os
from pathlib import Path
import re
import sys
import urllib.error
import urllib.parse
import urllib.request

API = "https://api.github.com"
TAG_PATTERN = re.compile(r"v2\.\d+\.\d+-alpha\.\d+\Z")


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
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def publish(exe_path, tag, commit, repository, token, run_url):
    if not TAG_PATTERN.fullmatch(tag):
        raise ValueError("Only v2 alpha tags may create a release")
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
    body = (
        f"Automatisch getestete Alpha. CI: {run_url}\n\n"
        "Die EXE ist nicht signiert. Windows SmartScreen kann eine Warnung zeigen; "
        "nach eigener Prüfung kann die Ausführung über ‚Weitere Informationen‘ und "
        "‚Trotzdem ausführen‘ gestartet werden. Die SHA256-Datei dient zur "
        "Integritätsprüfung."
    )
    payload = json.dumps({"tag_name": tag, "name": f"ShotKit {tag}", "body": body,
                          "draft": True, "prerelease": True, "make_latest": "false"}).encode()
    release = api(f"{base}/releases", token, "POST", payload)
    if not release["draft"] or not release["prerelease"]:
        raise ValueError("GitHub did not create a draft prerelease")
    upload_url = release["upload_url"].split("{", 1)[0]
    for name, content, content_type in (
        (filename, exe, "application/vnd.microsoft.portable-executable"),
        (filename + ".sha256", checksum, "text/plain"),
    ):
        asset = api(f"{upload_url}?name={urllib.parse.quote(name)}", token, "POST", content, content_type)
        expected = "sha256:" + hashlib.sha256(content).hexdigest()
        if asset["name"] != name or asset["size"] != len(content) or asset.get("digest") != expected:
            raise ValueError(f"Upload verification failed for {name}; draft retained")
    result = api(f"{base}/releases/{release['id']}", token, "PATCH", b'{"draft":false}')
    if result["draft"] or not result["prerelease"]:
        raise ValueError("Release was not published as a prerelease")
    return result["html_url"]


def main():
    if len(sys.argv) != 2:
        raise ValueError("Expected path to the tested executable")
    print(publish(sys.argv[1], os.environ["RELEASE_TAG"], os.environ["RELEASE_COMMIT"],
                  os.environ["REPOSITORY"], os.environ["GH_TOKEN"], os.environ["CI_RUN_URL"]))


if __name__ == "__main__":
    main()
