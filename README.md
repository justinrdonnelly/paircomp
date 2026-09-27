# Paircomp

Paircomp helps a person compare two copies of a file on isolated systems. Each
system runs its own instance and opens only its local file. The instances display
fingerprints for the person to compare; they do not communicate with each other.
If the whole-file fingerprints differ, Paircomp guides both instances to the
first differing line. The [MVP specification](paircomp_mvp_implementation_spec.md)
defines the comparison protocol and line semantics.

## Build and run

The minimum supported Rust version is **1.98.1**.

Build from the workspace root with Rust and Cargo:

```sh
cargo build --release -p paircomp
```

Run the resulting binary on each system against the corresponding local file:

```sh
target/release/paircomp FILE
```

For a development run, use `cargo run -p paircomp -- FILE`. The binary is
named `paircomp`; it also supports `--help` and `--version`. `FILE` must be a
readable regular file. File contents may contain arbitrary bytes; UTF-8 is not
required.

## Compare two copies

1. Start `paircomp FILE` separately on both systems. Compare the displayed
   whole-file fingerprints. If they match, answer `y` on both instances and the
   comparison is complete.
2. If they differ, answer `n` on both. Enter the **other** instance's displayed
   line count on each system, even when the counts are equal.
3. Both instances will request a fingerprint through the same line number.
   Compare those fingerprints and give the same `y` or `n` answer to each.
   Repeat until both report the first differing line. A shorter file may report
   that it ends before that line.

Answers accept `y`/`yes` and `n`/`no`, regardless of case or surrounding
whitespace. An empty answer at a `[y/N]` prompt means `no`. A line count must
be a nonnegative decimal integer; it has no default. Invalid input or input
ending before the comparison finishes aborts the session.

For example, suppose the first system has the bytes `a\nb\n` and the second has
`a\nb\nc\n` (`\n` denotes an LF byte):

| Stage | First system | Second system |
|---|---|---|
| Whole-file display | `Lines: 2`; fingerprint differs | `Lines: 3`; fingerprint differs |
| Whole-file answer | `n` | `n` |
| Other line count entered | `3` | `2` |
| `Compare through line 2` | Fingerprint of `a\nb\n`; answer `y` | Same fingerprint; answer `y` |
| Result | `First divergence: line 3`; local file ends before line 3 | `First divergence: line 3` |

The first system fingerprints all available bytes when asked for a line beyond
its EOF. Each instance reports exit status 1 after locating this difference.
For identical copies, the whole-file fingerprints match, both instances answer
`y`, and both exit with status 0 without starting a line search.

Keep **both files unchanged** from the first fingerprint until the session
ends. After editing or replacing either file, restart Paircomp on both systems;
results from an earlier session no longer apply. Accurate entry of the other
line count and matching answers on both instances are also necessary for a
valid result.

## Exit status and fingerprint limits

| Status | Meaning |
|---:|---|
| 0 | The user confirmed a whole-file fingerprint match, or `--help`/`--version` completed. |
| 1 | The first differing line was reported. |
| 2 | Invocation, file I/O, or interaction failed; restart the comparison. |

Paircomp compares raw bytes using BLAKE3. It does not normalize whitespace,
newlines, or encoding. Lines end at LF (`0x0A`); a CR (`0x0D`) remains part of
the line. A final LF does not create an extra line. The CLI displays the full
256-bit digest as 64 hexadecimal characters for both whole files and prefixes;
it does not truncate fingerprints. Under the usual ideal-hash model, a
coincidental match for two distinct byte sequences has probability about
1 in 2^256 per comparison. Matching fingerprints are therefore strong evidence,
not an absolute proof of byte equality. The comparison also assumes the files
remain stable and that both people provide consistent answers.

## Scope and licenses

The MVP localizes a difference to a **line**. It does not locate a byte or
character within that line. Within-line localization is a future enhancement;
the core already keeps line results as typed data and owns byte-level file
operations for that purpose. The MVP has no networking, file transfer,
automatic repair, directory comparison, or GUI.

`paircomp-core` owns file inspection, raw-byte hashing, and line-search state.
`paircomp` handles arguments and the interactive prompts. Paircomp's own
code is licensed [MPL-2.0](COPYING). Its direct third-party dependencies are:

| Dependency (locked version) | Used by | Purpose | License |
|---|---|---|---|
| [`blake3` 1.8.7](https://github.com/BLAKE3-team/BLAKE3) | `paircomp-core` | Full-file and line-prefix fingerprints | [CC0-1.0 OR Apache-2.0 OR Apache-2.0 WITH LLVM-exception](https://docs.rs/crate/blake3/1.8.7/source/Cargo.toml.orig) |
| [`clap` 4.6.7](https://github.com/clap-rs/clap) with `derive` | `paircomp` | Parse the file argument and provide help/version output | [MIT OR Apache-2.0](https://docs.rs/crate/clap/4.6.7/source/Cargo.toml.orig) |

The exact dependency versions, including transitive packages, are recorded in
[`Cargo.lock`](Cargo.lock). The license expressions above are from the locked
crates' manifests.
