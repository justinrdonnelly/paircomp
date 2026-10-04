from contextlib import redirect_stdout
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest
from urllib.request import Request

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from draft import GitHub, SafeRedirect, description, prepare_draft


class FakeGitHub:
    """Stateful release service: failed requests keep successful prior uploads."""
    def __init__(self, commit, release=None):
        self.commit = commit
        self.release = release
        self.assets = []
        self.calls = []
        self.fail_upload = None
        self.annotated = False
        self.tag_checks = 0
        self.change_tag_after = None
        self.publish_before_upload = False

    def get(self, path):
        self.calls.append(("GET", path))
        if path.startswith("/git/ref/tags/"):
            self.tag_checks += 1
            commit = "b" * 40 if self.change_tag_after and self.tag_checks > self.change_tag_after else self.commit
            return {"object": {"type": "tag" if self.annotated else "commit", "sha": commit}}
        if path.startswith("/git/tags/"):
            return {"object": {"type": "commit", "sha": self.commit}}
        if self.publish_before_upload:
            self.release["draft"] = False
        return copy.deepcopy(self.release)

    def pages(self, path):
        self.calls.append(("GET", path))
        if path == "/releases":
            return iter([copy.deepcopy(self.release)] if self.release else [])
        return iter(copy.deepcopy(self.assets))

    def request(self, method, path, payload, *, binary=False):
        self.calls.append((method, path, payload))
        if path == "/releases/generate-notes":
            return {"body": "Generated GitHub notes"}
        if path == "/releases":
            self.release = dict(payload, id=123, html_url="https://github.com/test/repo/releases/123", upload_url="https://uploads.github.com/repos/test/repo/releases/123/assets{?name,label}")
            return copy.deepcopy(self.release)
        name = path.split("?name=", 1)[1]
        if name == self.fail_upload:
            raise OSError("Simulated upload failure")
        if any(asset["name"] == name for asset in self.assets):
            raise AssertionError("Attempt to replace/duplicate an asset")
        asset = {"name": name, "id": len(self.assets) + 1, "state": "uploaded", "size": len(payload), "digest": "sha256:" + hashlib.sha256(payload).hexdigest()}
        self.assets.append(asset)
        return asset

    def asset_sha256(self, asset):
        return asset["digest"].removeprefix("sha256:")


class DraftTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.paths = []
        for name in ("gnu.tar.gz", "musl.tar.gz", "SHA256SUMS"):
            path = Path(self.temp.name) / name
            path.write_bytes(name.encode())
            self.paths.append(path)
        self.info = {"commit": "a" * 40, "version": "1.2.3"}
        self.api = FakeGitHub(self.info["commit"])
        self.stdout = io.StringIO()
        self.quiet = redirect_stdout(self.stdout)
        self.quiet.__enter__()
        self.addCleanup(self.quiet.__exit__, None, None, None)

    def run_draft(self):
        return prepare_draft(self.api, "paircomp-v1.2.3", self.info, self.paths, lambda notes: "Editable summary\n" + notes)

    def writes(self):
        return [call for call in self.api.calls if call[0] != "GET"]

    def test_new_draft_contains_notes_and_uploads_expected_assets(self):
        self.assertIn("/123", self.run_draft())
        self.assertTrue(self.api.release["draft"])
        self.assertFalse(self.api.release["prerelease"])
        self.assertEqual(self.api.release["target_commitish"], self.info["commit"])
        self.assertIn("Generated GitHub notes", self.api.release["body"])
        self.assertEqual([asset["name"] for asset in self.api.assets], [path.name for path in self.paths])
        self.assertIn("Expected assets:", self.stdout.getvalue())

    def test_identical_rerun_preserves_edited_title_and_body(self):
        self.run_draft()
        self.api.release.update(name="My edited title", body="My edited notes")
        self.api.calls.clear()
        self.run_draft()
        self.assertEqual(self.writes(), [])
        self.assertEqual(self.api.release["name"], "My edited title")
        self.assertEqual(self.api.release["body"], "My edited notes")

    def test_unrelated_assets_are_preserved(self):
        self.run_draft()
        extra = {"name": "manual-notes.txt", "id": 99, "state": "uploaded", "size": 10}
        self.api.assets.append(extra)
        self.api.calls.clear()
        self.run_draft()
        self.assertEqual(self.api.assets[-1], extra)
        self.assertEqual(self.writes(), [])

    def test_partial_upload_then_rerun_completes_only_missing_assets(self):
        self.api.fail_upload = "musl.tar.gz"
        with self.assertRaisesRegex(OSError, "upload failure"):
            self.run_draft()
        self.assertTrue(self.api.release["draft"])
        self.assertEqual(len(self.api.assets), 1)
        self.api.release["body"] = "Edited after failure"
        self.api.fail_upload = None
        self.api.calls.clear()
        self.run_draft()
        self.assertEqual(len(self.api.assets), 3)
        self.assertEqual(len(self.writes()), 2)
        self.assertEqual(self.api.release["body"], "Edited after failure")

    def test_conflict_detected_before_uploading_other_missing_asset(self):
        self.run_draft()
        self.api.assets.pop(0)
        # Keep the size unchanged: the digest must catch differing contents.
        self.api.assets[0]["digest"] = "sha256:" + "0" * 64
        self.api.calls.clear()
        with self.assertRaisesRegex(ValueError, "Conflicting asset musl"):
            self.run_draft()
        self.assertEqual(self.writes(), [])

    def test_incomplete_asset_refused(self):
        self.run_draft()
        self.api.assets[0]["state"] = "starter"
        self.api.calls.clear()
        with self.assertRaisesRegex(ValueError, "Conflicting asset"):
            self.run_draft()
        self.assertEqual(self.writes(), [])

    def test_published_release_refused(self):
        self.run_draft()
        self.api.release["draft"] = False
        self.api.calls.clear()
        with self.assertRaisesRegex(ValueError, "published"):
            self.run_draft()
        self.assertEqual(self.writes(), [])

    def test_changed_remote_tag_refused(self):
        self.api.commit = "b" * 40
        with self.assertRaisesRegex(ValueError, "tag changed"):
            self.run_draft()
        self.assertEqual(self.writes(), [])

    def test_tag_rechecked_before_create(self):
        self.api.change_tag_after = 1
        with self.assertRaisesRegex(ValueError, "tag changed"):
            self.run_draft()
        self.assertIsNone(self.api.release)

    def test_tag_rechecked_before_each_upload(self):
        self.api.change_tag_after = 3
        with self.assertRaisesRegex(ValueError, "tag changed"):
            self.run_draft()
        self.assertEqual(len(self.api.assets), 1)
        self.assertTrue(self.api.release["draft"])

    def test_published_during_run_refused_before_upload(self):
        self.api.publish_before_upload = True
        with self.assertRaisesRegex(ValueError, "published"):
            self.run_draft()
        self.assertEqual(self.api.assets, [])

    def test_annotated_tag_resolves(self):
        self.api.annotated = True
        self.run_draft()
        self.assertEqual(len(self.api.assets), 3)

    def test_wrong_version_refused_before_api_calls(self):
        self.info["version"] = "1.2.4"
        with self.assertRaisesRegex(ValueError, "version differs"):
            self.run_draft()
        self.assertEqual(self.api.calls, [])

    def test_description_includes_compatibility_and_pins(self):
        data = {"minimum_glibc": "2.36", "rust_toolchain": "1.98.1", "lockfile_sha256": "lock-hash", "builder_image": "image@sha256:digest"}
        body = description(self.info, "Generated notes", {"x86_64-unknown-linux-gnu": data, "x86_64-unknown-linux-musl": data})
        for expected in ("glibc 2.36", "statically linked", "Generated notes", "lock-hash", "image@sha256:digest", "SHA256SUMS", self.info["commit"]):
            self.assertIn(expected, body)


