# Paircomp

Paircomp is planned as a tool for manually comparing files on isolated systems and locating their first differing line. The [MVP specification](paircomp_mvp_implementation_spec.md) describes the intended behavior.

The current foundation provides a buildable Cargo workspace. The `paircomp` binary reports that the comparison workflow is not implemented yet. File inspection, hashing, and the interactive workflow belong to later milestones.

The workspace has no third-party dependencies yet. `paircomp-cli` depends on the local `paircomp-core` crate, which will own file operations and comparison state. Both crates use the existing [MPL-2.0 license](COPYING). Dependencies will be documented here as they are added.
