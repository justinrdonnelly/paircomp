"""Create or complete an unpublished draft without replacing notes or assets."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import subprocess
import tarfile
from urllib.error import HTTPError
from urllib.parse import quote, urlencode, urlsplit
from urllib.request import HTTPRedirectHandler, Request, build_opener

from artifacts import read_build_info, sha256, verify_bundle
from validate import tag_version, validate


class SafeRedirect(HTTPRedirectHandler):
    def redirect_request(self, request, fp, code, msg, headers, newurl):
        redirected = super().redirect_request(request, fp, code, msg, headers, newurl)
        if urlsplit(newurl).scheme != "https":
            raise ValueError("Refusing an insecure GitHub download redirect")
        if redirected and urlsplit(request.full_url).netloc != urlsplit(newurl).netloc:
            redirected.remove_header("Authorization")
        return redirected


class GitHub:
    def __init__(self, repository, token):
        if not repository or not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
            raise ValueError("Invalid GitHub repository")
        self.base = f"https://api.github.com/repos/{repository}"
        self.token = token
        self.opener = build_opener(SafeRedirect())

    def request(self, method, path, payload=None, *, binary=False):
        url = path if path.startswith("https://") else self.base + path
        if urlsplit(url).netloc not in {"api.github.com", "uploads.github.com"}:
            raise ValueError("Unexpected GitHub API/upload host")
        data = payload
        if not binary and payload is not None:
            data = json.dumps(payload).encode()
        accept = "application/vnd.github+json"
        if binary and method == "GET":
            accept = "application/octet-stream"
        headers = {
            "Authorization": f"Bearer {self.token}",
            "Accept": accept,
            "X-GitHub-Api-Version": "2022-11-28",
            "User-Agent": "paircomp-draft-release",
        }
        if data is not None:
            headers["Content-Type"] = "application/octet-stream" if binary else "application/json"
        request = Request(url, data=data, headers=headers, method=method)
        with self.opener.open(request, timeout=60) as response:
            if binary and method == "GET":
                return hashlib.file_digest(response, "sha256").hexdigest()
            return json.load(response)

    def get(self, path):
        return self.request("GET", path)

    def pages(self, path):
        page = 1
        while True:
            items = self.get(f"{path}?per_page=100&page={page}")
            yield from items
            if len(items) < 100:
                return
            page += 1

    def asset_sha256(self, asset):
        # Older assets may lack the digest field. Hash the authenticated download
        # in that case, dropping the token on cross-host redirects.
        digest = asset.get("digest")
        if digest and re.fullmatch(r"sha256:[a-f0-9]{64}", digest):
            return digest.removeprefix("sha256:")
        return self.request("GET", f"/releases/assets/{asset['id']}", binary=True)


def check_tag(api, tag, commit):
    ref = api.get("/git/ref/tags/" + quote(tag, safe=""))["object"]
    for _ in range(10):
        if ref["type"] == "commit":
            if ref["sha"] != commit:
                raise ValueError("Remote tag changed; refusing to modify a release")
            return
        if ref["type"] != "tag":
            break
        ref = api.get("/git/tags/" + ref["sha"])["object"]
    raise ValueError("Remote tag does not resolve to a commit")


def require_draft(release):
    if not release["draft"]:
        raise ValueError("Release is published; refusing to modify it")


def description(info, notes, build_info):
    version = info["version"]
    gnu = build_info["x86_64-unknown-linux-gnu"]
    musl = build_info["x86_64-unknown-linux-musl"]
    return f"""Describe the changes in this release here.

{notes}

### Downloads and compatibility

- `paircomp-{version}-x86_64-unknown-linux-gnu.tar.gz`: x86-64 Linux, glibc {gnu['minimum_glibc']} or newer, with the standard libraries listed in `BUILD-INFO.txt`. Built and tested in Debian 12.
- `paircomp-{version}-x86_64-unknown-linux-musl.tar.gz`: x86-64 Linux, statically linked; no dynamic loader, shared libraries, or separate musl installation required. Built and tested in Alpine 3.22, also exercised in Debian 12.
- `SHA256SUMS`: SHA-256 checksums for both archives. Run `sha256sum -c SHA256SUMS` with both downloads in the same directory.

