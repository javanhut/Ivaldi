# .ivaldiignore File Support

Ivaldi provides comprehensive file ignore functionality through `.ivaldiignore` files, similar to `.gitignore` but with enhanced pattern matching and built-in intelligent defaults.

## Overview

The `.ivaldiignore` file allows you to specify which files and directories should be excluded from:
- Repository tracking
- Sync operations to GitHub
- Timeline operations
- Backup creation

## File Format

### Basic Syntax
```bash
# Comments start with hash
filename.txt        # Ignore specific file
*.log              # Ignore all .log files
build/             # Ignore entire directory
temp/              # Ignore directory and contents
```

### Advanced Patterns
```bash
# Glob patterns
**/*.tmp           # All .tmp files in any subdirectory
src/**/*.test.js   # All .test.js files under src/
**/node_modules/   # node_modules anywhere in tree

# Negation (exceptions)
*.log              # Ignore all log files
!important.log     # But keep this one

# Directory-specific
logs/              # Ignore logs directory
logs/*.txt         # Ignore .txt files in logs only
```

## Built-in Ignore Patterns

Ivaldi automatically ignores these patterns (cannot be overridden):

### System Files
```bash
.DS_Store          # macOS Finder metadata
Thumbs.db          # Windows image cache
._.DS_Store        # macOS resource forks
```

### VCS Directories
```bash
.ivaldi/           # Ivaldi metadata
.git/              # Git metadata
.svn/              # Subversion metadata
.hg/               # Mercurial metadata
```

### Build Artifacts
```bash
build/             # Build output directory
dist/              # Distribution directory
target/            # Rust/Java build directory
```

### Temporary Files
```bash
*.tmp              # Temporary files
*.temp             # Temporary files
*.bak              # Backup files
*.swp              # Vim swap files
*.swo              # Vim swap files
*~                 # Editor backup files
```

### Log Files
```bash
*.log              # All log files
logs/              # Log directories
```

## Pattern Matching Rules

### Wildcards
| Pattern | Matches | Example |
|---------|---------|---------|
| `*` | Any characters except `/` | `*.txt` matches `file.txt` |
| `**` | Any characters including `/` | `**/*.js` matches `src/app.js` |
| `?` | Single character | `test?.txt` matches `test1.txt` |
| `[abc]` | Character class | `test[123].txt` matches `test1.txt` |

### Directory Patterns
```bash
dir/               # Ignore directory and all contents
dir/*              # Ignore contents but track directory
dir/**             # Ignore all subdirectories recursively
```

### Negation Patterns
```bash
*.log              # Ignore all log files
!debug.log         # Exception: keep debug.log
logs/              # Ignore logs directory
!logs/important/   # Exception: keep important logs
```

## Example .ivaldiignore Files

### Node.js Project
```bash
# Dependencies
node_modules/
npm-debug.log*
yarn-debug.log*
yarn-error.log*

# Build output
build/
dist/
.next/

# Environment files
.env
.env.local
.env.development.local
.env.test.local
.env.production.local

# Runtime data
pids/
*.pid
*.seed
*.pid.lock

# Coverage directory
coverage/
.nyc_output/

# IDE files
.vscode/
.idea/
*.suo
*.ntvs*
*.njsproj
*.sln
*.sw?
```

### Go Project
```bash
# Build artifacts
*.exe
*.exe~
*.dll
*.so
*.dylib

# Test binary
*.test

# Output of the go coverage tool
*.out

# Go workspace file
go.work

# Vendor directory
vendor/

# IDE files
.vscode/
.idea/
*.swp
*.swo

# OS files
.DS_Store
Thumbs.db
```

### Python Project
```bash
# Byte-compiled / optimized files
__pycache__/
*.py[cod]
*$py.class

# Distribution / packaging
.Python
build/
develop-eggs/
dist/
downloads/
eggs/
.eggs/
lib/
lib64/
parts/
sdist/
var/
wheels/
*.egg-info/
.installed.cfg
*.egg

# Virtual environments
venv/
env/
ENV/

# Testing
.tox/
.coverage
.pytest_cache/
.coverage.*

# IDE
.vscode/
.idea/
*.swp
*.swo
```

## Sync Behavior

### Upload Filtering
```bash
ivaldi sync origin
→ Processing 100 files...
→ CHECKMARK Skipped 35 ignored files (.ivaldiignore)
→ CHECKMARK Skipped 12 built-in ignores
→ CHECKMARK Uploaded 53 source files
```

### Ignore File Reporting
```bash
# Verbose sync shows ignored files
ivaldi sync origin --verbose
→ Ignored: build/app.exe (pattern: build/)
→ Ignored: *.log files (pattern: *.log)
→ Ignored: node_modules/ (pattern: node_modules/)
→ Uploading: src/main.go
→ Uploading: README.md
```

