# Versioning and Deprecation Policy

Ivaldi uses [Semantic Versioning](https://semver.org): every release is
`MAJOR.MINOR.PATCH`.

- **MAJOR** — incremented for a breaking change to the public contract (below).
- **MINOR** — incremented for backward-compatible new functionality.
- **PATCH** — incremented for backward-compatible bug fixes only.

## What "the public contract" means for Ivaldi

Ivaldi is a tool, not a library, so semantic versioning applies to the surfaces
users and automation actually depend on:

1. **CLI commands and flags** — command names, their arguments, and their
   documented behavior.
2. **Machine-readable output** — JSON schemas and documented exit codes.
3. **On-disk repository format** — the layout versioned by `.ivaldi/FORMAT`.
4. **Network and P2P protocol** — the wire format exchanged between peers.

A change that alters any of these in a way existing users cannot rely on
through is a breaking change and requires a MAJOR bump. Internal code, private
modules, undocumented behavior, and log wording are **not** part of the
contract and may change in any release.

### Repository format is never silently broken

The on-disk format is the strongest promise Ivaldi makes. Regardless of version
number:

- A newer format is introduced only with a forward migration and an automatic
  pre-migration backup.
- An older binary refuses a newer format with a clear error rather than
  guessing.
- No release ever discards recoverable data to complete an upgrade.

See the repository-format and migration work in [`plan.md`](plan.md), Gate 3.

## Pre-1.0 (the current `0.x` series)

Ivaldi is an implemented, comprehensively tested standalone VCS. The `0.x`
designation does not mean prototype, proof of concept, Git wrapper, or
Git-dependent frontend. It means the complete long-term public contract has not
yet been frozen. During `0.x`:

- A **MINOR** bump (`0.1 → 0.2`) may include breaking changes.
- A **PATCH** bump (`0.1.1 → 0.1.2`) is reserved for bug fixes.
- The repository-format guarantee above still holds — even pre-1.0, upgrades do
  not lose data.

The `1.0` release is defined by the certification and long-term support gates in
[`plan.md`](plan.md). Unchecked gates identify additional evidence or
commitments; they do not by themselves classify implemented native features as
absent or untested. From `1.0` onward, the full semantic-versioning contract
applies.

## Releases are cut by the maintainer

There is no fixed release cadence and no automatic version bumping. **The
maintainer decides when a release is cut and what version it carries.** Merging
to `main` does not by itself produce a release. A release exists only when the
maintainer tags it and publishes it.

Version numbers therefore describe the *nature* of the changes since the last
release (breaking / feature / fix), not a schedule.

## Deprecation policy

Nothing in the public contract is removed without warning. When a feature is
deprecated:

1. **It is announced** in the release notes of the release that deprecates it,
   with the reason and the recommended replacement.
2. **It keeps working** and emits a visible deprecation warning (on `stderr`,
   so machine-readable `stdout` stays clean) whenever it is used.
3. **It is removed only in a later MAJOR release** — never in a MINOR or PATCH —
   and no earlier than **both** of: one full MINOR release after it was first
   deprecated, and 90 calendar days after that deprecating release was
   published. Whichever is later governs, so users always have a released,
   non-breaking version and a minimum window of real time in which to migrate.

Removal of a deprecated feature is itself a breaking change and is listed
prominently in the MAJOR release notes.

Deprecation warnings can be suppressed for scripting with an explicit opt-out,
but the default is always to warn, so a deprecation is never silent.
