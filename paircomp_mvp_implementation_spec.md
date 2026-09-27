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
6. Ask whether to continue within that line, defaulting to yes. Both instances must choose the same answer. If they decline, finish with the line result.
7. When continuing, display the selected line's byte count, obtain the other instance's byte count, and bisect raw-byte prefixes of that line to locate the first differing byte. Report its 1-based byte position and, when the local line is valid UTF-8, its 1-based Unicode code-point position. Explain when the local byte is absent.

Optional within-line localization is included in v0.1. Hashing and localization remain byte based; character positions are supplementary annotations, with the exact semantics in sections 4.2 and 6.3.

## 3. Architecture: strict UI/library separation

Use a Cargo workspace or equivalent multi-crate layout. Prefer two crates from the beginning rather than putting the core logic in the binary crate.

The minimum supported Rust version for both crates is 1.98.1.

```text
paircomp/
├── Cargo.toml
├── COPYING
├── README.md
├── paircomp-core/
│   ├── Cargo.toml
│   └── src/
│       ├── lib.rs
│       ├── file.rs
│       └── search.rs
└── paircomp-cli/
    ├── Cargo.toml
    └── src/
        └── main.rs
```

### 3.1 `paircomp-core`

A reusable Rust library. It owns file inspection, fingerprint generation, file- and line-prefix calculations, both search states, and UTF-8 position mapping.

It must contain:

- no `clap` dependency,
- no prompts,
- no terminal formatting,
- no reads from stdin,
- no direct UI output.

It should return typed data/results and errors to its caller.

### 3.2 `paircomp` (in `paircomp-cli/`)

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

### 4.2 Within-line localization

After `DifferenceAtLine`, a frontend may start a separate `ByteSearch`. Keep the line result as typed data and all byte reading, line-boundary discovery, hashing, and character-position mapping in the core. Do not embed search arithmetic or UTF-8 decoding in the CLI, or introduce a general search framework.

The additional public API is:

```rust
pub struct LineInfo {
    pub byte_len: u64,
}

pub fn inspect_line(path: &Path, line: u64) -> Result<Option<LineInfo>, Error>;
pub fn fingerprint_line_prefix(
    path: &Path,
    line: u64,
    byte_count: u64,
) -> Result<Fingerprint, Error>;
pub fn utf8_character_position(
    path: &Path,
    line: u64,
    byte: u64,
) -> Result<Option<u64>, Error>;

pub struct ByteSearch { /* bounds/state */ }
pub enum ByteSearchStep {
    CompareThroughByte { byte: u64 },
    DifferenceAtByte { byte: u64 },
}
impl ByteSearch {
    pub fn new(local_byte_len: u64, other_byte_len: u64) -> Result<Self, Error>;
    pub fn current_step(&self) -> ByteSearchStep;
    pub fn record_result(&mut self, matched: bool) -> Result<(), Error>;
}
```

All three line-local file operations require a 1-based line number; line zero returns an error. `inspect_line` returns `None` beyond EOF. Existing lines include LF when present and always have at least one byte. `fingerprint_line_prefix` hashes only the first `byte_count` bytes of the selected line, clamping to its end. Zero bytes or a missing line hashes the empty sequence; never include bytes from the next line or an EOF marker.

`utf8_character_position` validates the entire selected line, returning `None` for invalid UTF-8 or a missing line. Every byte in a multibyte code point maps to that code point's 1-based position. Immediately after an existing valid UTF-8 line, return the next code-point position. Byte zero and positions farther than one past the line return an error. CR and LF each count as a code point. An absent line permits byte position 1 but has no character position. Do not normalize text or substitute replacement characters.

Both searches require an established mismatch and unchanged files. Repeated `current_step` calls preserve state; answers after completion return an error. Byte search construction rejects two zero lengths. File operations reopen the path and use bounded buffers, including across split UTF-8 sequences, without loading an arbitrarily long line into memory.

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
8. Report the line number. If it exceeds the local line count, also report that the local file ends before this line. Offer the optional within-line search in section 6.3. After the user repairs the file manually, Paircomp is simply rerun. Do not attempt synchronization or patching.

