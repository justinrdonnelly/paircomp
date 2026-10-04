#!/bin/sh
set -eu
# shellcheck source=scripts/release/pins.env
. /source/scripts/release/pins.env
export RUSTUP_TOOLCHAIN="$RUST_VERSION"
export CARGO_HOME=/build/cargo
export CARGO_INCREMENTAL=0
export RUSTFLAGS='-C target-cpu=x86-64 --remap-path-prefix=/source=/paircomp/source --remap-path-prefix=/build=/paircomp/build --remap-path-prefix=/usr/local/rustup=/paircomp/toolchain'
export CFLAGS='-march=x86-64 -mtune=generic -ffile-prefix-map=/source=/paircomp/source -ffile-prefix-map=/build=/paircomp/build'
export LC_ALL=C
export TZ=UTC

test "$(rustc --version | cut -d ' ' -f 2)" = "$RUST_VERSION"
test "$(rustc -vV | sed -n 's/^host: //p')" = "$TARGET"
# This also checks writable mount access and source visibility under keep-id.
test -r /source/Cargo.lock
mkdir -p /build/cargo
export CARGO_TARGET_DIR=/build/checks
case "$TARGET" in
    x86_64-unknown-linux-gnu)
        # The official images use rustup's minimal profile. Install only the
        # checks' components for this exact toolchain, in the disposable layer.
        rustup component add rustfmt clippy
        cargo fmt --all -- --check
        cargo clippy --locked --workspace --all-targets --target "$TARGET" -- -D warnings
        ;;
    x86_64-unknown-linux-musl) ;;
    *) echo 'Unsupported native target' >&2; exit 2 ;;
esac
cargo test --locked --workspace --target "$TARGET"
# Release compilation starts with fresh output, separate from checks/tests.
export CARGO_TARGET_DIR=/build/release
test ! -e "$CARGO_TARGET_DIR"
cargo build --locked --release --package paircomp --target "$TARGET"
cp "/build/release/$TARGET/release/paircomp" /build/paircomp
