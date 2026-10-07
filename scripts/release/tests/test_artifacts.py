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

    def make_archive(self, target, *, changed_info=None, bad_mode=False, extra=False, gzip_mtime=0, readme=None):
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
            "paircomp": b"\x7fELFsynthetic fixture",
            "README.md": readme if readme is not None else Path("README.md").read_bytes(),
            "COPYING": Path("COPYING").read_bytes(),
            "BUILD-INFO.txt": "".join(f"{key}={value}\n" for key, value in build_info.items()).encode(),
        }
        if extra:
            files["unexpected"] = b"extra entry"
        output = io.BytesIO()
        with tarfile.open(fileobj=output, mode="w", format=tarfile.GNU_FORMAT) as archive:
            directory = tarfile.TarInfo(root)
            directory.type = tarfile.DIRTYPE
            directory.mode = 0o755
            directory.mtime = self.info["source_date_epoch"]
            archive.addfile(directory)
            for name, data in sorted(files.items()):
                member = tarfile.TarInfo(f"{root}/{name}")
                member.size = len(data)
                member.mode = 0o755 if name == "paircomp" else 0o644
                if bad_mode and name == "paircomp":
                    member.mode = 0o777
                member.mtime = directory.mtime
                archive.addfile(member, io.BytesIO(data))
        path = Path("dist") / (root + ".tar.gz")
        path.write_bytes(gzip.compress(output.getvalue(), mtime=gzip_mtime))
        return path

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
