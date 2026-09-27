# Agent instructions

These instructions apply throughout this repository.

## Project and scope

Paircomp is a Rust utility for manually comparing files on isolated systems and locating their first differing line and, optionally, byte within that line using prefix fingerprints.

Read [paircomp_mvp_implementation_spec.md](paircomp_mvp_implementation_spec.md) before implementation. It is the source of truth for product behavior, architecture, the comparison protocol, edge cases, and acceptance criteria. Follow the user's current task within that design; the presence of the spec is not a request to implement every remaining feature at once. Keep the spec consistent with intentional behavior changes.

Within-line localization is part of v0.1. Search raw-byte prefixes and report a UTF-8 code-point position only when the selected local line is valid UTF-8. Keep line and byte search state frontend-neutral; avoid unused APIs or a general search framework.

## Implementation principles

- Keep file inspection, hashing, line boundaries, and search state in `paircomp-core`. The core must not depend on `clap`, terminal interaction, or UI output.
- Keep the `paircomp` CLI crate focused on arguments, prompts, formatting, and driving the core API. Do not duplicate search logic in the CLI.
- Compare raw bytes. Follow the spec's exact line, newline, and EOF semantics; do not normalize content or require UTF-8 to hash or localize differences.
- Prefer straightforward Rust, typed results, and small abstractions. Return errors for expected failures instead of panicking. Avoid speculative frameworks and optimizations.
- Keep dependencies small. Before adding one, verify its current license against the spec's requirements and record its purpose and license in the project's dependency documentation.
- Preserve the MPL-2.0 license in `COPYING` and use it as the existing project license file.

## Working practices

- Inspect the working tree and relevant existing code before editing. Preserve unrelated user changes and stage only files or hunks belonging to the intended commit.
- Keep changes focused on the requested outcome. Include directly related tests and documentation; avoid unrelated cleanup or formatting.
- Resolve routine implementation details using the spec and existing conventions. Ask when a missing decision materially affects behavior or scope; do not silently change a requirement.
- Put algorithm and edge-case tests in the core. Include the spec's paired-instance tests to verify that both sides request identical comparisons. Keep CLI tests focused on user interaction and exit statuses.
- In the handoff, summarize the changes, verification performed, and any remaining limitations. Clearly distinguish checks that passed from checks that could not be run.

## Verification

Once the Cargo workspace exists, run these checks for Rust changes before handing off or committing:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Run additional targeted checks when the changed behavior warrants them. If a check fails or the environment prevents it from running, report the reason and address failures caused by the change. Do not claim unrun checks passed.

For documentation-only changes, review consistency and run `git diff --check`; Cargo checks are unnecessary. Also inspect newly created files, which ordinary `git diff` does not include until staged.

## Commits

Use small, focused commits. Each commit should represent one coherent change, with its directly related tests and documentation, and be understandable and reviewable on its own. Do not split a change so finely that intermediate commits are needlessly broken.

Use a concise, descriptive subject and a thorough body. Explain the problem or motivation, what changed and why, relevant design decisions or tradeoffs. Scale the explanation to the change; avoid generic filler and file-by-file narration.

The required committer identity is:

- Name: `Justin Donnelly`
- Email: `justinrdonnelly@gmail.com`

Use the same identity as the author of new commits. When amending, rebasing, or cherry-picking existing commits, preserve their original author unless the user requests otherwise. Set identity for the commit command rather than changing global Git configuration, and verify the resulting author and committer metadata.

For example:

```sh
git -c user.name="Justin Donnelly" -c user.email="justinrdonnelly@gmail.com" commit
```

Every AI-assisted commit must include an `Assisted-by` trailer identifying the AI tool/model actually used, separated from the body by a blank line. For example, when the assisting model is GPT-6-Astra:

```text
Assisted-by: GPT-6-Astra
```

Use the actual identifier available in the session; do not copy the example for a different model or invent an unknown model version. If several tools/models materially contributed, include a separate `Assisted-by` trailer for each. Preserve existing assistance trailers when amending a commit and add any new attribution that applies.
