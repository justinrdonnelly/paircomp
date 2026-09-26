# Paircomp MVP Implementation Specification

*Implementation brief for an AI coding agent*

**Status:** MVP / v0.1 design
**License:** MPL-2.0 for Paircomp code, subject to compatible dependency licenses

## 1. Objective

Implement Paircomp, a small Rust utility that helps a human determine whether two isolated copies of a text file are identical and, when they are not, efficiently locate the first point of divergence.

The two copies are never available to the same Paircomp process. The user runs Paircomp independently on each system and manually compares short fingerprints displayed by the two instances.

The MVP must prioritize a small, understandable codebase and a clean architectural boundary between the comparison/hash library and the command-line user interface. The library must not depend on terminal interaction so that a future GUI or other frontend can use it without redesigning the core.

## 2. MVP user experience

The normal invocation is:

```console
$ paircomp FILE
```

The same command is run against the corresponding file on both isolated systems. Each instance computes a whole-file fingerprint and basic metadata. The human compares the displayed values.

1. If the whole-file fingerprints match, report that the files match and exit successfully.
2. If they differ, ask for the line count displayed by the other instance, then guide the user through a deterministic prefix-based bisection to locate the first differing line.
3. At each step, both instances must independently request comparison through the same line number and display a human-comparable fingerprint. A prefix that extends beyond the local EOF includes all available bytes.
4. The user tells each instance whether the two displayed fingerprints match.
5. When the search has isolated the first differing line, report its line number. If that line is beyond the local EOF, explain that the local file has no such line.

Within-line byte/character localization is out of scope for the MVP. It is a known future enhancement; preserve the extension points described in section 4.2 without implementing it in v0.1.

## 3. Architecture: strict UI/library separation

Use a Cargo workspace or equivalent multi-crate layout. Prefer two crates from the beginning rather than putting the core logic in the binary crate.

```text
paircomp/
├── Cargo.toml
├── COPYING
├── README.md
├── paircomp-core/
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
└── paircomp-cli/
    ├── Cargo.toml
    └── src/
        └── main.rs
```

### 3.1 `paircomp-core`

A reusable Rust library. It owns file inspection, fingerprint generation, file-prefix calculations, and search-state logic.

It must contain:

- no `clap` dependency,
- no prompts,
- no terminal formatting,
- no reads from stdin,
- no direct UI output.

It should return typed data/results and errors to its caller.

### 3.2 `paircomp-cli`

A thin CLI frontend. It owns:

- `clap` argument parsing,
- terminal output,
- stdin prompts,
- interpreting the user's match/no-match answers,
- choosing what library operation to call next.

Keep business logic out of this crate.

## 4. Core library API

Do not treat the exact API below as immutable, but preserve these responsibilities and keep the API frontend-neutral. Prefer domain types over strings intended for display.

```rust
pub struct FileInfo {
    pub byte_len: u64,
    pub line_count: u64,
    pub fingerprint: Fingerprint,
}

pub struct Fingerprint(/* digest bytes or an opaque representation */);

pub fn inspect_file(path: &Path) -> Result<FileInfo, Error>;

pub fn fingerprint_file(path: &Path) -> Result<Fingerprint, Error>;

/// Fingerprint the file prefix from byte 0 through and including `line`.
/// Line numbers are 1-based; line 0 denotes the empty prefix.
/// Include the terminating LF, if present. Requests beyond EOF hash the whole file.
pub fn fingerprint_through_line(
    path: &Path,
    line: u64,
) -> Result<Fingerprint, Error>;
```

Here, **“through line N” means the file prefix beginning at byte 0 and ending at the end of line N, or at EOF if the file has fewer than N lines**. It does not mean hashing line N by itself. Line 0 is the special case for an empty prefix. Section 6.1 defines line boundaries precisely.

For example, for:

```text
line 1
line 2
line 3
line 4
```

`fingerprint_through_line(path, 2)` hashes the raw bytes corresponding to the first two lines.

