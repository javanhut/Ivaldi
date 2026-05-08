# SSH Transport

Ivaldi speaks the standard git pack protocol over SSH for both fetch and
push. This works against any SSH-reachable git host — github.com,
gitlab.com, self-hosted Gitea / Forgejo / GitLab CE, plain `git@server`
repos, anywhere `ssh user@host git-upload-pack '/path'` would work for
the regular `git` CLI.

We deliberately **do not** ship an in-process SSH stack (no `russh` /
`libssh2`). Instead we spawn the system `ssh` binary as a subprocess and
pipe pack-protocol bytes over its stdin/stdout. Your existing keys, agent,
`~/.ssh/config`, and `known_hosts` keep working — anything we'd
reimplement we'd implement worse.

## URL forms

Both standard git URL shapes are accepted:

```text
git@host.example.com:owner/repo.git              # scp-like
git@host.example.com:/abs/path/to/repo.git       # scp-like, absolute path
ssh://git@host.example.com/owner/repo.git        # full URL, default port
ssh://git@host.example.com:2222/team/proj.git    # custom port
ssh://host.example.com/owner/repo.git            # default user (`git`)
```

The transport is auto-detected from the URL — no flags needed:

```bash
ivaldi portal add git@github.com:owner/repo.git
ivaldi portal add ssh://git@gitea.example.com:2222/team/proj.git
ivaldi download git@example.com:team/repo.git
```

## What works

- **Fetch** — `download`, `scout`, `harvest`. Spawns
  `ssh ... git-upload-pack '<path>'`, reads the ref advertisement, sends
  one `want <sha>` for the requested branch, drains the resulting pack.
- **Push** — `upload`. Spawns `ssh ... git-receive-pack '<path>'`, reads
  the ref advertisement, builds a git-format packfile of the new objects
  via `git_export` + `git_pack_writer`, sends an update command + flush
  + pack, parses the `report-status` response.
- **Full bidirectional fidelity**. A repo cloned via `ivaldi download`
  and re-pushed via `ivaldi upload` lands with **byte-identical commit
  SHA-1s** to the original — confirmed live against `octocat/Hello-World`'s
  `7fd1a60b…`. Author, committer, timestamps, and timezone offsets all
  round-trip exactly (see [git_export.md](git_export.md) for the
  translation rules).

## Authentication

SSH auth is whatever `ssh` itself does — agent, key files,
`~/.ssh/config`. We pass `-o BatchMode=yes` so non-interactive failures
don't hang waiting for a password prompt, and `-o ServerAliveInterval=30`
so the connection stays warm for slow packs.

There is **no** `ivaldi auth login` flow for SSH. If `git@host` works
with `git push`, it works with `ivaldi upload`.

When auth fails, the underlying `ssh` stderr is surfaced verbatim:

```text
$ ivaldi download git@example.com:no-access.git
Error: ssh: Permission denied (publickey).
```

## Push protocol (`git-receive-pack`)

```text
                    ssh stdin
   client  ───────────────────────►  server (git-receive-pack)
   <command pkt-line>\0report-status agent=ivaldi/0.1.0\n
   0000  (flush)
   <packfile bytes — git v2 format with SHA-1 trailer>

                    ssh stdout
   client  ◄───────────────────────  server
   "unpack ok\n"
   "ok refs/heads/<branch>\n"  (or "ng <ref> <reason>\n")
   0000  (flush)
```

Capabilities we declare:

- `report-status` — required so the server tells us per-ref outcome
- `agent=ivaldi/0.1.0` — informational

We deliberately do **not** request `side-band-64k` for v1 — keeps the
response parser trivial.

## Object set construction

The Ivaldi → git translation is in `src/git_export.rs`. For each push:

1. Read the server's ref advertisement; collect the SHA-1s it already has.
2. Walk back from the local timeline head along `prev_idx + merge_idxs`,
   collecting every leaf whose mapped git SHA-1 is **not** already
   server-known. (This is per-server, not global — we don't skip a leaf
   just because it was seen on some other portal.)
3. For each unmapped leaf:
   - Recursively translate its tree (Ivaldi tree → git tree).
   - Mint git commit canonical bytes from leaf fields. For leaves
     originally imported from git, the `git.committer` /
     `git.committer_time` / `git.author_tz` meta keys carry the original
     identity verbatim, producing SHA-1-identical output.
4. Pack everything with `src/git_pack_writer.rs` (v2, no deltas — bigger
   wire payload but minimum new code; deltas can come later).

Convenient invariants from Ivaldi's storage:

- Ivaldi blob CAS bytes are *literally* git blob canonical form
  (`blob <size>\0<content>`). No translation needed beyond stripping the
  envelope to get the body for packing.
- Ivaldi tree-node hashes (BLAKE3) and git tree-node SHA-1s are independent —
  we recompute when translating, but the underlying file content is
  shared.

## Limitations / not yet built

- **No deltas in the pack writer**. Pushes are slightly larger than what
  `git push` would send. Receive-side decompression is identical.
- **No `--force-with-lease`**. `--force` is plumbed through but does the
  full force; lease semantics are a follow-up.
- **No multi-branch push in one round-trip**. Each `upload` ships one
  branch.
- **No `git-receive-pack` push-options or atomic transactions**. Servers
  that require these will need follow-up work.

## Files / modules

- `src/ssh_transport.rs` — `SshTarget` URL parser, `SshClient::fetch_repo`,
  `SshClient::push_repo`, `parse_report_status`.
- `src/git_export.rs` — Ivaldi commit/tree/blob → git object translation.
- `src/git_pack_writer.rs` — git v2 pack format encoder.
- `src/git_remote.rs` — shared protocol helpers (`pkt_line`,
  `parse_discovery`, `parse_packfile`, `git_object_id`).
- `src/portal.rs` — `Portal::transport()` returns
  `Transport::Ssh(SshTarget)` for SSH URLs.

## Verifying a real round-trip

```bash
git init --bare /tmp/round-trip.git
mkdir hw-clone-parent && cd hw-clone-parent
ivaldi download octocat/Hello-World hw && cd hw
ivaldi portal remove octocat/Hello-World
ivaldi portal add "git@local:/tmp/round-trip.git"   # use a local ssh shim or a real ssh host
ivaldi config --set user.name "RT"
ivaldi config --set user.email "rt@x"
ivaldi upload master

# Confirm the commit SHA-1 matches upstream:
git -C /tmp/round-trip.git rev-parse refs/heads/master
# 7fd1a60b01f91b314f59955a4e4d4e80d8edf11d
```
