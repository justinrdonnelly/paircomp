import gzip
import hashlib
import io
import os
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import artifacts
from validate import metadata


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        previous = os.getcwd()
        self.addCleanup(os.chdir, previous)
        os.chdir(self.temp.name)
        for directory in ("paircomp-cli", "scripts/release", "dist"):
            Path(directory).mkdir(parents=True)
        Path("paircomp-cli/Cargo.toml").write_text('[package]\nversion = "1.2.3"\n')
        Path("README.md").write_bytes(b"Source README\n")
        Path("COPYING").write_bytes(b"Source license\n")
        Path("Cargo.lock").write_bytes(b"Committed lockfile\n")
        self.pins = {
            "RUST_VERSION": "1.98.1",
            "GNU_IMAGE": "rust:gnu@sha256:" + "1" * 64,
            "MUSL_IMAGE": "rust:musl@sha256:" + "2" * 64,
            "GLIBC_BASELINE": "2.36",
        }
        Path("scripts/release/pins.env").write_text(
            "".join(f"{key}={value}\n" for key, value in self.pins.items())
        )
        self.git("init", "-q")
        self.git("add", ".")
        self.git("-c", "user.name=Release Test", "-c", "user.email=release-test@example.invalid", "commit", "-qm", "Artifact fixture")
        self.info = metadata("HEAD")
        self.lock_sha = hashlib.sha256(Path("Cargo.lock").read_bytes()).hexdigest()
        for target in artifacts.TARGETS:
            self.make_archive(target)

    def git(self, *args):
        return subprocess.check_output(["git", *args], text=True, stderr=subprocess.PIPE).strip()

    def make_archive(
        self, target, *, changed_info=None, bad_mode=False, extra=False,
        gzip_mtime=0, readme=None, directory="dist",
        executable=b"\x7fELFsynthetic fixture",
    ):
        root = f"paircomp-1.2.3-{target}"
        is_gnu = target.endswith("-gnu")
        build_info = {
            "source_commit": self.info["commit"],
            "rust_toolchain": "1.98.1",
            "builder_image": self.pins["GNU_IMAGE" if is_gnu else "MUSL_IMAGE"],
            "packaging_image": self.pins["GNU_IMAGE"],
            "target": target,
            "lockfile_sha256": self.lock_sha,
            "minimum_glibc": "2.36" if is_gnu else "none",
            "shared_libraries": "libc.so.6 libgcc_s.so.1" if is_gnu else "",
            "cpu": "x86-64",
            "source_date_epoch": str(self.info["source_date_epoch"]),
        }
        build_info.update(changed_info or {})
        files = {
            # Synthetic ELF marker tests archive validation, not binary linking.
            "paircomp": executable,
            "README.md": readme if readme is not None else Path("README.md").read_bytes(),
            "COPYING": Path("COPYING").read_bytes(),
            "BUILD-INFO.txt": "".join(f"{key}={value}\n" for key, value in build_info.items()).encode(),
        }
        if extra:
            files["unexpected"] = b"extra entry"
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w", format=tarfile.GNU_FORMAT) as archive:
            root_entry = tarfile.TarInfo(root)
            root_entry.type = tarfile.DIRTYPE
            root_entry.mode = 0o755
            root_entry.mtime = self.info["source_date_epoch"]
            archive.addfile(root_entry)
            for name, data in sorted(files.items()):
                member = tarfile.TarInfo(f"{root}/{name}")
                member.size = len(data)
                member.mode = 0o755 if name == "paircomp" else 0o644
                if bad_mode and name == "paircomp":
                    member.mode = 0o777
                member.mtime = root_entry.mtime
                archive.addfile(member, io.BytesIO(data))
        path = Path(directory) / (root + ".tar.gz")
        path.write_bytes(gzip.compress(output.getvalue(), mtime=gzip_mtime))
        return path

    def make_comparison_bundle(self):
        artifacts.verify_bundle("dist", self.info, write_checksums=True)
        Path("rebuild").mkdir()
        for target in artifacts.TARGETS:
            self.make_archive(target, directory="rebuild")
        return artifacts.verify_bundle("rebuild", self.info, write_checksums=True)

    def test_valid_bundle_and_checksum_verification(self):
        paths = artifacts.verify_bundle("dist", self.info, write_checksums=True)
        self.assertEqual(len(paths), 3)
        self.assertEqual(artifacts.verify_bundle("dist", self.info), paths)
        for path in paths[:-1]:
            self.assertIn(hashlib.sha256(path.read_bytes()).hexdigest(), paths[-1].read_text())

    def test_checksum_conflict(self):
        artifacts.verify_bundle("dist", self.info, write_checksums=True)
        Path("dist/SHA256SUMS").write_text("wrong checksums\n")
        with self.assertRaisesRegex(ValueError, "SHA256SUMS"):
            artifacts.verify_bundle("dist", self.info)

    def test_missing_archive_blocks_bundle(self):
        Path("dist", artifacts.archive_names("1.2.3")[1]).unlink()
        with self.assertRaises(FileNotFoundError):
            artifacts.verify_bundle("dist", self.info, write_checksums=True)
        self.assertFalse(Path("dist/SHA256SUMS").exists())

    def test_wrong_commit_toolchain_image_or_lockfile_rejected(self):
        target = artifacts.TARGETS[0]
        for key in ("source_commit", "rust_toolchain", "builder_image", "packaging_image", "target", "lockfile_sha256"):
            with self.subTest(key=key):
                self.make_archive(target, changed_info={key: "wrong"})
                with self.assertRaisesRegex(ValueError, "source/pins"):
                    artifacts.verify_archives("dist", self.info)

    def test_contents_permissions_and_gzip_timestamp_rejected(self):
        target = artifacts.TARGETS[0]
        for arguments, expected in (
            ({"extra": True}, "contents/order"),
            ({"bad_mode": True}, "permissions/metadata"),
            ({"gzip_mtime": 123}, "gzip header"),
            ({"readme": b"Different documentation"}, "README.md differs"),
        ):
            with self.subTest(arguments=arguments):
                self.make_archive(target, **arguments)
                with self.assertRaisesRegex(ValueError, expected):
                    artifacts.verify_archives("dist", self.info)

    def test_glibc_baseline_mismatch_rejected(self):
        for target in artifacts.TARGETS:
            for baseline in ("2.34", "2.37"):
                with self.subTest(target=target, baseline=baseline):
                    self.make_archive(target, changed_info={"minimum_glibc": baseline})
                    with self.assertRaisesRegex(ValueError, "source/pins"):
                        artifacts.verify_archives("dist", self.info)
            self.make_archive(target)

    def test_musl_shared_library_rejected(self):
        self.make_archive(artifacts.TARGETS[1], changed_info={"shared_libraries": "libc.so"})
        with self.assertRaisesRegex(ValueError, "shared library"):
            artifacts.verify_archives("dist", self.info)

    def test_working_tree_edits_do_not_change_source_provenance(self):
        Path("README.md").write_text("uncommitted edit")
        Path("Cargo.lock").write_text("uncommitted lockfile")
        artifacts.verify_bundle("dist", self.info, write_checksums=True)

    def test_independent_matching_bundles_are_not_modified(self):
        self.make_comparison_bundle()
        paths = [path for directory in ("dist", "rebuild") for path in Path(directory).iterdir()]
        before = {path: path.read_bytes() for path in paths}
        compared = artifacts.compare_bundles("dist", "rebuild", self.info)
        self.assertEqual([path.name for path in compared], artifacts.archive_names("1.2.3") + ["SHA256SUMS"])
        self.assertEqual({path: path.read_bytes() for path in paths}, before)

    def test_changed_executable_in_either_target_fails_reproducibility(self):
        self.make_comparison_bundle()
        for target in artifacts.TARGETS:
            with self.subTest(target=target):
                changed = self.make_archive(target, directory="rebuild", executable=b"\x7fELFdifferent executable")
                artifacts.verify_bundle("rebuild", self.info, write_checksums=True)
                with self.assertRaisesRegex(ValueError, "Reproducibility mismatch") as caught:
                    artifacts.compare_bundles("dist", "rebuild", self.info)
                self.assertIn(changed.name, str(caught.exception))
                self.assertIn(artifacts.sha256(changed), str(caught.exception))
                self.assertIn(artifacts.sha256(Path("dist", changed.name)), str(caught.exception))
                self.make_archive(target, directory="rebuild")
                artifacts.verify_bundle("rebuild", self.info, write_checksums=True)

    def test_comparison_rejects_wrong_source_in_either_bundle(self):
        self.make_comparison_bundle()
        target = artifacts.TARGETS[0]
        for directory in ("dist", "rebuild"):
            with self.subTest(directory=directory):
                self.make_archive(target, directory=directory, changed_info={"source_commit": "wrong"})
                with self.assertRaisesRegex(ValueError, "source/pins"):
                    artifacts.compare_bundles("dist", "rebuild", self.info)
                self.make_archive(target, directory=directory)

    def test_comparison_rejects_and_preserves_bad_checksums_in_either_bundle(self):
        self.make_comparison_bundle()
        for directory in ("dist", "rebuild"):
            with self.subTest(directory=directory):
                sums = Path(directory, "SHA256SUMS")
                original = sums.read_bytes()
                sums.write_bytes(b"wrong checksums\n")
                with self.assertRaisesRegex(ValueError, "SHA256SUMS"):
                    artifacts.compare_bundles("dist", "rebuild", self.info)
                self.assertEqual(sums.read_bytes(), b"wrong checksums\n")
                sums.write_bytes(original)

    def test_comparison_rejects_missing_archive_in_either_bundle(self):
        self.make_comparison_bundle()
        for directory in ("dist", "rebuild"):
            for filename in artifacts.archive_names("1.2.3"):
                with self.subTest(directory=directory, filename=filename):
                    path = Path(directory, filename)
                    original = path.read_bytes()
                    path.unlink()
                    with self.assertRaises(FileNotFoundError):
                        artifacts.compare_bundles("dist", "rebuild", self.info)
                    path.write_bytes(original)

    def test_checksum_file_must_match_as_bytes(self):
        self.make_comparison_bundle()
        sums = Path("rebuild/SHA256SUMS")
        sums.write_bytes(sums.read_bytes().replace(b"\n", b"\r\n"))
        # Both sets of checksums describe the same archives, but the files differ.
        artifacts.verify_bundle("rebuild", self.info)
        with self.assertRaisesRegex(ValueError, "Reproducibility mismatch") as caught:
            artifacts.compare_bundles("dist", "rebuild", self.info)
        self.assertIn("SHA256SUMS", str(caught.exception))
        self.assertNotIn(".tar.gz", str(caught.exception))

    def test_comparison_command_exit_status(self):
        self.make_comparison_bundle()
        command = [sys.executable, artifacts.__file__, "--commit", self.info["commit"], "--compare", "rebuild", "dist"]
        matched = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(matched.returncode, 0, matched.stderr)
        self.assertIn(self.info["commit"], matched.stdout)
        self.assertIn("SHA256SUMS", matched.stdout)
        self.make_archive(artifacts.TARGETS[0], directory="rebuild", executable=b"\x7fELFdifferent executable")
        artifacts.verify_bundle("rebuild", self.info, write_checksums=True)
        differed = subprocess.run(command, capture_output=True, text=True, check=False)
        self.assertEqual(differed.returncode, 1)
        self.assertIn("Reproducibility mismatch", differed.stderr)

    def test_comparison_command_cannot_rewrite_checksums(self):
        self.make_comparison_bundle()
        sums = Path("dist/SHA256SUMS")
        sums.write_bytes(b"preserve this invalid checksum file\n")
        refused = subprocess.run(
            [sys.executable, artifacts.__file__, "--commit", self.info["commit"],
             "--compare", "rebuild", "--write-checksums", "dist"],
            capture_output=True, text=True, check=False,
        )
        self.assertEqual(refused.returncode, 2)
        self.assertEqual(sums.read_bytes(), b"preserve this invalid checksum file\n")
