# Security Policy

Ivaldi is a version control system and treats silent data loss, repository
escape, integrity bypass, credential disclosure, and unauthenticated remote
code execution as security issues.

## Supported versions

Ivaldi is currently pre-1.0. Security fixes are applied to the latest release
line and `main`; older pre-1.0 releases may require upgrading because their
public and repository-format contracts are not yet stable.

No pre-1.0 release is represented as production-ready. The criteria for that
designation are tracked in [plan.md](plan.md).

## Reporting a vulnerability

Do not open a public issue for a suspected vulnerability. Use GitHub's private
vulnerability reporting for this repository:

<https://github.com/javanhut/ivaldi/security/advisories/new>

Include, where possible:

- the affected Ivaldi version or commit;
- operating system and filesystem;
- a minimal reproducer or damaged repository fixture with private data removed;
- the expected and observed behavior;
- whether user data, credentials, repository boundaries, or remote peers are
  affected; and
- any known workaround.

Reports should receive an acknowledgement within seven days. Triage will
classify impact, identify supported affected versions, and coordinate a fix and
disclosure date with the reporter. Please allow a reasonable remediation window
before public disclosure, especially when repository-format recovery is needed.

## Security response principles

- Preserve evidence and recoverable user data; never destroy a damaged
  repository as part of remediation.
- Fail closed when authenticity or integrity cannot be established.
- Publish affected versions, upgrade instructions, and recovery steps.
- Add a regression test or fuzz corpus entry for every reproducible parser or
  integrity defect.
- Rotate or revoke signing material if release provenance may be affected.

## Scope

Security-sensitive surfaces include repository parsing, filesystem paths,
atomic updates, locks, migrations, pack/delta handling, Git transports, P2P
identity and trust, authentication tokens, hooks, rescue/recovery output, and
release artifacts.

General bugs and feature requests that do not expose sensitive data or cross a
trust boundary may be filed in the public issue tracker.