Using the maximum of the two counts makes the initial bounds identical on both systems regardless of which file is local. Beyond-EOF prefix behavior ensures that comparison at the initial upper bound would hash the entire file on each system. Matching prefixes exclude all lines through the midpoint; mismatching prefixes retain the midpoint as a candidate.

### 6.1 Newlines and edge cases

- Lines are numbered starting at 1 and are defined by raw LF bytes (`0x0A`), regardless of encoding.
- Each LF terminates a line and belongs to that line's fingerprinted bytes. CR (`0x0D`) is ordinary content, including immediately before LF; a lone CR does not terminate a line.
- A nonempty suffix after the last LF counts as one unterminated line. A trailing LF does not create an additional empty line.
- The line count is the number of LF bytes, plus one if the file is nonempty and does not end in LF. An empty file has zero lines.
- A request through line 0 hashes the empty byte sequence. A request through any line beyond EOF hashes the entire available file, including for an empty file.
- Treat LF versus CRLF and the presence/absence of a final newline as real differences; do not normalize.
- File-level hashing, line localization, and byte localization must work for arbitrary bytes without UTF-8 decoding. UTF-8 decoding is used only for the optional character annotation; omit that annotation for invalid UTF-8. Do not display line contents or convert them lossily.

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

### 6.3 Within-line protocol and positions

1. After reporting the differing line, ask `Continue within this line? [Y/n]`. Both instances must choose the same answer. Declining completes the comparison with status 1.
2. When continuing, display the selected line's byte count, including any CR/LF. A missing line has count zero. Always ask for the other instance's count, even when it is equal or either line is absent. Accept a nonnegative decimal `u64`.
3. Set `low = 1` and `high = max(local_byte_len, other_byte_len)`. Reject `high == 0` as an inconsistent mismatch. The line search establishes equal preceding lines and a differing selected line, supplying the differing upper bound without an additional fingerprint comparison.
4. While `low < high`, request the line-local prefix through byte `mid = low + (high - low) / 2`. Hash from this line's beginning through the requested byte, clamping to the local line's end. An absent line hashes no bytes.
5. On a match, set `low = mid + 1`; otherwise set `high = mid`. When the bounds meet, return `DifferenceAtByte { byte: low }` without another comparison.
6. Report the line and 1-based byte position. If the local byte is absent, explain whether the local file ends before the line or the local line ends before the byte. Add the UTF-8 character position when available, as defined in section 4.2. Complete with status 1.

Both instances therefore request the same byte positions and locate the same first differing byte, including unequal line lengths, insertion/deletion of bytes, and an absent line. Fingerprints in this stage include only the selected line's prefix, rather than the preceding lines. Correctness retains the assumptions of accurate counts, consistent answers, stable files, and no fingerprint collision.

For UTF-8 `café` versus `cafè`, byte counts are both 5; comparisons through bytes 3 and 4 match, yielding byte 5 and character 4. For `b"a"` versus `b"a\n"`, the result is byte 2 and character 2 on both copies; the shorter copy explains that its byte is absent. For `b"a\r\n"` versus `b"a\n"`, the result is byte 2, comparing CR with LF. An absent line versus any existing line yields byte 1; only the existing line can have a character annotation.

Character positions count Unicode code points, including CR and LF separately, rather than grapheme clusters or visual editor columns. Combining marks and some emoji contain multiple code points. Invalid UTF-8 anywhere in the selected local line suppresses the annotation even when the differing byte precedes the invalid sequence; other lines' encodings do not affect it.

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
- offer within-line continuation, then display and obtain the selected line's byte counts,
- drive the two search states and format the byte result and optional character position.

It should not implement hashing or midpoint/search calculations itself.

Use `PathBuf`/`OsString`-compatible argument handling so Unix paths are not unnecessarily restricted to UTF-8.

### 7.1 Prompt input

- Match prompts accept `y`/`yes` and `n`/`no`, case-insensitively, after trimming surrounding whitespace.
- The displayed `[y/N]` default means that a submitted blank or whitespace-only answer is `no`.
- The continuation prompt `[Y/n]` accepts the same answers but defaults to `yes` on a submitted blank or whitespace-only answer. Tell users to choose the same continuation answer on both copies.
- The other-line-count and other-byte-count prompts have no default. Require decimal digits representing a `u64`, after trimming surrounding whitespace; zero is valid.
- Invalid answers or counts produce a diagnostic and terminate with status 2. The user must restart both instances to begin a new comparison session.
- Stdin EOF is an aborted interaction and terminates with status 2. It must never be interpreted as a blank answer or a sequence of `no` answers.
- A partial answer followed by EOF without a newline also aborts; only a newline submits a prompt answer.

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
Choose the same continuation answer on both copies.
Continue within this line? [Y/n] y

