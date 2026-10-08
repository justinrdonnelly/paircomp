# Linux downloads and draft releases

The [release workflow](../.github/workflows/release.yml) builds the tagged source
commit for `x86_64-unknown-linux-gnu` and `x86_64-unknown-linux-musl`, verifies both
downloads, and prepares an **unpublished draft**. Review its title, replace the
editable summary, edit GitHub's generated notes, and publish it manually.
Crates.io publishing is a separate manual process. Existing published releases
are not backfilled or changed by this workflow.

## Build environment

Actions uses the explicit `ubuntu-26.04` x86-64 runner, which provides Podman and
ShellCheck. The runner and Podman versions appear in each build job's summary.
The runner provides orchestration; the containers determine the compiler and
libc baseline.
See GitHub's [runner software inventory](https://github.com/actions/runner-images/blob/main/images/ubuntu/Ubuntu2604-Readme.md).

All build pins live in [pins.env](../scripts/release/pins.env): Rust 1.98.1,
the official `1.98.1-bookworm` and `1.98.1-alpine3.22` **Linux AMD64 manifest
digests**, and the Debian 12 glibc 2.36 baseline. Both targets build natively in
their own libc environment. GNU performs Rust formatting and Clippy checks;
both environments run the complete workspace tests. Cargo uses `--locked`,
explicit targets, release mode, and default features. The official GNU image has
a minimal Rust profile, so the script installs Rust 1.98.1's rustfmt and Clippy
components in the disposable container layer.

GNU downloads require glibc **2.36 or newer**, the normal x86-64 loader, and the
standard libraries recorded in `BUILD-INFO.txt` (such as `libgcc_s.so.1`). The
packaging check rejects required glibc symbol versions above 2.36 or unexpected
shared libraries. musl downloads must have no ELF interpreter or `DT_NEEDED`
libraries and require no libc installation. Both use portable x86-64 CPU settings.
The workflow exercises GNU in Debian 12 and musl in Alpine 3.22 and Debian 12;
it does not claim testing on other distributions.