class TransportTests(unittest.TestCase):
    def test_api_serializes_json_and_authenticated_binary_upload(self):
        api = GitHub("test/repo", "secret")
        calls = []

        class Opener:
            def open(self, request, timeout):
                calls.append(request)
                return io.BytesIO(b'{"id": 123}')

        api.opener = Opener()
        api.request("POST", "/releases", {"draft": True, "body": "Notes\nwith newlines"})
        self.assertEqual(calls[0].full_url, "https://api.github.com/repos/test/repo/releases")
        self.assertEqual(json.loads(calls[0].data), {"draft": True, "body": "Notes\nwith newlines"})
        self.assertEqual(calls[0].get_header("Authorization"), "Bearer secret")
        self.assertEqual(calls[0].get_header("Content-type"), "application/json")
        api.request("POST", "https://uploads.github.com/repos/test/repo/releases/123/assets?name=file", b"archive", binary=True)
        self.assertEqual(calls[1].data, b"archive")
        self.assertEqual(calls[1].get_header("Content-type"), "application/octet-stream")

    def test_api_hashes_binary_download_response(self):
        api = GitHub("test/repo", "secret")
        calls = []

        class Opener:
            def open(self, request, timeout):
                calls.append(request)
                return io.BytesIO(b"downloaded asset")

        api.opener = Opener()
        expected = hashlib.sha256(b"downloaded asset").hexdigest()
        self.assertEqual(api.request("GET", "/releases/assets/1", binary=True), expected)
        self.assertEqual(calls[0].get_header("Accept"), "application/octet-stream")

    def test_api_refuses_unexpected_upload_host(self):
        api = GitHub("test/repo", "secret")
        with self.assertRaisesRegex(ValueError, "Unexpected"):
            api.request("POST", "https://example.invalid/upload", b"archive", binary=True)

    def test_download_redirect_does_not_forward_token(self):
        request = Request("https://api.github.com/asset", headers={"Authorization": "Bearer secret"})
        redirected = SafeRedirect().redirect_request(request, None, 302, "Found", {}, "https://release-assets.githubusercontent.com/signed-download")
        self.assertIsNone(redirected.get_header("Authorization"))

    def test_insecure_redirect_refused(self):
        request = Request("https://api.github.com/asset", headers={"Authorization": "Bearer secret"})
        with self.assertRaisesRegex(ValueError, "insecure"):
            SafeRedirect().redirect_request(request, None, 302, "Found", {}, "http://example.invalid/download")

    def test_pagination_includes_later_drafts(self):
        api = GitHub("test/repo", "secret")
        pages = iter([[{"id": value} for value in range(100)], [{"id": 100}]])
        api.get = lambda path: next(pages)
        self.assertEqual(len(list(api.pages("/releases"))), 101)

    def test_older_asset_without_digest_is_downloaded(self):
        api = GitHub("test/repo", "secret")
        calls = []
        def request(method, path, **kwargs):
            calls.append((method, path, kwargs))
            return "computed-hash"
        api.request = request
        self.assertEqual(api.asset_sha256({"id": 3}), "computed-hash")
        self.assertEqual(calls, [("GET", "/releases/assets/3", {"binary": True})])