Each archive contains `paircomp`, `README.md`, `COPYING`, and `BUILD-INFO.txt` in a matching top-level directory.

### Build information

Source commit: `{info['commit']}`. Rust: `{gnu['rust_toolchain']}`. Default features, committed lockfile (`{gnu['lockfile_sha256']}`), portable x86-64 CPU settings.

- GNU builder and packaging image: `{gnu['builder_image']}`
- musl builder: `{musl['builder_image']}`

The toolchain and images are pinned and archive metadata is normalized. Automated reproducibility comparisons are planned separately. Crates.io publication is manual.
"""


def prepare_draft(api, tag, info, paths, body):
    if tag_version(tag) != info["version"]:
        raise ValueError("Tag version differs from the artifact version")
    check_tag(api, tag, info["commit"])
    releases = [release for release in api.pages("/releases") if release["tag_name"] == tag]
    if len(releases) > 1:
        raise ValueError("Multiple releases use this tag; recover manually")
    release = releases[0] if releases else None
    existing = {}
    if release:
        require_draft(release)
        for asset in api.pages(f"/releases/{release['id']}/assets"):
            if asset["name"] in existing:
                raise ValueError("Duplicate release assets; recover manually")
            existing[asset["name"]] = asset
    # Compare every workflow-owned asset before doing any mutation.
    missing = []
    for path in paths:
        asset = existing.get(path.name)
        if asset is None:
            missing.append(path)
        elif (
            asset.get("state") != "uploaded"
            or asset["size"] != path.stat().st_size
            or api.asset_sha256(asset) != sha256(path)
        ):
            raise ValueError(f"Conflicting asset {path.name}; recover manually (nothing replaced)")
    if release is None:
        notes = api.request("POST", "/releases/generate-notes", {
            "tag_name": tag,
            "target_commitish": info["commit"],
        })
        check_tag(api, tag, info["commit"])
        release = api.request("POST", "/releases", {
            "tag_name": tag,
            "target_commitish": info["commit"],
            "name": f"Paircomp {info['version']}",
            "body": body(notes["body"]),
            "draft": True,
            "prerelease": False,
        })
        require_draft(release)
    print("Draft: " + release["html_url"], flush=True)
    print("Expected assets: " + ", ".join(path.name for path in paths), flush=True)
    for path in missing:
        check_tag(api, tag, info["commit"])
        current = api.get(f"/releases/{release['id']}")
        require_draft(current)
        if current["tag_name"] != tag:
            raise ValueError("Draft tag changed; refusing to upload")
        upload_url = current["upload_url"].split("{", 1)[0] + "?" + urlencode({"name": path.name})
        asset = api.request("POST", upload_url, path.read_bytes(), binary=True)
        if (
            asset.get("state") != "uploaded"
            or asset["size"] != path.stat().st_size
            or api.asset_sha256(asset) != sha256(path)
        ):
            raise ValueError(f"Upload verification failed: {path.name}; draft remains unpublished")
        print("Uploaded: " + path.name, flush=True)
    check_tag(api, tag, info["commit"])
    require_draft(api.get(f"/releases/{release['id']}"))
    return release["html_url"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tag", required=True)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--artifacts", required=True, type=Path)
    parser.add_argument("--repository", default=os.environ.get("GITHUB_REPOSITORY"))
    args = parser.parse_args()
    info = validate(args.commit, args.tag)
    paths = verify_bundle(args.artifacts, info)
    builds = {}
    # The archives were checked above before any remote writes.
    for path in paths[:-1]:
        with tarfile.open(path) as archive:
            data = read_build_info(archive, path.name.removesuffix(".tar.gz"))
        builds[data["target"]] = data
    token = os.environ["GITHUB_TOKEN"]
    url = prepare_draft(
        GitHub(args.repository, token), args.tag, info, paths,
        lambda notes: description(info, notes, builds),
    )
    if os.environ.get("GITHUB_STEP_SUMMARY"):
        with open(os.environ["GITHUB_STEP_SUMMARY"], "a", encoding="utf-8") as summary:
            summary.write(f"Draft release: {url}\n\nExpected assets:\n\n")
            for path in paths:
                summary.write(f"- `{path.name}`\n")


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, KeyError, HTTPError, tarfile.TarError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Draft preparation failed: {error}") from error
