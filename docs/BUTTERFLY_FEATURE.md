# Ivaldi Butterfly Feature

The Butterfly feature enables you to create divergent variations (variants) of the same timeline, allowing you to experiment with different approaches while keeping them logically grouped under the same base timeline.

## Overview

Butterfly timelines allow you to:
- Create multiple variants of the same timeline for experimentation
- Switch seamlessly between variants with automatic work preservation
- Track upload status for each variant independently
- Delete variants safely with confirmation prompts

## Key Concepts

- **Base Timeline**: The original timeline (e.g., `feature_auth`)
- **Butterfly Variant**: A divergent copy with unique identifier (e.g., `feature_auth:diverged:jwt_approach`)
- **Auto-numbered Variants**: System-generated IDs (e.g., `feature_auth:diverged:1`, `feature_auth:diverged:2`)
- **Named Variants**: Custom identifiers (e.g., `feature_auth:diverged:oauth_approach`)

## Command Reference

### Main Commands (with aliases)

```bash
# All of these are equivalent - use whichever feels natural:
ivaldi butterfly      # Primary command
ivaldi bf             # Short alias
ivaldi variant        # Descriptive alias
```

### Creating Variants

```bash
# Create auto-numbered variant (creates :diverged:1, :diverged:2, etc.)
ivaldi bf
ivaldi butterfly
ivaldi variant

# Create named variant
ivaldi bf jwt_approach
ivaldi butterfly oauth_flow  
ivaldi variant new_feature

# Explicit create command
ivaldi bf create experiment
ivaldi variant new testing
```

### Listing Variants

```bash
# List all variants for current base timeline
ivaldi bf list
ivaldi butterfly ls
ivaldi variant show

# Example output:
# Base timeline: feature_auth
# 
# Variants:
#   feature_auth                        (base)
# * feature_auth:diverged:1             (active)  
#   feature_auth:diverged:jwt_approach
#   feature_auth:diverged:oauth_flow
```

### Switching Between Variants

```bash
# Switch to variant by identifier
ivaldi bf 1
ivaldi butterfly jwt_approach
ivaldi variant oauth_flow

# Explicit switch command
ivaldi bf switch 1
ivaldi variant to jwt_approach

# Return to base timeline
ivaldi timeline source
```

### Deleting Variants

```bash
# Delete variant (with safety checks)
ivaldi bf delete 1
ivaldi butterfly remove jwt_approach
ivaldi variant rm oauth_flow

# Force delete (bypass safety checks)
ivaldi bf delete old_test --force
ivaldi variant rm experiment --force
```

### Upload Status Tracking

```bash
# Show upload status for all variants
ivaldi bf upload-status
ivaldi butterfly uploads
ivaldi variant up-status

# Show detailed status for specific variant
ivaldi bf uploads 1
ivaldi butterfly upload-status jwt_approach
```

## Workflow Examples

### Basic Experimentation Workflow

```bash
# 1. Start with a timeline
ivaldi timeline create auth_system
# (auth_system)

# 2. Create first approach
ivaldi bf jwt_approach
# (auth_system:diverged:jwt_approach)

# 3. Work on JWT implementation
echo "JWT implementation" > auth.js
ivaldi gather auth.js
ivaldi seal -m "Implement JWT authentication"

# 4. Try different approach
ivaldi bf oauth_flow
# (auth_system:diverged:oauth_flow)
# Previous work auto-shelved and this variant's work restored

# 5. Work on OAuth approach
echo "OAuth implementation" > auth.js
ivaldi gather auth.js  
ivaldi seal -m "Implement OAuth authentication"

# 6. Upload OAuth variant
ivaldi upload
# Uploads auth_system:diverged:oauth_flow to origin

# 7. Check which variants have been uploaded
ivaldi bf uploads
# auth_system                        (base)    [never uploaded]
# auth_system:diverged:jwt_approach             [never uploaded]  
# auth_system:diverged:oauth_flow    (active)  [uploaded 2 min ago to origin]

# 8. Switch back to JWT approach
ivaldi bf jwt_approach
# (auth_system:diverged:jwt_approach)
# Auto-restores shelved JWT work

# 9. Complete and upload JWT approach
ivaldi seal -m "Complete JWT implementation"
ivaldi upload

# 10. Compare variants and choose the best one
ivaldi bf list
ivaldi bf oauth_flow  # Switch to chosen approach
ivaldi timeline source  # Return to base timeline

# 11. Clean up unused variants
ivaldi bf delete jwt_approach
ivaldi bf delete unused_experiment --force
```