The core may later expose an in-memory/indexed representation so repeated prefix checks do not repeatedly scan the file from byte zero. Do not prematurely optimize this for v0.1. Correctness and a clear API are more important.

### 4.1 Search state

Represent bisection as library state rather than embedding midpoint arithmetic in the CLI. A frontend should be able to ask the core what comparison is needed next, report whether that comparison matched, and receive the next state/result.

Illustrative API:

```rust
pub struct LineSearch { /* bounds/state */ }

pub enum SearchStep {
    CompareThroughLine { line: u64 },
    DifferenceAtLine { line: u64 },
}

impl LineSearch {
    /// Construct only after the user reports a whole-file mismatch.
    pub fn new(local_line_count: u64, other_line_count: u64) -> Result<Self, Error>;
    pub fn current_step(&self) -> SearchStep;
    pub fn record_result(&mut self, matched: bool) -> Result<(), Error>;
}
```

This exact API is not required, but preserve the separation of responsibilities. A future GUI should be able to drive exactly the same search state machine without emulating CLI prompts.

Construction must reject a reported whole-file mismatch when both line counts are zero. Calling `current_step` must not advance the search; `record_result` applies an answer to the current comparison. Recording an answer after the search has completed must return an error. The core owns the bounds and transitions specified in section 6.

### 4.2 Known future enhancement: within-line localization

A future version may continue from `DifferenceAtLine` to locate the first differing byte within that line, with character-oriented presentation only where the encoding and character semantics are explicitly defined.

Design for this extension by keeping:

- the localized line number available as typed data, independent of CLI text;
- byte reading, line-boundary discovery, and prefix hashing in the core, so a later operation can derive the local byte range for the reported line;
- line-search state separate from prompting and result presentation, so a frontend can later start a separate within-line search using the line result.

The later extension must account for unequal line lengths and a line that is absent because the local file has ended. Do not assume that byte offsets and character positions are interchangeable. No within-line search implementation, unused public API, or general search framework is required for v0.1.

## 5. Fingerprinting

Use BLAKE3 for the MVP. Use the maintained Rust `blake3` crate rather than implementing a hash function.

Feed raw file bytes into the hash. Do not normalize:

- whitespace,
- newline style,
- encoding,
- Unicode,
- or any other content.

Paircomp is establishing exact file equality.

Internally retain the full digest. The CLI may display a shortened, human-friendly fingerprint for intermediate comparisons. The truncation length must provide a deliberately chosen and documented collision probability.

Do not silently treat a very short display token as cryptographic proof of equality. For the initial whole-file comparison, displaying the full digest is acceptable and may be preferable for v0.1.

## 6. Line semantics and search algorithm

The search target is the first point where the two files cease to have identical prefixes.

Prefix comparison is essential: it allows the method to locate the first divergence even when one copy contains an added or absent line relative to the other.

Both instances must use the following protocol. Correct localization assumes unchanged files, accurate entry of the other count, consistent match/no-match answers, and no fingerprint collision.

1. Compute and display the whole-file fingerprint and local line count. Ask the user whether the whole-file fingerprints match. If they do, report a match and stop.
2. On a mismatch, always ask for the line count displayed by the other instance, even when the counts are equal. Accept a nonnegative integer representable as `u64`. Each instance uses its own count as `local_line_count` and the entered count as `other_line_count`.
3. Initialize inclusive candidate bounds `low = 1` and `high = max(local_line_count, other_line_count)`. If `high == 0`, report an inconsistent answer and abort: both files are empty and cannot differ.
4. While `low < high`, choose `mid = low + (high - low) / 2` using integer division, and request comparison through line `mid`.
5. Fingerprint bytes from the beginning of the local file through that line, including its terminator if present. If the local file ends before that line, hash the whole local file; do not fail, pad the input, or add an EOF marker to the hash.
6. If the user reports a match, set `low = mid + 1`; otherwise set `high = mid`. Repeat from step 4.
7. When `low == high`, return `DifferenceAtLine { line: low }` without requesting another comparison. The initial whole-file mismatch establishes the upper bound, including when it is line 1.
8. Report the line number. If it exceeds the local line count, also report that the local file ends before this line. After the user repairs the file manually, Paircomp is simply rerun. Do not attempt synchronization or patching.

