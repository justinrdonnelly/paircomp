"""Validate a selected commit and, optionally, a stable CLI release tag."""

import argparse
import json
import re
import subprocess
import tomllib


STABLE_VERSION = r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def tag_version(tag):
    if not re.fullmatch(r"paircomp-v" + STABLE_VERSION, tag):
        raise ValueError(f"Not a stable CLI release tag: {tag}")
    return tag.removeprefix("paircomp-v")


def metadata(commit):
    commit = git("rev-parse", "--verify", f"{commit}^{{commit}}")
    manifest = tomllib.loads(git("show", f"{commit}:paircomp-cli/Cargo.toml"))
    version = manifest["package"]["version"]
    suffix = r"(?:-[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?(?:\+[0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*)?"
    if not re.fullmatch(STABLE_VERSION + suffix, version):
        raise ValueError(f"Invalid CLI package version: {version}")
    return {
        "commit": commit,
        "version": version,
        "source_date_epoch": int(git("show", "-s", "--format=%ct", commit)),
    }


def validate(commit, tag=None, main_ref="refs/remotes/origin/main"):
    info = metadata(commit)
    if tag is not None:
        if tag_version(tag) != info["version"]:
            raise ValueError("Tag version does not match the paircomp CLI crate version")
        tagged = git("rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}")
        if tagged != info["commit"]:
            raise ValueError("Tag no longer points to the selected commit")
        result = subprocess.run(
            ["git", "merge-base", "--is-ancestor", info["commit"], main_ref],
            check=False,
        )
        if result.returncode != 0:
            raise ValueError("Tagged commit is not in fetched remote main history")
    return info


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--commit", required=True)
    parser.add_argument("--tag")
    parser.add_argument("--main-ref", default="refs/remotes/origin/main")
    parser.add_argument("--github-output")
    args = parser.parse_args()
    info = validate(args.commit, args.tag, args.main_ref)
    if args.github_output:
        with open(args.github_output, "a", encoding="utf-8") as output:
            for key, value in info.items():
                output.write(f"{key}={value}\n")
    print(json.dumps(info))


if __name__ == "__main__":
    try:
        main()
    except (ValueError, subprocess.CalledProcessError) as error:
        raise SystemExit(f"Release validation failed: {error}") from error
