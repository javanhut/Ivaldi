# Contributing to Ivaldi

Thank you for your interest in Ivaldi. This is a version control system whose
entire purpose is to not lose people's work, so the bar for contributions is
correctness and accountability, not volume. Please read this document in full
before opening a pull request.

## Licensing of contributions

Ivaldi is licensed under [AGPL-3.0-only](LICENSE). By submitting a
contribution, you agree that your contribution is licensed under the same
terms (inbound = outbound). Do not submit code you do not have the right to
license this way.

## The core principle: the committer owns the commit

**Every commit is the full responsibility of the person who commits it.**

Whoever's name is on a commit is certifying that they have read the code, they
understand what it does, and they stand behind it. There is no such thing here
as "the tool wrote it" as a defense for a defect. If it merges under your name,
you own it.

To make that accountability explicit, every commit must be signed off using
the Developer Certificate of Origin:

```
git commit -s
```

This appends a `Signed-off-by: Your Name <you@example.com>` trailer, which
certifies that you wrote or have the right to submit the code and that you take
responsibility for it. Commits without a sign-off will not be merged.

## AI-assisted contributions

AI assistance is **permitted**. Ivaldi does not care whether a human or a model
typed the first draft — it cares that a human has reviewed, tested, and taken
responsibility for the result.

Two rules apply:

1. **A person must commit the work.** An AI agent may generate code, but a
   human must be the committer, must have actually reviewed every line, and
   signs off under their own name (see above). You are 100% responsible for
   checking the code — not the model.

2. **External contributors must disclose AI involvement.** If you are not a
   core maintainer and any part of a contribution was produced with AI
   assistance, state it explicitly in the commit message using these trailers:

   ```
   AI-Assisted: yes
   AI-Model: <name and version, e.g. Claude Opus 4.x, GPT-4o, Llama 3.1>
   AI-Interaction: <one line on how it was used, e.g. "drafted the pack
                    reader; human-reviewed, edited, and tested">
   ```

   Non-disclosure of AI involvement in an external contribution is grounds for
   rejecting the pull request.

Disclosure does not lower the bar — reviewed, human-owned AI output is welcome;
unreviewed AI output is not, regardless of who typed it.

## Tests are a specification, not an obstacle

Existing tests describe behavior Ivaldi has promised to preserve. They are the
guardrail against silent data loss. Treat them accordingly.

- **All existing tests must keep passing.** A change that breaks a test is
  breaking a promise; fix the code, not the test.
- **You may not modify or delete a test authored by the maintainers as part of
  another change.** A known failure mode of AI agents (and rushed humans) is to
  edit a failing test until it passes instead of fixing the underlying bug.
  That is not allowed here.
- If you genuinely believe an existing test is wrong, that is its **own,
  separate pull request** that changes only the test and explains, in detail,
  why the previously-asserted behavior was incorrect. It must be reviewed and
  approved on its own merits. It may never be bundled silently into a feature
  or bug-fix PR.
- **New code must come with new tests.** Any non-trivial change adds tests that
  fail before your change and pass after it.

## Pull request format

Every pull request description must contain, at minimum, these four sections:

1. **Files changed** — the list of files touched and a one-line note on the
   role of each.
2. **What changed** — a concrete description of the change, not a restatement
   of the title.
3. **Why** — the problem being solved or the behavior being added, and why this
   is the right fix (root cause, not symptom).
4. **Tests added** — which tests were added or updated, and how to run them. If
   you touched an existing maintainer-authored test, call it out prominently
   here and justify it (see the tests section above).

Commit messages must be explicit and self-contained. State what the commit does
and why. "fix" and "update" are not acceptable commit messages.

## Merge requirements

A pull request may be merged into `main` only when **all** of the following
hold:

- [ ] The full test suite passes in CI — no skipped, ignored, or disabled
      tests to get it green.
- [ ] No maintainer-authored test was modified or deleted (unless this PR is
      the dedicated, approved test-change PR described above).
- [ ] Every commit is signed off (`-s`), and AI involvement is disclosed where
      required.
- [ ] The PR description has all four sections above.
- [ ] The change has been reviewed and approved by a maintainer.

`main` is protected. Nothing merges that has not gone through this process.
