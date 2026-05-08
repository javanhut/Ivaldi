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
| **Self-hosted SSH** | `git@gitea.example.com:team/proj.git` |
| **Self-hosted SSH w/ port** | `ssh://git@example.com:2222/team/proj.git` |
| **Peer-to-peer** | `ivaldi://10.0.0.5:9418` |
| **Peer-to-peer w/ timeline** | `ivaldi://alice.local:9418/main` |

GitLab variants (`gitlab.com` host, `gitlab:` prefix) resolve to
`platform=GitLab`. **SSH** URLs against any host are accepted — even
self-hosted Gitea / Forgejo / GitLab CE — because the SSH transport
doesn't need `platform` for routing. **`ivaldi://`** URLs synthesize a
portal with `owner="peer", repo="<host>:<port>"`; the actual transport
is decided by `Portal::transport()` at use time.

The `/tree/<branch>` suffix on HTTPS URLs is captured as `branch_hint`
and used by `ivaldi download` as the default branch to check out.

## `Portal::transport()` — auto-routing

Every place that uses a portal calls `portal.transport()` to decide which
transport stack runs. Returns one of:

| Variant | Trigger | Driver |
|---|---|---|
| `Transport::Https` (default) | shorthand / HTTPS URL / no `base_url` | `SmartHttpClient` + GitHub REST API |
| `Transport::Ssh(SshTarget)` | `base_url` parses as a git SSH URL | `SshClient` (subprocess `ssh`) |
| `Transport::Peer(PeerUrl)` | `base_url` parses as `ivaldi://` | Noise XX over TCP |

Implementation: at parse time, SSH and `ivaldi://` inputs round-trip the
*original URL* into `base_url`; HTTPS shorthands leave `base_url` as
`None`. `transport()` re-parses `base_url` to recover the typed target.
Nothing on disk changes — the existing `<platform> <owner>/<repo> [base_url]`
file format already carries the URL.

## CLI

```bash
# Any accepted format works — transport picked automatically
ivaldi portal add torvalds/linux                       # HTTPS
ivaldi portal add https://github.com/torvalds/linux.git
ivaldi portal add git@github.com:torvalds/linux.git    # SSH (system ssh agent)
ivaldi portal add ssh://git@gitea.example.com:2222/team/proj.git  # self-hosted SSH
ivaldi portal add ivaldi://10.0.0.5:9418               # peer-to-peer

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
    Err(RepoSpecError::UnsupportedHost(host))    => // bitbucket.org HTTPS, etc.
                                                    //   (SSH inputs to ANY host
                                                    //    are accepted — see SSH transport)
    Err(RepoSpecError::Invalid)                  => // malformed URL
    Ok(spec)                                     => // ...
}
```

## Storage

`.ivaldi/portals` — one portal per line. Format:
`<platform> <owner>/<repo> [base_url]`. The `base_url` column round-trips
SSH and `ivaldi://` URLs so `Portal::transport()` can re-derive the
right transport without separate state.

```
github javanhut/IvaldiVCS
gitlab team/project https://gitlab.internal.com
github team/proj git@gitea.example.com:team/proj.git
github peer/10.0.0.5:9418 ivaldi://10.0.0.5:9418
```

(For `ivaldi://` portals, `platform` and the synthesized `owner/repo`
are placeholders — only `base_url` is consulted.)

## Cross-references

- [`docs/ssh.md`](ssh.md) — SSH transport (any git host).
- [`docs/p2p.md`](p2p.md) — Ivaldi-native peer-to-peer.
- [`docs/sync.md`](sync.md) — `RemoteFetcher` enum that dispatches based
  on `portal.transport()`.
