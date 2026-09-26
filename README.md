# Paircomp

Paircomp compares files on isolated systems by showing fingerprints that a person can compare. When the whole-file fingerprints differ, its interactive CLI guides both sides to the first differing line. The [MVP specification](paircomp_mvp_implementation_spec.md) describes the intended behavior.

The CLI displays the full 256-bit BLAKE3 fingerprint as 64 hexadecimal characters for whole files and line prefixes. It does not truncate fingerprints. For two distinct inputs under the usual ideal-hash model, the chance of a coincidental full-digest match is approximately 1 in 2^256 per comparison. This is a statistical limit, so matching displayed fingerprints are not absolute proof of identical bytes. Keep both files unchanged during a comparison session and restart after editing either file.

`paircomp-cli` depends on the local `paircomp-core` crate, which owns file operations and comparison state. Both crates use the existing [MPL-2.0 license](COPYING).

| Third-party dependency | Used by | Purpose | License |
|---|---|---|---|
| [`blake3`](https://github.com/BLAKE3-team/BLAKE3) | `paircomp-core` | Full-file and line-prefix fingerprints | [CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception](https://docs.rs/crate/blake3/1.8.7/source/Cargo.toml.orig) |
| [`clap`](https://github.com/clap-rs/clap) with `derive` | `paircomp-cli` | Parse the file argument and provide help/version output | [MIT OR Apache-2.0](https://docs.rs/crate/clap/4.6.7/source/Cargo.toml.orig) |
