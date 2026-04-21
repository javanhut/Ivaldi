# Portal Module (`portal.rs`)

Remote repository connection management for Ivaldi VCS.

## Overview

Portals represent connections to remote repositories (GitHub, GitLab,
self-hosted). Every place that accepts a repository identifier — `portal add`,
`download`, the TUI portal dialog — runs the string through the same
`parse_repo_spec` function, so any supported format works anywhere.

## Accepted formats

All of these resolve to `owner=torvalds, repo=linux, platform=GitHub`:

| Format | Example |
|---|---|
| Shorthand | `torvalds/linux` |
| Shorthand w/ `.git` | `torvalds/linux.git` |
| Platform shorthand | `github:torvalds/linux` |
| Bare host | `github.com/torvalds/linux` |
| HTTPS URL | `https://github.com/torvalds/linux` |
| HTTPS w/ `.git` | `https://github.com/torvalds/linux.git` |
| HTTP URL | `http://github.com/torvalds/linux` |
| SSH (scp-style) | `git@github.com:torvalds/linux.git` |
| SSH URL | `ssh://git@github.com/torvalds/linux.git` |
| URL with branch | `https://github.com/torvalds/linux/tree/master` |

GitLab variants (`gitlab.com` host, `gitlab:` prefix) resolve to
`platform=GitLab`. Hosts other than `github.com` / `gitlab.com` are rejected
with `RepoSpecError::UnsupportedHost`.

The `/tree/<branch>` suffix is captured as `branch_hint` and used by
`ivaldi download` as the default branch to check out.

## CLI

```bash
# Any accepted format works:
ivaldi portal add torvalds/linux
ivaldi portal add https://github.com/torvalds/linux.git
ivaldi portal add git@github.com:torvalds/linux.git

ivaldi download https://github.com/rust-lang/book/tree/main
#                                                  ^^^^^^^^ becomes the checked-out branch

ivaldi portal list
ivaldi portal remove owner/repo
```

Platform is inferred from the host (e.g. `gitlab.com` → GitLab) — `--gitlab`
still works as an override for shorthand input.

## Library Usage

```rust
use ivaldi::portal::{parse_repo_spec, Portal, PortalManager, Platform, RepoSpec};

// Structured parse (full info, including branch hint)
let spec: RepoSpec = parse_repo_spec("https://github.com/torvalds/linux/tree/master")?;
assert_eq!(spec.owner, "torvalds");
assert_eq!(spec.repo, "linux");
assert_eq!(spec.platform, Platform::GitHub);
assert_eq!(spec.branch_hint.as_deref(), Some("master"));

// Legacy wrapper returning Option<Portal>
let portal = Portal::parse("git@github.com:torvalds/linux.git").unwrap();

let mgr = PortalManager::new(&ivaldi_dir);
mgr.add(&portal)?;

// GitLab with custom URL
let gl = Portal::parse("team/project").unwrap()
    .with_platform(Platform::GitLab)
    .with_base_url("https://gitlab.internal.com");
mgr.add(&gl)?;

// List, get default, get specific
let portals = mgr.list()?;
let default = mgr.get_default()?;
let specific = mgr.get("owner/repo")?;

// Remove
mgr.remove("owner/repo")?;
```

## Errors

```rust
use ivaldi::portal::RepoSpecError;

match parse_repo_spec(input) {
    Err(RepoSpecError::Empty)                    => // empty input
    Err(RepoSpecError::MissingSegment)           => // "noslash", "empty/", etc.
    Err(RepoSpecError::UnsupportedHost(host))    => // bitbucket.org, etc.
    Err(RepoSpecError::Invalid)                  => // malformed URL
    Ok(spec)                                     => // ...
}
```

## Storage

`.ivaldi/portals` — one portal per line:

```
github javanhut/IvaldiVCS
gitlab team/project https://gitlab.internal.com
```