Using the maximum of the two counts makes the initial bounds identical on both systems regardless of which file is local. Beyond-EOF prefix behavior ensures that comparison at the initial upper bound would hash the entire file on each system. Matching prefixes exclude all lines through the midpoint; mismatching prefixes retain the midpoint as a candidate.

### 6.1 Newlines and edge cases

- Lines are numbered starting at 1 and are defined by raw LF bytes (`0x0A`), regardless of encoding.
- Each LF terminates a line and belongs to that line's fingerprinted bytes. CR (`0x0D`) is ordinary content, including immediately before LF; a lone CR does not terminate a line.
- A nonempty suffix after the last LF counts as one unterminated line. A trailing LF does not create an additional empty line.
- The line count is the number of LF bytes, plus one if the file is nonempty and does not end in LF. An empty file has zero lines.
- A request through line 0 hashes the empty byte sequence. A request through any line beyond EOF hashes the entire available file, including for an empty file.
- Treat LF versus CRLF and the presence/absence of a final newline as real differences; do not normalize.
- File-level hashing and line localization must work for arbitrary bytes without UTF-8 decoding. The MVP reports line numbers and does not need to display line contents. Any future content/character presentation must handle non-UTF-8 input explicitly rather than silently converting it lossily.

Examples use Rust byte-string notation:

| File bytes | Line count |
|---|---:|
| `b""` | 0 |
| `b"\n"` | 1 |
| `b"a"` | 1 |
| `b"a\n"` | 1 |
| `b"a\n\n"` | 2 |
| `b"a\r\n"` | 1 |
| `b"a\rb"` | 1 |

For `b"a\n"` versus `b"a\nb\n"`, the first divergence is line 2; the shorter instance reports that line 2 is beyond its EOF. For `b"a"` versus `b"a\n"`, it is line 1. For an empty file versus any nonempty file, it is line 1.

### 6.2 File stability during a session

Both files must remain unchanged from initial inspection until the comparison session ends. Tell the user to keep them unchanged and restart Paircomp after any edit or replacement. The MVP may reread the file for each prefix and does not need to implement snapshots, locking, or reliable mutation detection. Results are only valid under this stability requirement; document it in the README as well.

## 7. CLI responsibilities

Use `clap` with its derive API. The MVP surface should remain intentionally small:

```console
paircomp FILE
paircomp --help
paircomp --version
```

The CLI should:

- validate that `FILE` can be opened as a regular input file,
- call `paircomp-core`,
- format returned metadata/fingerprints,
- prompt for match/no-match answers,
- obtain the other instance's line count after a whole-file mismatch,
- drive the search state.

It should not implement hashing or midpoint/search calculations itself.

Use `PathBuf`/`OsString`-compatible argument handling so Unix paths are not unnecessarily restricted to UTF-8.

### 7.1 Prompt input

- Match prompts accept `y`/`yes` and `n`/`no`, case-insensitively, after trimming surrounding whitespace.
- The displayed `[y/N]` default means that a submitted blank or whitespace-only answer is `no`.
- The other-line-count prompt has no default. Require decimal digits representing a `u64`, after trimming surrounding whitespace; zero is valid.
- Invalid answers or counts produce a diagnostic and terminate with status 2. The user must restart both instances to begin a new comparison session.
- Stdin EOF is an aborted interaction and terminates with status 2. It must never be interpreted as a blank answer or a sequence of `no` answers.

## 8. Example interaction