## Multiple Ignore Files

### Hierarchy
Ivaldi checks for ignore files in this order:
1. **Global**: `~/.ivaldiignore` (user-wide patterns)
2. **Repository**: `.ivaldiignore` (project-specific patterns)
3. **Directory**: `subdir/.ivaldiignore` (directory-specific patterns)

### Inheritance
Patterns from parent directories apply to subdirectories:
```bash
# Root .ivaldiignore
*.log

# src/.ivaldiignore  
*.tmp

# Result: src/ ignores both *.log and *.tmp files
```

## Advanced Features

### Conditional Patterns
```bash
# Only ignore in specific environments
[development]
.env.development

[production]
.env.production
debug/
```

### Size-based Filtering
```bash
# Ignore files larger than 100MB (GitHub limit)
*.zip
*.tar.gz
*.dmg
*.iso
```

### Time-based Patterns
```bash
# Ignore temporary files older than 1 day
# (requires special Ivaldi syntax)
@age:1d *.tmp
@age:7d logs/
```

## Command Integration

### Refresh Ignore Patterns
```bash
# Force reload of ignore patterns
ivaldi refresh
→ Reloading .ivaldiignore patterns...
→ Found 23 patterns in .ivaldiignore
→ Applied 15 built-in patterns
→ CHECKMARK Ignore patterns refreshed
```

### Test Ignore Patterns
```bash
# Check if file would be ignored
ivaldi check-ignore build/app.exe
→ build/app.exe would be ignored (pattern: build/)

# Test multiple files
ivaldi check-ignore src/main.go build/app.exe *.log
→ src/main.go: not ignored
→ build/app.exe: ignored (build/)
→ debug.log: ignored (*.log)
```

### List Ignored Files
```bash
# Show all ignored files in repository
ivaldi status --ignored
→ Ignored files:
→   build/app.exe (pattern: build/)
→   debug.log (pattern: *.log)
→   node_modules/ (pattern: node_modules/)
```

## Performance Optimization

### Pattern Efficiency
Ivaldi optimizes pattern matching:
```bash
# Fast patterns (use these)
*.txt              # Simple extension
build/             # Directory match
src/**/*.js        # Scoped wildcard

# Slower patterns (avoid if possible)
**/*test*/**       # Complex nested wildcards
*/temp/*/cache/*   # Multiple wildcards
```

### Caching
Ignore pattern results are cached:
```bash
→ Building ignore pattern cache...
→ Cached 1,247 file ignore results
→ Pattern matching: 0.02s (cached)
```

## Troubleshooting

### Pattern Not Working
```bash
# Debug pattern matching
ivaldi check-ignore --debug file.txt
→ Testing against pattern: *.txt
→ Converted to regex: ^.*\.txt$
→ Match result: true
→ file.txt is ignored by pattern: *.txt
```

### Performance Issues
```bash
# Check pattern complexity
ivaldi analyze-ignore
→ .ivaldiignore contains 45 patterns
→ Complex patterns: 3 (may impact performance)
→ Suggestions:
→   Replace **/*test*/** with test/ or **/test/**
```

### File Still Being Tracked
```bash
# Check if file was previously tracked
ivaldi status --show-tracked file.txt
→ file.txt is tracked (added before ignore pattern)
→ Use: ivaldi exclude file.txt (to stop tracking)
```

## Best Practices

### Keep Patterns Simple
```bash
# Good
*.log
build/
node_modules/

# Avoid
**/*debug*/**/*
**/temp/**/cache/**
```

### Use Comments
```bash
# Language-specific ignores
*.pyc              # Python bytecode
*.class            # Java bytecode

# Build artifacts
build/             # Webpack output
dist/              # Distribution files
target/            # Maven/Rust builds
```

### Regular Maintenance
```bash
# Periodically review ignore patterns
ivaldi analyze-ignore
→ 12 patterns never matched any files
→ Consider removing unused patterns for performance
```

### Test Before Committing
```bash
# Check what will be synced
ivaldi sync origin --dry-run
→ Would upload 23 files
→ Would skip 45 ignored files
→ Review looks good? Run: ivaldi sync origin
```

## Related Commands

- [`ivaldi sync`](SYNC_COMMAND.md) - Respect ignore patterns during sync
- [`ivaldi gather`](GATHER_COMMAND.md) - Apply ignore patterns when staging
- [`ivaldi exclude`](EXCLUDE_COMMAND.md) - Add patterns to ignore file
- [`ivaldi refresh`](REFRESH_COMMAND.md) - Reload ignore patterns