The host creates a disposable `git archive` snapshot of the selected commit;
uncommitted edits and untracked files are excluded. The build scripts and pins
must be committed in that selected revision. Podman must run **rootless** with
`--userns=keep-id`. Source mounts are read-only. Build/output mounts are writable,
and returned archives must belong to the invoking user. SELinux `:Z` labels
apply only to disposable directories, never the repository or final output
directory. GitHub tokens, checkout credentials, host home directories, and
container sockets are not mounted or forwarded into containers. See Podman's
[user mapping and volume documentation](https://docs.podman.io/en/latest/markdown/podman-run.1.html).

## Local builds

Use a normal user on an x86-64 Linux host with rootless Podman, Bash, Git,
Python 3.11 or newer, tar, and standard GNU utilities. Configure subordinate
UID/GID ranges and a working rootless storage/network setup according to
[Podman's rootless instructions](https://github.com/containers/podman/blob/main/docs/tutorials/rootless_tutorial.md).
The first build needs network access for the pinned images, crates, and Rust
check components. Do not use `sudo` or a rootful/remote Podman service.

Check startup, then build both downloads from the committed branch tip:

```sh
podman --version
podman info --format '{{.Host.Security.Rootless}}'
bash scripts/release/build.sh all HEAD dist
```

`Rootless` must be `true`; the script also checks the invoking UID and CPU
architecture. Successful builds establish container startup, mount access,
image pulls, and archive ownership. A container or check failure stops the
build. Use a fresh output directory for each build; existing archives are never
overwritten. A completed first archive can remain if the second build fails.

To select a commit or one target:

```sh
bash scripts/release/build.sh all paircomp-v1.2.3 /tmp/paircomp-1.2.3-downloads
bash scripts/release/build.sh musl HEAD /tmp/paircomp-musl-download
```

`build.sh all` verifies both archives and creates `SHA256SUMS` automatically.
One-target builds return only their archive. If you build GNU and musl separately
from the same commit, collect both archives in one directory, then verify their
source information and generate the combined checksum file:

```sh
python3 scripts/release/artifacts.py --commit HEAD --write-checksums dist
```

`--write-checksums` creates or overwrites `SHA256SUMS`. To verify an existing
bundle's archive metadata and checksums without rewriting that file, omit the
flag:

```sh
python3 scripts/release/artifacts.py --commit HEAD dist
```

For both commands, use the directory containing the two archives and the commit
that produced them; replace `HEAD` if you built another revision.

Each archive is named `paircomp-VERSION-x86_64-unknown-linux-{gnu,musl}.tar.gz`.
Inside its matching top-level directory are `paircomp` (0755), `README.md`,
`COPYING`, and `BUILD-INFO.txt` (0644). Build information records the source
commit, Rust version, builder and packaging digests, target, lockfile SHA-256,
CPU and libc/library requirements, and source timestamp.

Both archives use the pinned GNU image's tar/gzip. Entries have sorted names,
the source commit's timestamp, numeric owner/group zero, and fixed permissions;
gzip omits the timestamp and original filename. Compiler paths are consistent
and remapped; compiled release output is fresh.

GNU and musl Linux release archives have been reproduced byte for byte between
local builds, a VM, and GitHub Actions using the same source commit and this
documented, pinned build environment. Matching complete archives covers the
executables, bundled files, and archive metadata. Reproducibility applies to
this specified build recipe and environment, following the
[Reproducible Builds definition](https://reproducible-builds.org/docs/definition/).

Reproducibility has been verified manually; the workflow does not yet compare
independent builds automatically. To verify another commit, rebuild it with
the same recipe and pins and compare the complete archive hashes with the
corresponding GitHub Actions downloads.

## Test before releasing

The workflow's `validate` job runs ShellCheck and the release logic tests before
container builds. Run the same checks from the repository root with ShellCheck
and Python installed; Podman and GitHub credentials are not needed:

```sh
shellcheck -x scripts/release/*.sh
python3 -m unittest discover -s scripts/release/tests -v
```

ShellCheck is a [GPLv3](https://github.com/koalaman/shellcheck/blob/v0.11.0/LICENSE)
tool for checking shell scripts; `-x` follows the sourced `pins.env` file.
The Python tests cover stable/malformed/prerelease tags, independent core
versions, version mismatches, commits outside fetched remote `main`, changed
local/remote tags, draft creation, preserved edited titles/notes, identical
assets, partial uploads, conflicting/incomplete assets, published-release
refusal, pagination, and token handling on download redirects. They also check
archive contents, metadata, permissions, source provenance, and checksums.
Mock GitHub tests exercise recovery without creating releases in the repository.

For real container verification, run `build.sh all`. It checks linking, extracts
each archive, and exercises `--version`, `--help`, a confirmed file match (status
0), and a line/byte difference (status 1). The musl download is exercised again
in its native Alpine image. `artifacts.py` independently verifies archive
contents, permissions, normalized metadata, build provenance, and `SHA256SUMS`.
You can inspect and exercise the returned downloads on another machine:

```sh
cd dist
sha256sum -c SHA256SUMS
tar -tzvf paircomp-1.2.3-x86_64-unknown-linux-gnu.tar.gz
tar -xzf paircomp-1.2.3-x86_64-unknown-linux-gnu.tar.gz
paircomp-1.2.3-x86_64-unknown-linux-gnu/paircomp --version
paircomp-1.2.3-x86_64-unknown-linux-gnu/paircomp --help
```

After the workflow is merged into `main`, select **Linux downloads and draft
release → Run workflow** in GitHub Actions and choose `main`. Dispatch is
**build-only**: it returns three individually downloadable Actions artifacts
without invoking the draft job: the GNU `.tar.gz`, the musl `.tar.gz`, and
`SHA256SUMS`. Each downloads directly as its original file, without a ZIP
wrapper. Download all three into the same directory and run
`sha256sum -c SHA256SUMS`. All expire after 14 days. Dispatch may select a
development branch and build its crate version without a version tag or
membership in `main`.

Test actual GitHub draft creation with the **next real stable CLI release tag**
after the build-only run succeeds. Prerelease tags are rejected and are
unnecessary for testing.

## Tag and review a stable release

1. Set the `paircomp` CLI crate's intended stable `MAJOR.MINOR.PATCH` version,
   update the lockfile as needed, verify changes, and merge them into `main`.
   `paircomp-core` has an independent version and release process.
2. Fetch remote `main` and ensure the selected commit is in its history. Tag
   that exact commit as `paircomp-vMAJOR.MINOR.PATCH` and push the tag. For example,
   for a future version 1.2.3:

   ```sh
   git fetch origin main
   git tag -a paircomp-v1.2.3 origin/main -m 'Paircomp 1.2.3'
   git push origin paircomp-v1.2.3
   ```

3. The workflow rejects malformed/prerelease CLI tags, version mismatches, and
   commits outside remote `main`. `paircomp-core-*` tags do not trigger it.
   It builds the validated exact commit and rechecks `main` and the live remote
   tag before preparing the draft. Annotated and lightweight tags are supported.
4. After both native builds and archive verification succeed, the draft job
   uses only GitHub's temporary `GITHUB_TOKEN` with `contents: write`. Other jobs
   have `contents: read`; checkout credentials are not persisted. Actions are
   pinned to full commit SHAs. Runs for one ref are serialized, without
   cancelling an active run.
5. Open the draft URL from the draft job's log/summary. Check its editable
   summary, generated notes, compatibility details, source/pins, and exactly
   these workflow-owned assets: the two versioned archives and `SHA256SUMS`.
   Download them, verify checksums, and review the binaries before clicking
   **Publish release**. Publishing crates remains manual.

## Reruns and recovery

Reruns never update an existing draft's title or description. They compare the
size and SHA-256 of all existing workflow-owned assets before uploading anything;
identical assets are skipped and missing assets are uploaded. GitHub's asset
digest is used when available, otherwise the authenticated asset download is
hashed. Authorization is removed on redirects to another host. Unrelated assets
are left alone. The tag and draft status are checked again before each upload.

A partial upload or network failure leaves the release unpublished. Rerun the
failed job to complete missing uploads from the existing Actions artifacts;
rerunning all jobs rebuilds from the same source and pins and compares the new
assets. Rebuilt **Actions artifacts** can replace that run's earlier build
artifacts; **release assets** are always compared and never replaced. The bundle
job downloads the GNU and musl artifacts by the IDs returned by the build jobs,
verifies them, and uploads `SHA256SUMS` separately. It passes all three artifact
IDs to the draft job, which downloads those exact files. Edited notes survive
either rerun. The log prints the draft URL and expected asset list before
uploads, including when a later upload fails.

A differing asset, a GitHub `starter`/incomplete asset, a changed tag, or an
already published release makes the job fail. It never deletes or replaces
assets and never unpublishes a release. For a conflict, inspect the existing
draft and trusted Actions artifacts, identify why the bytes differ, and manually
remove only the incorrect workflow-owned asset from the **draft** before
rerunning. Keep any edited description. Do not delete good assets just to bypass
a mismatch. If a tag moved, resolve the source/tag discrepancy manually before
another attempt. Published releases require a new version and tag.

There is no atomic GitHub operation that locks a tag or draft across a check and
upload. Avoid editing tags, assets, or publishing while a release run is active.

## Maintain the pins

Update `pins.env` deliberately when changing the Rust toolchain or builders.
Resolve each official tag's **linux/amd64 child manifest digest**, rather than
the multi-platform index digest, with `podman manifest inspect` or a registry
inspection tool. Check the official image's source Dockerfile and Rust version.
The initial manifests came from
[docker-rust revision 4dc5209](https://github.com/rust-lang/docker-rust/tree/4dc5209f5e720a3d6b5eb336cabb69a424a822c1/stable).
Record the resolution date, preserve the Debian baseline unless intentionally
changing compatibility, and run both complete builds and build-only Actions
again. If the minimum Rust version changes, update workspace metadata and the
specification with the same intentional change.

Update action SHAs after reviewing the corresponding upstream release and its
license. The workflow defines each action pin on its first use with a YAML anchor;
later steps reuse that reference through aliases. Update the anchored reference,
its version comment, and the license link below together. CI adds no Rust
dependencies. The release tooling uses Python's standard library and these
MIT-licensed actions (licenses checked at the pinned revisions):

| Action | Purpose | License |
|---|---|---|
| `actions/checkout` v7.0.1 | Select the exact commit without persisted credentials | [MIT](https://github.com/actions/checkout/blob/3d3c42e5aac5ba805825da76410c181273ba90b1/LICENSE) |
| `actions/upload-artifact` v7.0.1 | Retain checked downloads between jobs and for manual testing | [MIT](https://github.com/actions/upload-artifact/blob/043fb46d1a93c77aae656e7c1c64a875d1fc6a0a/LICENSE) |
| `actions/download-artifact` v8.0.1 | Retrieve this run's checked downloads | [MIT](https://github.com/actions/download-artifact/blob/3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c/LICENSE) |

These versions use Node.js 24 and
[require Actions Runner 2.327.1 or newer](https://github.com/actions/checkout/blob/3d3c42e5aac5ba805825da76410c181273ba90b1/README.md#checkout-v5).
Uploads set
[`archive: false`](https://github.com/actions/upload-artifact/blob/043fb46d1a93c77aae656e7c1c64a875d1fc6a0a/README.md#upload-an-individual-file-unzipped),
which permits one file per artifact and uses its basename as the artifact name.
The explicit `name` also matches that basename so `overwrite: true` deletes the
correct artifact on reruns. Downloads use `skip-decompress: true` to preserve
each file in `dist`.
Download-artifact v8
[fails on an artifact digest mismatch by default](https://github.com/actions/download-artifact/blob/3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c/README.md#v8---whats-new).