### Auto-numbered Workflow

```bash
# Quick experimentation with auto-numbered variants
ivaldi timeline create feature_x

# Create first variant (automatically becomes :diverged:1)
ivaldi bf
# (feature_x:diverged:1)

# Try something
echo "approach 1" > code.js
ivaldi seal -m "First approach"

# Create second variant (automatically becomes :diverged:2)  
ivaldi bf
# (feature_x:diverged:2)

# Try different approach
echo "approach 2" > code.js
ivaldi seal -m "Second approach"

# List variants
ivaldi bf list
# Base timeline: feature_x
# Variants:
#   feature_x                 (base)
#   feature_x:diverged:1      
# * feature_x:diverged:2      (active)

# Switch between numbered variants
ivaldi bf 1
ivaldi bf 2
```

## State Management

### Auto-Shelving

When switching between variants, Ivaldi automatically:
- Shelves any uncommitted changes from the current variant
- Switches to the target variant  
- Restores any previously shelved changes for the target variant

This ensures no work is ever lost when experimenting across variants.

### State Isolation

Each variant maintains completely separate:
- Working directory contents
- Staged files (anvil state)
- Commit history
- Upload tracking

### Upload Tracking

Each variant tracks:
- Last successful upload timestamp
- Portal uploaded to
- Upload success/failure status
- Complete upload history (last 50 records)
- Associated commit/seal information

## Safety Features

### Deletion Protection

- Cannot delete the currently active variant
- Warning for variants with uncommitted changes
- Warning for variants with unpushed commits
- `--force` flag to override safety checks

### Error Recovery

- Auto-shelving uses crash-safe operations
- Incomplete variant switches are recoverable
- State validation on load prevents corruption

## Integration

### Timeline Commands

Butterfly variants work seamlessly with existing timeline commands:

```bash
# Timeline source returns to base from any variant
ivaldi timeline source

# Timeline list shows butterfly variants
ivaldi timeline list

# Status shows current variant
ivaldi status
```

### Upload Commands

Standard upload commands work with variants:

```bash
# Upload current variant to origin (default behavior)
ivaldi upload

# Upload to specific portal
ivaldi upload upstream

# Sync commands work normally
ivaldi sync origin
```

## Advanced Usage

### Mixed Workflows

You can mix butterfly variants with regular timelines:

```bash
# Create regular timeline
ivaldi timeline create hotfix

# Create butterfly variants within it
ivaldi bf critical_fix
ivaldi bf alternative_fix

# Switch between regular timelines and variants
ivaldi timeline switch main
ivaldi bf oauth_approach
ivaldi timeline switch hotfix
```

### Collaboration

When collaborating:
- Upload variants using standard `ivaldi upload` command
- Each variant becomes a separate branch on the remote
- Team members can work on different variants simultaneously
- Use descriptive variant names for clarity

```bash
# Good variant names for collaboration
ivaldi bf sarah_authentication_approach
ivaldi bf mike_caching_optimization  
ivaldi bf team_performance_fix
```

## Best Practices

1. **Use Descriptive Names**: `jwt_auth` instead of `test1`
2. **Clean Up Regularly**: Delete unused variants to keep workspace tidy
3. **Upload Frequently**: Keep team synchronized with variant progress
4. **Document Approaches**: Use clear commit messages explaining variant purpose
5. **Return to Base**: Use `ivaldi timeline source` when variant work is complete

## Troubleshooting

### Common Issues

**Variant not found**:
```bash
ivaldi bf list  # See available variants
```

**Cannot delete active variant**:
```bash
ivaldi bf 1     # Switch to different variant first
ivaldi bf delete old_variant
```

**Auto-shelve failed**:
```bash
# Manual commit before switching
ivaldi gather .
ivaldi seal -m "WIP: save current progress" 
ivaldi bf switch target_variant
```

**Upload tracking missing**:
```bash
# Upload tracking starts after first upload
ivaldi upload
ivaldi bf uploads  # Now shows status
```

The butterfly feature makes experimentation safe, organized, and trackable - helping you explore different approaches without fear of losing work or cluttering your timeline history.