# Ivaldi Sync Quick Reference

## Timeline Discovery

### List all remote timelines
```bash
ivaldi scout origin
# or
ivaldi portal list-timelines origin
```

**Output:**
```
Found 5 remote timelines:
  - main (latest: 2024-01-15)
  - develop (latest: 2024-01-14)
  - feature-auth (latest: 2024-01-13)
  - bugfix-123 (latest: 2024-01-12)
  - release-2.0 (latest: 2024-01-10)
```

## Sync Operations

### Sync current timeline
```bash
ivaldi sync origin
# Syncs current timeline with same-named remote timeline
```

### Sync specific timeline
```bash
ivaldi sync origin --timeline feature-auth
# Syncs feature-auth from remote to local
```

### Sync all remote timelines
```bash
ivaldi sync origin --all
# Discovers and syncs ALL remote timelines
```

**Progress:**
```
Discovered 5 remote timelines: main, develop, feature-auth, bugfix-123, release-2.0

Syncing timeline: main
├─ Downloading files... [====================] 100% (42/42)
└─ Successfully synced timeline: main

Syncing timeline: develop
├─ Downloading files... [====================] 100% (15/15)
└─ Successfully synced timeline: develop

Summary: Synced 5 timelines, 0 failed
```

### Sync selected timelines
```bash
ivaldi sync origin --timelines main,develop,feature-auth
# Only syncs specified timelines
```

## Upload Operations

### Upload current timeline
```bash
ivaldi upload
# Uploads current timeline to remote
```

### Upload all local timelines
```bash
ivaldi upload --all
# Uploads ALL local timelines to remote
```

**Output:**
```
Uploading 3 timelines to origin:
  ✓ main: 42 files uploaded
  ✓ feature-x: 15 files uploaded (new branch created)
  ✓ bugfix-y: 8 files uploaded

All timelines uploaded successfully
```

### Upload specific timelines
```bash
ivaldi upload --timelines main,feature-x
# Only uploads specified timelines
```

## Performance Features

### Concurrent Downloads (8x Faster)
- Automatic for all sync operations
- 8 parallel workers by default
- Progress bar shows real-time status

### Smart Change Detection
- Only syncs modified files
- Skips binaries and large files
- Per-timeline change tracking

### Local Change Preservation
- Automatically saves uncommitted work
- Restores after sync completion
- Never loses local modifications

## Common Workflows

### Initial Clone with All Branches
```bash
# Option 1: Mirror with full Git history
ivaldi mirror https://github.com/team/project.git
ivaldi sync origin --all  # Get all branches

# Option 2: Download current state
ivaldi download https://github.com/team/project.git
ivaldi sync origin --all  # Get all branches
```

### Keep Multiple Features in Sync
```bash
# Working on feature-auth timeline
ivaldi timeline switch feature-auth
# ... make changes ...

# Sync all timelines before starting new work
ivaldi sync origin --all

# Now all timelines are up to date
ivaldi timeline list
# → main ✓, develop ✓, feature-auth ✓ (current)
```

### Team Collaboration Pattern
```bash
# Every morning, sync all team branches
ivaldi sync origin --all

# Work on your feature
ivaldi timeline switch my-feature
# ... develop ...

# Upload your branch
ivaldi upload

# Before merging, sync target branch
ivaldi sync origin --timeline main
ivaldi timeline switch main
ivaldi fuse my-feature
```

### Selective Team Sync
```bash
# Only sync branches you care about
ivaldi sync origin --timelines main,develop,sarah-feature,john-bugfix

# Ignore other branches like experiments, old releases, etc.
```

## Troubleshooting

### Check sync status
```bash
ivaldi status --remote
# Shows local vs remote status for all timelines
```

### Force resync
```bash
ivaldi sync origin --timeline main --force
# Overwrites local timeline with remote version
```

### Clear upload state (if corrupted)
```bash
rm .ivaldi/upload_state/*.json
# Removes all upload state tracking files
```

### Debug sync issues
```bash
ivaldi sync origin --verbose
# Shows detailed sync information
```

## Configuration

### Set default sync behavior
```bash
# In .ivaldi/config.json
{
  "sync": {
    "concurrent_workers": 8,
    "auto_preserve_local": true,
    "default_strategy": "automatic"
  }
}
```

### Exclude files from sync
```bash
# In .ivaldiignore
build/
*.log
*.tmp
node_modules/
.env
```

## Tips & Best Practices

1. **Use `--all` for team projects**: Keep all branches in sync
2. **Scout before sync**: Use `ivaldi scout` to see what's available
3. **Sync before merge**: Always sync target timeline before fusing
4. **Upload after features**: Push your timeline after completing features
5. **Monitor progress**: Watch the progress bars for large syncs

## Quick Commands

```bash
# Discovery
ivaldi scout origin                         # List all remote timelines

# Sync
ivaldi sync origin                          # Sync current timeline
ivaldi sync origin --all                    # Sync all timelines
ivaldi sync origin --timelines a,b,c        # Sync specific timelines

# Upload
ivaldi upload                               # Upload current timeline
ivaldi upload --all                         # Upload all timelines
ivaldi upload --timelines a,b,c             # Upload specific timelines

# Status
ivaldi status --remote                      # Compare local vs remote
```