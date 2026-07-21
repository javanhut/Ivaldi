# Dogfooding Ivaldi with a Git mirror

Ivaldi's own repository is developed with Ivaldi as the primary VCS and
authoritative working history. Every sealed timeline is additionally exported
to a Git remote as an independent, cross-format disaster-recovery copy during
the pre-1.0 compatibility period.

This mirror is defense in depth, not an architectural dependency or backend.
Git does not participate in Ivaldi's native storage, seals, timelines, fusion,
recovery, or peer synchronization, and Ivaldi does not invoke Git for native VCS
operations. The mirror exercises the optional compatibility bridge while also
keeping an off-site copy in a separately implemented format.

The workflow below is the one this repo actually uses.

## 1. Run Ivaldi alongside the existing `.git`

`.ivaldi/` and `.git/` coexist in the same working tree — neither reads the
other. Initialize Ivaldi in a repo that already has Git:

```bash
ivaldi forge            # creates .ivaldi/ next to .git/
```

Keep them from tracking each other:

```bash
ivaldi exclude ".git/"  # add .git/ to .ivaldiignore
echo ".ivaldi/" >> .gitignore
```

Seed Ivaldi's history from the Git one by importing the remote once, so the
mirror fast-forwards instead of diverging:

```bash
ivaldi download git@github.com:you/repo.git .   # import existing history
```

## 2. Do daily work in Ivaldi

Normal Ivaldi loop — see [quick-start.md](quick-start.md):

```bash
ivaldi gather .
ivaldi seal "message"
```

The mirror pushes **sealed** history. Uncommitted or un-`gather`ed changes are
not backed up until you seal them — same as Git.

## 3. Mirror to Git

[`scripts/mirror-to-git.sh`](../scripts/mirror-to-git.sh) pushes every timeline
to a Git remote using `ivaldi upload` (SSH speaks the git pack protocol; HTTPS
uses the host's REST API). It **never force-pushes**, so the mirror can only fast-forward — a
diverged mirror fails loudly rather than being overwritten.

```bash
# One-off, or set IVALDI_MIRROR_REMOTE once in your shell profile.
scripts/mirror-to-git.sh git@github.com:you/repo.git
```

The script adds the remote as an Ivaldi portal (idempotent), then uploads
each timeline non-force with `upload --portal` — the mirror works no matter
which portal is the default.

## 4. Make it continuous

Run the script on a schedule so the mirror trails your seals automatically.

**Cron** (verified reliable) — mirror every 15 minutes:

```cron
*/15 * * * * cd /path/to/repo && IVALDI_MIRROR_REMOTE=git@github.com:you/repo.git scripts/mirror-to-git.sh >> /tmp/ivaldi-mirror.log 2>&1
```

**On seal** — Ivaldi lays down `.ivaldi/hooks/` for pre/post-operation scripts;
dropping a call to the mirror script there ties a push to each seal. Cron is the
safer default because it still runs if a hook is skipped, and it retries the next
tick after a transient network failure.

## What is automated vs manual

| Step | Status |
|---|---|
| Push all sealed timelines to Git | **Automated** (`mirror-to-git.sh`) |
| Non-force / never-overwrite safety | **Automated** (script refuses `--force`) |
| Run on a schedule | **Automated once** you install the cron entry above |
| `ivaldi forge` + import existing Git history | Manual, one-time (step 1) |
| `ivaldi seal` your work | Manual (that's the point) |
| Recover *from* the Git mirror after local loss | Manual: `ivaldi download <remote>` into a fresh checkout |

## Portal selection

`ivaldi upload` and `ivaldi sync` target the default (first) portal unless
`--portal owner/repo` names another; `ivaldi portal set-default owner/repo`
changes which portal is the default. The mirror script passes `--portal`
explicitly, so the mirror does not need to be the default portal.