```text
$ paircomp file.txt

File: file.txt
Lines: 1247
Size: 38291 bytes
Fingerprint: <fingerprint>

Keep both files unchanged during this session. Restart after editing either file.

Does this fingerprint match the other copy? [y/N] n
Line count displayed by the other copy: 1247

Compare through line 624:
Fingerprint: <fingerprint>
Does this fingerprint match? [y/N] y

Compare through line 936:
Fingerprint: <fingerprint>
Does this fingerprint match? [y/N] n

...

First divergence: line 737

Inspect/correct the corresponding files, then run paircomp again.
```

## 9. Error handling

- Return library errors; do not print from `paircomp-core`.
- CLI diagnostics go to stderr; normal interactive/output information goes to stdout.
- Exit with status 0 when the user confirms a whole-file match, or for successful `--help`/`--version` output.
- Exit with status 1 after successfully localizing and reporting a difference.
- Exit with status 2 for invocation errors, I/O errors, or aborted/invalid interaction, including a reported mismatch between two empty files. A detected difference is a completed comparison, distinct from these errors.
- Do not panic for expected user/file errors.
- Keep the error model simple for v0.1; avoid adding a large error-handling dependency unless it provides clear value.

## 10. Dependencies and licensing

Paircomp source should be licensed MPL-2.0.

Dependencies may use compatible permissive licenses. Before adding any dependency, verify its current license and record why it is needed. Keep dependencies deliberately small.

| Dependency | Crate | Purpose |
|---|---|---|
| `blake3` | `paircomp-core` | BLAKE3 fingerprinting |
| `clap` with `derive` | `paircomp-cli` | CLI parsing/help/version |

## 11. Testing requirements

Put most tests in `paircomp-core`. The core must be testable without spawning a terminal process.

Required cases:

- Identical files produce identical fingerprints.
- A one-byte change changes the fingerprint.
- Fingerprinting through line N includes exactly the intended file-prefix bytes.
- Line counts match every example in section 6.1; line 0 hashes the empty sequence and requests beyond EOF hash the whole file.
- Difference on the first line is found.
- Difference on the last line is found.
- Files with differing line counts due to an added/absent line are localized to the first divergence.
- Different final-newline state is detected.
- Empty and one-line files behave correctly.
- CRLF versus LF is detected as a difference.
- Invalid UTF-8 bytes can be hashed and localized to the correct line without decoding errors.
- Search-state midpoint progression terminates and returns the earliest divergent line for representative cases.
- Construction rejects a reported mismatch when both line counts are zero, repeated `current_step` calls preserve state, and recording an answer after completion returns an error.

Use multiple fixtures for the added/absent-line case when useful—for example, a differing line near the beginning, middle, and end—but treat these as positioning cases rather than separate “insertion” and “deletion” semantics.

Include paired-state tests that simulate both isolated instances using two fixtures. Construct one search with counts `(a, b)` and the other with `(b, a)`. At each step, assert that both request the same line, compute each fixture's actual prefix fingerprint, and feed the same equality result into both states. Assert that both terminate at the expected first divergent line. Cover equal and unequal line counts, an appended line, empty versus nonempty input, and differing final-newline state. For identical files, verify that the whole-file comparison completes without creating a line search.

CLI tests should focus on argument parsing and a small number of end-to-end interactions, including acquisition of the other count, the beyond-EOF message, and exit statuses 0/1/2. Verify that a submitted blank match answer uses the `no` default, while stdin EOF and invalid input abort with status 2; a blank line-count answer is invalid. Do not duplicate core algorithm tests through the CLI.

## 12. Explicit non-goals for v0.1

- No networking or direct communication between Paircomp instances.
- No automatic file transfer.
- No patch/delta generation or application.
- No rsync-style synchronization.
- No attempt to display a conventional two-file diff.
- No within-line byte/character localization; this is a known future enhancement with the design requirements in section 4.2.
- No directory-tree comparison.
- No configuration file.
- No selectable hash algorithms unless a concrete need appears.
- No GUI, but the architecture must permit one later.
- No elaborate plugin/framework architecture.
- No premature optimization of repeated file scans unless profiling shows it matters.

## 13. Implementation plan and checklist

