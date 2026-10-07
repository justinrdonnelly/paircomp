#!/usr/bin/env bash
# Build committed sources only. Usage: bash scripts/release/build.sh all|gnu|musl [COMMIT] [OUTPUT]
set -euo pipefail

kind=${1:-all}
selected_commit=${2:-HEAD}
output=${3:-dist}
case "$kind" in all|gnu|musl) ;; *) echo 'Expected all, gnu, or musl' >&2; exit 2 ;; esac
if [[ $(id -u) == 0 ]]; then
    echo 'Release builds require a normal user and rootless Podman; do not use sudo.' >&2
    exit 2
fi
if [[ $(uname -m) != x86_64 ]]; then
    echo 'Release builds require a native x86-64 Linux host.' >&2
    exit 2
fi
if [[ $(podman info --format '{{.Host.Security.Rootless}}') != true ]]; then
    echo 'Podman must run rootless.' >&2
    exit 2
fi

repo=$(git rev-parse --show-toplevel)
cd "$repo"
commit=$(git rev-parse --verify "$selected_commit^{commit}")
version=$(python3 scripts/release/validate.py --commit "$commit" | python3 -c 'import json,sys; print(json.load(sys.stdin)["version"])')
epoch=$(git show -s --format=%ct "$commit")
mkdir -p "$output"
output=$(cd "$output" && pwd)
work=$(mktemp -d)
trap 'rm -rf -- "$work"' EXIT
mkdir -p "$work/source"
git archive "$commit" | tar -x -C "$work/source"
# Pins and container scripts come from the same snapshot as the application.
# shellcheck source=scripts/release/pins.env
source "$work/source/scripts/release/pins.env"
lock_sha=$(sha256sum "$work/source/Cargo.lock" | cut -d ' ' -f 1)
podman --version
targets=("$kind")
if [[ "$kind" == all ]]; then targets=(gnu musl); fi

for flavor in "${targets[@]}"; do
    target="x86_64-unknown-linux-$flavor"
    image=$GNU_IMAGE
    if [[ "$flavor" == musl ]]; then image=$MUSL_IMAGE; fi
    archive="paircomp-$version-$target.tar.gz"
    if [[ -e "$output/$archive" ]]; then
        echo "Refusing to overwrite $output/$archive; use an empty output directory." >&2
        exit 2
    fi
    mkdir -p "$work/$flavor/build" "$work/$flavor/output"
    # Only disposable directories get SELinux labels. Never mount the checkout,
    # credentials, host home, or container socket into a build container.
    run=(podman run --rm --pull=never --platform=linux/amd64
         --userns=keep-id --user "$(id -u):$(id -g)"
         --cap-drop=all --security-opt=no-new-privileges
         -v "$work/source:/source:ro,Z"
         -v "$work/$flavor/build:/build:Z"
         -v "$work/$flavor/output:/output:Z"
         --workdir /source
         --env "PAIRCOMP_VERSION=$version" --env "SOURCE_COMMIT=$commit"
         --env "SOURCE_DATE_EPOCH=$epoch" --env "LOCKFILE_SHA256=$lock_sha"
         --env "TARGET=$target" --env "BUILDER_IMAGE=$image")
    podman pull --platform=linux/amd64 "$image"
    if [[ "$image" != "$GNU_IMAGE" ]]; then podman pull --platform=linux/amd64 "$GNU_IMAGE"; fi
    "${run[@]}" "$image" sh /source/scripts/release/container-build.sh
    # Even the musl archive uses the same pinned GNU tar/gzip environment.
    "${run[@]}" --network=none "$GNU_IMAGE" sh /source/scripts/release/container-package.sh
    # Exercise the extracted download in its native libc environment too.
    "${run[@]}" --network=none "$image" sh /source/scripts/release/smoke.sh \
        "/build/extracted/paircomp-$version-$target/paircomp" "$version"
    if [[ $(stat -c %u "$work/$flavor/output/$archive") != "$(id -u)" ]]; then
        echo 'Container output is not owned by the calling user.' >&2
        exit 1
    fi
    cp "$work/$flavor/output/$archive" "$output/$archive"
done
if [[ "$kind" == all ]]; then
    python3 scripts/release/artifacts.py --commit "$commit" --write-checksums "$output"
fi
