# Changelog

All notable changes to Ivaldi will be documented in this file.

## [Unreleased] - 2024

### Added

#### Butterfly Timeline Variants System 🦋
- Complete butterfly sub-timeline system for safe experimentation
- Create variants with `ivaldi bf`, `ivaldi butterfly`, or `ivaldi variant`
- Auto-numbered variants (`:diverged:1`, `:diverged:2`) and named variants (`:diverged:jwt_approach`)
- Automatic work shelving when switching between variants - never lose progress
- Independent state management per variant (workspace, commits, uploads)
- Upload tracking per variant with detailed history (`ivaldi bf upload-status`)
- Safe variant deletion with confirmation prompts (`ivaldi bf delete <name>`)
- Multiple command aliases for natural usage patterns
- Complete integration with existing timeline commands (`ivaldi timeline source`)
- Comprehensive test suite ensuring reliability

#### Concurrent Sync Performance (8x Faster)
- Implemented worker pool pattern with 8 concurrent workers for file downloads
- Added `ConcurrentDownloader` struct for parallel download operations
- Real-time progress tracking with visual progress bars
- Thread-safe download counter with mutex protection
- Adaptive worker count based on repository size

#### Remote Timeline Discovery
- New `ListRemoteTimelines()` function to discover all remote branches
- Support for GitHub, GitLab, and native Ivaldi repositories
- Returns timeline names and commit SHAs for validation
- Enables bulk and selective timeline synchronization

#### Bulk Timeline Synchronization
- `SyncAllTimelines()` - Sync all remote timelines automatically
- `SyncSelectedTimelines()` - Sync specific timelines by name
- Error resilience - continues on individual timeline failures
- Detailed per-timeline sync results and error reporting

#### Multiple Timeline Upload Support
- Per-timeline upload state tracking (`owner_repo_timeline.json`)
- Independent upload states prevent timeline conflicts
- `Timeline` field added to `UploadState` struct
- Support for `ivaldi upload --all` to upload all local timelines

#### Smart Change Detection
- `saveSelectedFiles()` optimized file scanning
- Skip binary files and files >1MB during local preservation
- Focus on common source code file extensions
- Glob pattern matching for efficient file discovery

### Fixed

#### Timeline Upload Conflicts
- Fixed issue where multiple timelines shared upload state
- Each timeline now maintains independent upload tracking
- Proper branch creation for new timelines on GitHub
- Better error messages for timeline-specific operations

#### Sync Data Loss Prevention
- Local changes now properly preserved during sync
- Three-way merge support for conflict resolution
- Automatic restoration of local changes after sync
- Working directory properly updated after timeline merge

### Enhanced

#### Timeline Name Validation
- `validateTimelineName()` ensures Git branch compatibility
- Checks for invalid characters, length limits, and patterns
- Prevents problematic timeline names before creation
- Clear error messages for validation failures

#### Error Handling
- Improved error messages with timeline context
- Better detection of "branch doesn't exist" scenarios
- Graceful handling of missing or corrupted state files
- Warning messages for non-critical failures

#### GitHub API Integration
- Optimized tree downloads with concurrent blob fetching
- Better authentication token handling
- Support for large repository downloads via tarball
- Automatic branch creation for new timelines

### Performance Improvements

- **8x faster sync** for large repositories with concurrent downloads
- **Reduced memory usage** with streaming file downloads
- **Optimized file scanning** skips binaries and large files
- **Batch API operations** minimize round trips to GitHub
- **Smart caching** with 15-minute cache for repeated operations

### Documentation

- Comprehensive sync architecture documentation
- Updated README with new sync features and examples
- API reference for new sync functions
- Troubleshooting guide for common sync issues

## [Previous Versions]

### Shell Integration
- Oh My Zsh plugin for timeline info in prompt
- Automatic installation scripts
- Command aliases for common operations

### GitHub Direct Integration
- REST API integration without git dependency
- Automatic branch creation and management
- Support for .ivaldiignore files

### Timeline Management
- Human-friendly commit names (e.g., `bright-river-42`)
- Natural language navigation (`jump to "yesterday"`)
- Timeline renaming with `--to` syntax
- Automatic work preservation on timeline switch

### Core Features
- Workshop metaphor commands (forge, gather, seal, fuse)
- Content-addressed storage system
- Mathematical impossibility of data loss
- Smart conflict resolution