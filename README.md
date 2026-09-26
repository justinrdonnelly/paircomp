# Paircomp

Paircomp is planned as a tool for manually comparing files on isolated systems and locating their first differing line. The [MVP specification](paircomp_mvp_implementation_spec.md) describes the intended behavior.

The current workspace has file inspection and prefix hashing in `paircomp-core`. The `paircomp` binary still reports that the comparison workflow is not implemented yet; deterministic search and the interactive workflow belong to later milestones.

`paircomp-cli` depends on the local `paircomp-core` crate, which owns file operations and will own comparison state. Both crates use the existing [MPL-2.0 license](COPYING).

| Third-party dependency | Used by | Purpose | License |
|---|---|---|---|
| [`blake3`](https://github.com/BLAKE3-team/BLAKE3) | `paircomp-core` | Full-file and line-prefix fingerprints | [CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception](https://docs.rs/crate/blake3/1.8.7/source/Cargo.toml.orig) |
