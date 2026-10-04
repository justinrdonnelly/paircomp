"""Check release archive metadata and create/verify the combined SHA256SUMS."""

import argparse
import hashlib
from pathlib import Path
import subprocess
import tarfile

from validate import metadata


TARGETS = ("x86_64-unknown-linux-gnu", "x86_64-unknown-linux-musl")
FILES = ("paircomp", "README.md", "COPYING", "BUILD-INFO.txt")
GNU_LIBRARIES = {"libc.so.6", "libm.so.6", "libgcc_s.so.1", "ld-linux-x86-64.so.2"}


def sha256(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def source_bytes(commit, path):
    return subprocess.check_output(["git", "show", f"{commit}:{path}"])


def read_pins(data):
    return dict(
        line.split("=", 1)
        for line in data.decode().splitlines()
        if line and not line.startswith("#")
    )


def archive_names(version):
    return [f"paircomp-{version}-{target}.tar.gz" for target in TARGETS]


def read_build_info(archive, root):
    data = archive.extractfile(f"{root}/BUILD-INFO.txt").read().decode()
    return dict(line.split("=", 1) for line in data.splitlines())


def verify_archives(directory, info):
    directory = Path(directory)
    commit = info["commit"]
    pins = read_pins(source_bytes(commit, "scripts/release/pins.env"))
    lock_sha = hashlib.sha256(source_bytes(commit, "Cargo.lock")).hexdigest()
    for target, filename in zip(TARGETS, archive_names(info["version"])):
        path = directory / filename
        root = filename.removesuffix(".tar.gz")
        with path.open("rb") as stream:
            header = stream.read(10)
        if header[:4] != b"\x1f\x8b\x08\x00" or header[4:8] != b"\0" * 4:
            raise ValueError(f"Unnormalized gzip header: {filename}")
        with tarfile.open(path, "r:gz") as archive:
            members = archive.getmembers()
            expected_names = [root] + sorted(f"{root}/{name}" for name in FILES)
            if [member.name for member in members] != expected_names:
                raise ValueError(f"Unexpected archive contents/order: {filename}")
            for member in members:
                is_dir = member.name == root
                mode = 0o755 if is_dir or member.name == f"{root}/paircomp" else 0o644
                if (
                    (is_dir and not member.isdir())
                    or (not is_dir and not member.isfile())
                    or member.mode != mode
                    or member.uid != 0
                    or member.gid != 0
                    or member.uname
                    or member.gname
                    or member.mtime != info["source_date_epoch"]
                ):
                    raise ValueError(f"Unexpected archive permissions/metadata: {member.name}")
            for name in ("README.md", "COPYING"):
                if archive.extractfile(f"{root}/{name}").read() != source_bytes(commit, name):
                    raise ValueError(f"Archive {name} differs from the source commit")
            if archive.extractfile(f"{root}/paircomp").read(4) != b"\x7fELF":
                raise ValueError("Archive executable is not ELF")
            build_info = read_build_info(archive, root)
        is_gnu = target.endswith("-gnu")
        expected = {
            "source_commit": commit,
            "rust_toolchain": pins["RUST_VERSION"],
            "builder_image": pins["GNU_IMAGE" if is_gnu else "MUSL_IMAGE"],
            "packaging_image": pins["GNU_IMAGE"],
            "target": target,
            "lockfile_sha256": lock_sha,
            "minimum_glibc": pins["GLIBC_BASELINE"] if is_gnu else "none",
            "cpu": "x86-64",
            "source_date_epoch": str(info["source_date_epoch"]),
        }
        if any(build_info.get(key) != value for key, value in expected.items()):
            raise ValueError(f"Build information does not match selected source/pins: {filename}")
        libraries = build_info.get("shared_libraries", "missing").split()
        if is_gnu:
            if not libraries or set(libraries) - GNU_LIBRARIES:
                raise ValueError("Unexpected GNU shared libraries")
        elif libraries:
            raise ValueError("musl executable requires a shared library")
    return [directory / name for name in archive_names(info["version"])]


def checksums(paths):
    return "".join(f"{sha256(path)}  {path.name}\n" for path in paths)


def verify_bundle(directory, info, write_checksums=False):
    directory = Path(directory)
    paths = verify_archives(directory, info)
    sums = checksums(paths)
    sums_path = directory / "SHA256SUMS"
    if write_checksums:
        sums_path.write_text(sums, encoding="utf-8")
    elif sums_path.read_text(encoding="utf-8") != sums:
        raise ValueError("SHA256SUMS does not match the two archives")
    return paths + [sums_path]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--write-checksums", action="store_true")
    parser.add_argument("directory", type=Path)
    args = parser.parse_args()
    paths = verify_bundle(args.directory, metadata(args.commit), args.write_checksums)
    print("Verified: " + ", ".join(path.name for path in paths))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, OSError, tarfile.TarError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Artifact verification failed: {error}") from error
