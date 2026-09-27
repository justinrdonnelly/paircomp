# paircomp-core

`paircomp-core` is the reusable library behind the Paircomp command-line tool.
It inspects regular files as raw bytes, computes full-file and line-prefix BLAKE3
fingerprints, and provides the search state used to locate the first differing
line between copies on isolated systems. It does not handle prompts or terminal
output.

The minimum supported Rust version is **1.98.1**.

The crate root (`lib.rs`) defines shared types and errors and exports the public
API. `file.rs` implements file inspection and through-line hashing; `search.rs`
contains search state and its private bisection bounds.

The public API includes `inspect_file`, `fingerprint_file`,
`fingerprint_through_line`, and `LineSearch`. Lines end at LF; a final LF does not
create an extra line. A prefix request beyond the local end of file fingerprints
all available bytes. The caller compares fingerprints from the two systems and
feeds each match result to the search state.

## API documentation

Build and open the rustdoc reference, including API contracts and examples:

```sh
cargo doc -p paircomp-core --no-deps --open
```

Rust examples are checked by
`cargo test -p paircomp-core --doc` (also included in `cargo test --workspace`);
examples marked `no_run` are compiled without being executed.

Licensed under [MPL-2.0](COPYING).
