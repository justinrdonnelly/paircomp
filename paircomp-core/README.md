# paircomp-core

`paircomp-core` is the reusable library behind the Paircomp command-line tool.
It inspects regular files as raw bytes, computes BLAKE3 prefix fingerprints,
and provides separate line and byte searches to locate the first difference
between copies on isolated systems. It also maps byte positions to UTF-8
code-point positions. It does not handle prompts or terminal output.

The minimum supported Rust version is **1.98.1**.

The crate root (`lib.rs`) defines shared types and errors and exports the public
API. `file.rs` implements file inspection and through-line hashing; `search.rs`
contains both search states and their private bisection bounds. `within_line.rs`
implements line-local inspection, prefix hashing, and UTF-8 position mapping.

`Fingerprint` retains the complete 256-bit BLAKE3 digest, and equality compares
all 32 bytes. Frontends choose how to display it using `Fingerprint::as_bytes`.
The Paircomp CLI shows the first 8 hexadecimal characters by default, or all 64
with `--full-digest`; this display choice does not change the library's digests.

The public API includes `inspect_file`, `fingerprint_file`,
`fingerprint_through_line`, and `LineSearch`. Lines end at LF; a final LF does not
create an extra line. A prefix request beyond the local end of file fingerprints
all available bytes. The caller compares fingerprints from the two systems and
feeds each match result to the search state.

After locating a differing line, use `inspect_line` to obtain its byte length
(including LF), or `None` if it is absent. Exchange lengths, treating absence as
zero, and construct `ByteSearch`. Feed comparisons from `fingerprint_line_prefix`
into that search. These fingerprints start at the selected line's beginning;
requests beyond its end hash the whole line without including the next line.
Zero bytes or an absent line hashes the empty sequence. Both zero lengths are
inconsistent with a differing line and are rejected.

`utf8_character_position` maps a 1-based byte position to a 1-based Unicode
code-point position when the entire line is valid UTF-8. It returns `None` for
invalid UTF-8 or an absent line. All bytes of a multibyte character map to that
character, and one byte past an existing line maps to its next character
position. CR and LF count separately; code points need not equal visual columns.
Byte zero or positions farther than one past the line are invalid. All three
line-local file operations reject line zero.

File operations use buffered reads and reopen the path on each call. Both files
must remain unchanged from inspection until both searches and position mapping
are complete. After editing or replacing either file, restart both instances.

## API documentation

Build and open the rustdoc reference, including API contracts and examples:

```sh
cargo doc -p paircomp-core --no-deps --open
```

Rust examples are checked by
`cargo test -p paircomp-core --doc` (also included in `cargo test --workspace`);
examples marked `no_run` are compiled without being executed.

Licensed under [MPL-2.0](COPYING).
