# Ivaldi Jump Improvements Test Guide

## New Features Implemented

### 1. Workspace-Preserving Jump
The jump functionality now supports preserving uncommitted changes during jumps.

**Usage:**
```bash
# Jump while preserving workspace changes
ivaldi jump --preserve prev-seal-001
ivaldi jump -p bright-river-42

# Enhanced CLI with natural language
ivaldi enhanced jump --preserve "yesterday at 3pm"
```

### 2. Jump History
Ivaldi now maintains a jump history that allows you to return to previous positions.

**Usage:**
```bash
# Regular jump (automatically saves history)
ivaldi jump prev-seal-001

# Jump back to previous position
ivaldi jump back
# or
ivaldi enhanced jump back
```

### 3. Force Jump
Override local changes when jumping.

**Usage:**
```bash
# Force jump even if it overwrites local changes
ivaldi jump --force prev-seal-001
ivaldi jump -f bright-river-42
```

### 4. Advanced Options
```bash
# Don't save current position to jump history
ivaldi jump --no-history prev-seal-001

# Combine options
ivaldi jump --preserve --no-history prev-seal-001
```

## Key Improvements

1. **Workspace Preservation**: The `--preserve` flag keeps uncommitted changes during jumps
2. **Jump History**: Automatic tracking of jump positions with `jump back` functionality
3. **Selective Restoration**: Only overwrite files that exist in the target seal when preserving
4. **Workspace Shelving**: Automatic shelving and unshelving of workspace state
5. **Enhanced CLI Integration**: Full support in both regular and enhanced CLI modes

## File Changes Made

1. `forge/repository.go`: Added new jump methods and workspace preservation logic
2. `forge/enhanced.go`: Enhanced repository jump methods with options
3. `ui/cli/commands.go`: Updated regular CLI with new jump flags and commands
4. `ui/enhanced_cli/cli.go`: Enhanced CLI with advanced jump functionality

## Technical Details

- **Jump History**: Stored in `.ivaldi/jump_history.json` (max 10 entries)
- **Workspace Shelving**: Stored in `.ivaldi/jump_shelves/` directory
- **Preservation Logic**: Selectively restores files while keeping uncommitted changes
- **Timeline Awareness**: Maintains timeline context during jumps

## Testing Workflow

1. Create a repository and make some changes
2. Seal the changes: `ivaldi seal "initial version"`
3. Make more uncommitted changes
4. Jump with preservation: `ivaldi jump --preserve prev-seal-001`
5. Verify uncommitted changes are preserved
6. Jump back: `ivaldi jump back`
7. Verify original state is restored

This implementation addresses the core issues:
- ✅ Workspace preservation during seal jumps
- ✅ Timeline context maintenance
- ✅ Git checkout-like behavior
- ✅ Safe restoration of previous states