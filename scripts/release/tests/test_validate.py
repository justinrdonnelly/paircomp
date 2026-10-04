import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import validate


class ValidationTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        previous = os.getcwd()
        self.addCleanup(os.chdir, previous)
        os.chdir(self.temp.name)
        self.git("init", "-q")
        self.git("config", "user.name", "Release Test")
        self.git("config", "user.email", "release-test@example.invalid")
        Path("paircomp-cli").mkdir()
        Path("paircomp-core").mkdir()
        Path("paircomp-cli/Cargo.toml").write_text('[package]\nname = "paircomp"\nversion = "1.2.3"\n')
        # The independent core version must not influence tag validation.
        Path("paircomp-core/Cargo.toml").write_text('[package]\nname = "paircomp-core"\nversion = "9.8.7"\n')
        self.git("add", ".")
        self.git("commit", "-qm", "Fixture")
        self.commit = self.git("rev-parse", "HEAD")
        self.git("update-ref", "refs/remotes/origin/main", self.commit)
        self.git("tag", "paircomp-v1.2.3")

    def git(self, *args):
        return subprocess.check_output(["git", *args], text=True, stderr=subprocess.PIPE).strip()

    def test_stable_cli_tag_and_independent_core_version(self):
        self.assertEqual(validate.validate(self.commit, "paircomp-v1.2.3")["version"], "1.2.3")

    def test_annotated_tag(self):
        self.git("tag", "-af", "paircomp-v1.2.3", "-m", "Release")
        self.assertEqual(validate.validate(self.commit, "paircomp-v1.2.3")["commit"], self.commit)

    def test_invalid_tags(self):
        for tag in ("paircomp-core-v1.2.3", "paircomp-v1.2.3-rc.1", "paircomp-v1.2.3+build", "paircomp-v01.2.3", "paircomp-v1.2", "paircomp-v1.2.3/extra"):
            with self.subTest(tag=tag), self.assertRaisesRegex(ValueError, "stable CLI"):
                validate.validate(self.commit, tag)

    def test_version_mismatch(self):
        with self.assertRaisesRegex(ValueError, "version does not match"):
            validate.validate(self.commit, "paircomp-v1.2.4")

    def branch_commit(self):
        Path("change").write_text("branch work")
        self.git("add", ".")
        self.git("commit", "-qm", "Branch work")
        return self.git("rev-parse", "HEAD")

    def test_commit_outside_remote_main(self):
        commit = self.branch_commit()
        self.git("tag", "-f", "paircomp-v1.2.3")
        with self.assertRaisesRegex(ValueError, "remote main history"):
            validate.validate(commit, "paircomp-v1.2.3")

    def test_changed_local_tag(self):
        self.branch_commit()
        self.git("tag", "-f", "paircomp-v1.2.3")
        with self.assertRaisesRegex(ValueError, "no longer points"):
            validate.validate(self.commit, "paircomp-v1.2.3")

    def test_build_only_allows_untagged_branch(self):
        commit = self.branch_commit()
        self.assertEqual(validate.validate(commit)["commit"], commit)

    def test_tagged_ancestor_of_remote_main_is_allowed(self):
        newer_commit = self.branch_commit()
        self.git("update-ref", "refs/remotes/origin/main", newer_commit)
        self.assertEqual(validate.validate(self.commit, "paircomp-v1.2.3")["commit"], self.commit)

    def test_build_only_allows_development_version(self):
        Path("paircomp-cli/Cargo.toml").write_text('[package]\nname = "paircomp"\nversion = "1.3.0-dev.1"\n')
        commit = self.branch_commit()
        self.assertEqual(validate.validate(commit)["version"], "1.3.0-dev.1")
