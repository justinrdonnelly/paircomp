#!/bin/sh
# Always executed inside the pinned GNU image, including for musl downloads.
set -eu
# shellcheck source=scripts/release/pins.env
. /source/scripts/release/pins.env
export LC_ALL=C
export TZ=UTC
test "$(getconf GNU_LIBC_VERSION)" = "glibc $GLIBC_BASELINE"
readelf -h /build/paircomp > /build/elf-header.txt
readelf -l /build/paircomp > /build/elf-program.txt
readelf -d /build/paircomp > /build/elf-dynamic.txt
grep -q 'Machine:.*Advanced Micro Devices X86-64' /build/elf-header.txt
needed=$(sed -n 's/.*(NEEDED).*\[\(.*\)\].*/\1/p' /build/elf-dynamic.txt | sort)
minimum_glibc=none
case "$TARGET" in
    x86_64-unknown-linux-gnu)
        grep -q 'Requesting program interpreter: /lib64/ld-linux-x86-64.so.2' /build/elf-program.txt
        readelf --version-info /build/paircomp > /build/elf-versions.txt
        required_glibc=$(grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' /build/elf-versions.txt | sed 's/GLIBC_//' | sort -Vu | tail -n 1)
        test -n "$required_glibc"
        printf 'Detected glibc symbol requirement: %s; supported baseline: %s\n' "$required_glibc" "$GLIBC_BASELINE"
        test "$(printf '%s\n%s\n' "$required_glibc" "$GLIBC_BASELINE" | sort -V | tail -n 1)" = "$GLIBC_BASELINE"
        for library in $needed; do
            case "$library" in libc.so.6|libm.so.6|libgcc_s.so.1|ld-linux-x86-64.so.2) ;;
                *) echo "Unexpected shared library: $library" >&2; exit 1 ;;
            esac
        done
        minimum_glibc=$GLIBC_BASELINE
        ;;
    x86_64-unknown-linux-musl)
        if grep -q 'INTERP' /build/elf-program.txt; then
            echo 'musl executable has a dynamic loader' >&2; exit 1
        fi
        test -z "$needed"
        ;;
    *) exit 2 ;;
esac

name="paircomp-$PAIRCOMP_VERSION-$TARGET"
mkdir -p "/build/package/$name"
cp /build/paircomp "/build/package/$name/paircomp"
cp /source/README.md /source/COPYING "/build/package/$name/"
{
    printf 'source_commit=%s\n' "$SOURCE_COMMIT"
    printf 'rust_toolchain=%s\n' "$RUST_VERSION"
    printf 'builder_image=%s\n' "$BUILDER_IMAGE"
    printf 'packaging_image=%s\n' "$GNU_IMAGE"
    printf 'target=%s\n' "$TARGET"
    printf 'lockfile_sha256=%s\n' "$LOCKFILE_SHA256"
    printf 'minimum_glibc=%s\n' "$minimum_glibc"
    printf 'shared_libraries=%s\n' "$(printf '%s' "$needed" | tr '\n' ' ')"
    printf 'cpu=x86-64\n'
    printf 'source_date_epoch=%s\n' "$SOURCE_DATE_EPOCH"
} > "/build/package/$name/BUILD-INFO.txt"
chmod 755 "/build/package/$name" "/build/package/$name/paircomp"
chmod 644 "/build/package/$name/README.md" "/build/package/$name/COPYING" "/build/package/$name/BUILD-INFO.txt"
cd /build/package
# Write tar first so failure cannot be hidden by a successful gzip pipeline.
tar --sort=name --format=gnu --mtime="@$SOURCE_DATE_EPOCH" \
    --owner=0 --group=0 --numeric-owner -cf /build/download.tar "$name"
gzip -n -9 -c /build/download.tar > "/output/$name.tar.gz"
chmod 644 "/output/$name.tar.gz"
mkdir /build/extracted
tar -xzf "/output/$name.tar.gz" -C /build/extracted
sh /source/scripts/release/smoke.sh "/build/extracted/$name/paircomp" "$PAIRCOMP_VERSION"