Work through these milestones in order, marking items complete after the corresponding work and verification are finished. Each milestone may involve several small, focused commits. Choose commit boundaries around coherent changes, include their related tests and documentation, and keep intermediate commits buildable. Follow the commit-message, identity, attribution, and verification instructions in [AGENTS.md](AGENTS.md).

### 13.1 Foundation

- [ ] Create the Cargo workspace with `paircomp-core` and `paircomp-cli`, with the CLI depending on the core and exposing a binary named `paircomp`.
- [ ] Configure package/version/license metadata and use the existing `COPYING` license file.
- [ ] Establish a buildable crate structure that preserves the architecture in section 3. Add dependencies only as they are needed, verifying and documenting their licenses and purpose at that time.

Completion criteria: the workspace builds, the core can be built independently, and dependency direction and crate responsibilities match section 3.

### 13.2 File inspection and prefix hashing

- [ ] Implement `FileInfo`, full-file BLAKE3 fingerprinting, and typed library errors.
- [ ] Implement line counting and fingerprinting through line N with the exact raw-byte, terminator, line-0, and beyond-EOF semantics in section 6.1.
- [ ] Add the hashing and line-semantics tests from section 11, including empty files, final-newline differences, CRLF, and invalid UTF-8.

Completion criteria: tests verify the expected metadata and exact bytes included in each prefix; expected file errors are returned without panics or core UI output.

### 13.3 Deterministic search

- [ ] Implement the core search state and exact bounds/transitions from sections 4.1 and 6, including unequal line counts and invalid state operations.
- [ ] Add the paired-instance tests from section 11, using actual prefix comparisons to drive both states.
- [ ] Preserve the typed line result and core byte operations needed for the future extension in section 4.2, without implementing within-line localization or unused APIs.

Completion criteria: both simulated instances request the same line at every step, terminate at the expected first divergence, and handle all specified search edge cases without CLI involvement.

### 13.4 CLI workflow

- [ ] Implement the `clap` interface, regular-file validation, path handling, metadata output, and fingerprint formatting. Choose and document the displayed fingerprint length and any truncation's collision probability as required by section 5.
- [ ] Implement whole-file comparison, collection of the other count, and prompts that drive the core search state. Include the file-stability notice, beyond-EOF result message, input rules, diagnostics, and exit statuses from sections 6 through 9.
- [ ] Add focused argument-parsing and interaction tests from section 11, covering successful matches, localized differences, and invalid/aborted interactions.

Completion criteria: `paircomp FILE`, `--help`, and `--version` work as specified; interaction tests verify exit statuses 0/1/2; the CLI contains no hashing or search calculations.

### 13.5 Documentation and MVP verification

- [ ] Complete the README with build/run instructions, paired-instance usage, a comparison example, exit statuses, fingerprint limitations, and the requirement to restart after editing either file.
- [ ] Complete dependency/license notices and document the MVP scope, including within-line localization as a future enhancement.
- [ ] Run the verification checks in `AGENTS.md` and review the implementation against every requirement in section 11 and the definition of done in section 14. Exercise the workflow with two separate instances, each opening only its own file, for both matching and differing fixtures, including unequal line counts.

Completion criteria: required checks pass, documented usage matches the implemented behavior, and the definition of done is satisfied. Record verification results and any remaining limitations in the handoff; leave incomplete checklist items unchecked.

## 14. Definition of done for the MVP

On two systems containing corresponding files, a user can run `paircomp FILE` on each.

If the files are identical, the user can establish that immediately by comparing the whole-file fingerprints.

If they differ, entering the other instance's line count on each system and following the same match/no-match answers deterministically guides the user to the first differing line, including when the counts differ or one file is empty. Both files must remain unchanged during the session.

The core functionality is exposed by `paircomp-core` with no dependency on `clap`, stdin/stdout, or terminal UI, and the CLI is a thin frontend over that API.

The MVP ends at line-level localization. Its typed result and separation of byte operations, search state, and UI permit later within-line localization without implementing that enhancement now.
