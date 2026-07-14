#!/usr/bin/env bash
#
# mirror-to-git.sh — mirror this Ivaldi repository's sealed history to a Git remote.
#
# Ivaldi is the primary VCS for this repo; the Git remote is the backup of record
# while Ivaldi is pre-1.0 (see docs/dogfooding.md). This pushes every timeline to
# a configured Git portal using Ivaldi's own `upload` (SSH speaks the git pack
# protocol; HTTPS uses the host's REST API). It NEVER force-pushes: the mirror
# can only fast-forward, so a diverged
# mirror fails loudly instead of being overwritten.
#
# Usage:
#   scripts/mirror-to-git.sh [GIT_REMOTE]
#   IVALDI_MIRROR_REMOTE=git@github.com:you/repo.git scripts/mirror-to-git.sh
#
# GIT_REMOTE is anything `ivaldi portal add` accepts, e.g.:
#   you/repo                       GitHub HTTPS shorthand
#   git@github.com:you/repo.git    SSH (uses your ssh agent/keys)
#   ssh://git@gitea.example:22/you/repo.git   self-hosted SSH
#
# Environment:
#   IVALDI_MIRROR_REMOTE   remote to mirror to (the CLI arg overrides it)
#   IVALDI                 ivaldi binary to use (default: ivaldi)
#
set -euo pipefail

IVALDI="${IVALDI:-ivaldi}"

usage() {
  sed -n '3,22p' "$0" | cut -c3-
}

# Self-check of the three load-bearing text parses below — no repo or network
# needed. Run with `mirror-to-git.sh --self-test`.
if [ "${1:-}" = "--self-test" ]; then
  add="Added portal: javanhut/ivaldi"
  [ "${add##*: }" = "javanhut/ivaldi" ] || { echo "repr parse FAIL"; exit 1; }
  def="$(printf '    "repo": "you/repo",\n' | grep -m1 '"repo"' | sed 's/.*: *"\(.*\)".*/\1/')"
  [ "$def" = "you/repo" ] || { echo "default parse FAIL"; exit 1; }
  tls="$(printf '* main\n  feature\n' | cut -c3-)"
  [ "$tls" = "$(printf 'main\nfeature')" ] || { echo "timeline parse FAIL"; exit 1; }
  echo "self-test ok"
  exit 0
fi

case "${1:-}" in
  -h|--help) usage; exit 0 ;;
esac

REMOTE="${1:-${IVALDI_MIRROR_REMOTE:-}}"
if [ -z "$REMOTE" ]; then
  echo "error: no git remote given (pass it as an arg or set \$IVALDI_MIRROR_REMOTE)." >&2
  echo >&2
  usage >&2
  exit 2
fi

# Must run from the repository root.
if [ ! -d .ivaldi ]; then
  echo "error: no .ivaldi/ here — run this from the repository root." >&2
  exit 2
fi

if ! command -v "$IVALDI" >/dev/null 2>&1; then
  echo "error: '$IVALDI' not found on PATH (set \$IVALDI or run 'make install')." >&2
  exit 2
fi

# 1. Ensure a portal for the mirror exists. Idempotent: re-adding is a no-op.
add_out="$("$IVALDI" portal add "$REMOTE")"
echo "$add_out"
mirror_repr="${add_out##*: }"   # "Added portal: X" / "Portal already configured: X"

# 2. `ivaldi upload` always targets the DEFAULT (first) portal, and there is no
#    CLI today to select a portal or change the default (see cmd_upload and
#    PortalManager::get_default — it returns list().next()). So this script can
#    only drive the mirror when the mirror IS the default portal. Verify that and
#    refuse rather than silently push somewhere else — a wrong-remote "backup" is
#    worse than an obvious failure.
#    TODO(ivaldi): add `ivaldi upload --portal <repr>` (or `portal set-default`)
#    and delete this guard.
default_repr="$("$IVALDI" portal list --json | grep -m1 '"repo"' | sed 's/.*: *"\(.*\)".*/\1/')"
if [ "$default_repr" != "$mirror_repr" ]; then
  echo "error: mirror portal '$mirror_repr' is not the default portal ('$default_repr')." >&2
  echo "       'ivaldi upload' only pushes to the default (first) portal." >&2
  echo "       Make the mirror the default — remove the others, or add the mirror" >&2
  echo "       first into a repo with no portals:" >&2
  echo "         ivaldi portal remove $default_repr" >&2
  exit 3
fi

# 3. Push every timeline at its current head, non-force. A rejected timeline (a
#    mirror that somehow diverged) fails the run but does not stop the others, and
#    we never fall back to --force.
# Capture the list first so a failure to list is caught — pipefail does not
# cover the process substitution a `done < <(...)` loop would use.
if ! timelines="$("$IVALDI" timeline list | cut -c3-)"; then
  echo "!! could not list timelines" >&2
  exit 1
fi

failed=0
while IFS= read -r tl; do
  [ -n "$tl" ] || continue
  echo ">> mirroring timeline: $tl"
  if ! "$IVALDI" upload "$tl"; then
    echo "!! timeline '$tl' rejected (not a fast-forward?) — NOT forcing." >&2
    failed=1
  fi
done <<< "$timelines"

if [ "$failed" -ne 0 ]; then
  echo "mirror finished with rejected timelines; the Git mirror was left untouched." >&2
  exit 1
fi
echo "mirror complete: all timelines pushed to '$mirror_repr'."