Line 737 size: 124 bytes (including any CR/LF)
Byte count displayed for this line by the other copy: 124

Compare line 737 through byte 62:
Fingerprint: <fingerprint>
Does this fingerprint match? [y/N] n

...

First divergence: line 737, byte 43 (UTF-8 character 43)

Inspect/correct the corresponding files, then run paircomp again.
```

## 9. Error handling

- Return library errors; do not print from `paircomp-core`.
- CLI diagnostics go to stderr; normal interactive/output information goes to stdout.
- Exit with status 0 when the user confirms a whole-file match, or for successful `--help`/`--version` output.
- Exit with status 1 after the user declines within-line continuation or successfully completes the byte search.
- Exit with status 2 for invocation errors, I/O errors, or aborted/invalid interaction, including a reported mismatch between two empty files or two zero-length lines. EOF at the continuation prompt is an abort, not a choice to finish. A detected difference is a completed comparison, distinct from these errors.
- Do not panic for expected user/file errors.
- Keep the error model simple for v0.1; avoid adding a large error-handling dependency unless it provides clear value.

## 10. Dependencies and licensing

Paircomp source should be licensed MPL-2.0.

Dependencies may use compatible permissive licenses. Before adding any dependency, verify its current license and record why it is needed. Keep dependencies deliberately small.

| Dependency | Crate | Purpose |
|---|---|---|
| `blake3` | `paircomp-core` | BLAKE3 fingerprinting |
| `clap` with `derive` | `paircomp` | CLI parsing/help/version |

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

Within-line tests must also cover:

- Exact byte prefixes, including byte zero, beyond-line requests, absent lines, LF and CRLF, and rejection of line zero for line-local operations.
- Paired line and byte searches with swapped counts and actual fingerprints: first/last-byte changes, inserted/absent bytes, unequal lengths, added lines, empty files, missing final newlines, CRLF versus LF, and invalid UTF-8.
- UTF-8 code-point mapping inside multibyte characters, emoji, combining marks, CR/LF, and immediately after a valid line. Validate the whole local line and omit annotations for absent or invalid UTF-8 lines. Reject zero or out-of-range byte coordinates.
- Long lines and UTF-8 sequences across read-buffer boundaries; short and interrupted reads; propagated I/O errors; search-state stability, zero-length mismatch rejection, completion errors, and maximum `u64` bounds.
- Focused CLI tests for default continuation, declining, byte-count exchange, unchanged match-prompt defaults, character annotations, local absence explanations, and invalid/aborted input at every new prompt. Include paired CLI sessions and verify that whole-file matches bypass both searches.

## 12. Explicit non-goals for v0.1

- No networking or direct communication between Paircomp instances.
- No automatic file transfer.
- No patch/delta generation or application.
- No rsync-style synchronization.
- No attempt to display a conventional two-file diff.
- No grapheme-cluster or visual-column localization, non-UTF-8 character decoding, or line-content excerpts.
- No directory-tree comparison.
- No configuration file.
- No selectable hash algorithms unless a concrete need appears.
- No GUI, but the architecture must permit one later.
- No elaborate plugin/framework architecture.
- No premature optimization of repeated file scans unless profiling shows it matters.

## 13. Implementation plan and checklist

Work through these milestones in order, marking items complete after the corresponding work and verification are finished. Each milestone may involve several small, focused commits. Choose commit boundaries around coherent changes, include their related tests and documentation, and keep intermediate commits buildable. Follow the commit-message, identity, attribution, and verification instructions in [AGENTS.md](AGENTS.md).

### 13.1 Foundation

- [x] Create the Cargo workspace with `paircomp-core` and the `paircomp` CLI package, with the CLI depending on the core and exposing a binary named `paircomp`.
- [x] Configure package/version/license metadata and use the existing `COPYING` license file.
- [x] Establish a buildable crate structure that preserves the architecture in section 3. Add dependencies only as they are needed, verifying and documenting their licenses and purpose at that time.

Completion criteria: the workspace builds, the core can be built independently, and dependency direction and crate responsibilities match section 3.

### 13.2 File inspection and prefix hashing

- [x] Implement `FileInfo`, full-file BLAKE3 fingerprinting, and typed library errors.
- [x] Implement line counting and fingerprinting through line N with the exact raw-byte, terminator, line-0, and beyond-EOF semantics in section 6.1.
- [x] Add the hashing and line-semantics tests from section 11, including empty files, final-newline differences, CRLF, and invalid UTF-8.

Completion criteria: tests verify the expected metadata and exact bytes included in each prefix; expected file errors are returned without panics or core UI output.

### 13.3 Deterministic search

- [x] Implement the core search state and exact bounds/transitions from sections 4.1 and 6, including unequal line counts and invalid state operations.
- [x] Add the paired-instance tests from section 11, using actual prefix comparisons to drive both states.
- [x] Preserve the typed line result and core byte operations needed for the separate within-line search in section 4.2.

Completion criteria: both simulated instances request the same line at every step, terminate at the expected first divergence, and handle all specified search edge cases without CLI involvement.

### 13.4 CLI workflow

- [x] Implement the `clap` interface, regular-file validation, path handling, metadata output, and fingerprint formatting. Choose and document the displayed fingerprint length and any truncation's collision probability as required by section 5.
- [x] Implement whole-file comparison, collection of the other count, and prompts that drive the core search state. Include the file-stability notice, beyond-EOF result message, input rules, diagnostics, and exit statuses from sections 6 through 9.
- [x] Add focused argument-parsing and interaction tests from section 11, covering successful matches, localized differences, and invalid/aborted interactions.

Completion criteria: `paircomp FILE`, `--help`, and `--version` work as specified; interaction tests verify exit statuses 0/1/2; the CLI contains no hashing or search calculations.

### 13.5 Documentation and MVP verification

- [x] Complete the README with build/run instructions, paired-instance usage, a comparison example, exit statuses, fingerprint limitations, and the requirement to restart after editing either file.
- [x] Complete dependency/license notices and document the MVP scope.
- [x] Run the verification checks in `AGENTS.md` and review the implementation against every requirement in section 11 and the definition of done in section 14. Exercise the workflow with two separate instances, each opening only its own file, for both matching and differing fixtures, including unequal line counts.

Completion criteria: required checks pass, documented usage matches the implemented behavior, and the definition of done is satisfied. Record verification results and any remaining limitations in the handoff; leave incomplete checklist items unchecked.

### 13.6 Optional within-line localization

- [ ] Add line inspection, bounded line-prefix hashing, and separate byte-search state with the protocol in section 6.3.
- [ ] Add UTF-8 code-point mapping with full-line validation and EOF/absence semantics from section 4.2.
- [ ] Add the optional CLI workflow with default-yes continuation, byte-count exchange, typed results, and unchanged match-prompt defaults.
- [ ] Add the within-line core and CLI tests from section 11, update project guidance and READMEs, and run the required verification checks.

Completion criteria: both instances deterministically locate the first differing byte when continuing, character annotations match the specified UTF-8 semantics, and users can still finish at the line. All required checks pass.

## 14. Definition of done for the MVP

On two systems containing corresponding files, a user can run `paircomp FILE` on each.

If the files are identical, the user can establish that immediately by comparing the whole-file fingerprints.

If they differ, entering the other instance's line count on each system and following the same match/no-match answers deterministically guides the user to the first differing line, including when the counts differ or one file is empty. Users can stop at that line or continue by exchanging its byte counts and comparing line-prefix fingerprints to locate the first differing byte. Both files must remain unchanged during the session.

The core functionality is exposed by `paircomp-core` with no dependency on `clap`, stdin/stdout, or terminal UI, and the CLI is a thin frontend over that API.

The MVP supports both line-level and optional within-line localization. Byte results work for arbitrary contents, and valid UTF-8 lines also receive Unicode code-point positions. Both search states and position mapping remain reusable independently of the CLI.
