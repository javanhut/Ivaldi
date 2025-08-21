# Ivaldi Sync Architecture

## Overview

Ivaldi's sync system provides high-performance, resilient synchronization between local and remote repositories. This document details the architecture, optimizations, and capabilities of the sync subsystem.

## Core Components

### 1. Network Manager (`core/network/network.go`)

The NetworkManager handles all remote repository interactions:

- **GitHub API Integration**: Direct REST API communication without git dependency
- **Concurrent Downloads**: Worker pool pattern with 8 parallel workers
- **Progress Tracking**: Real-time download progress with visual indicators
- **Multi-Platform Support**: GitHub, GitLab, and native Ivaldi protocols

### 2. Sync Manager (`core/sync/sync.go`)

The SyncManager coordinates synchronization operations:

- **Local Change Preservation**: Automatic saving and restoration of uncommitted work
- **Timeline Management**: Creation and management of temporary timelines for merging
- **Conflict Resolution**: Smart merging strategies with multiple options
- **Bulk Operations**: Support for syncing multiple timelines in one operation

### 3. Fuse Manager (`core/fuse/fuse.go`)

The FuseManager handles timeline merging:

- **Merge Strategies**: Fast-forward, automatic, manual, and squash
- **Conflict Detection**: Identifies and reports merge conflicts
- **Timeline Cleanup**: Automatic cleanup of temporary timelines

## Key Features

### Concurrent File Downloads

```go
type ConcurrentDownloader struct {
    networkMgr   *NetworkManager
    owner        string
    repo         string
    workerCount  int
    progress     *downloadProgress
}
```

- **Worker Pool**: 8 concurrent workers for parallel file downloads
- **Progress Tracking**: Thread-safe progress updates with mutex protection
- **Error Resilience**: Individual file failures don't stop the entire operation
- **Adaptive Performance**: Automatically adjusts based on repository size

### Per-Timeline Upload State

```go
type UploadState struct {
    Timeline         string            `json:"timeline"`
    LastUploadedSeal string            `json:"last_uploaded_seal"`
    LastUploadTime   time.Time         `json:"last_upload_time"`
    FileHashes       map[string]string `json:"file_hashes"`
}
```

- **Independent Tracking**: Each timeline maintains its own upload state
- **Smart Change Detection**: Only uploads files that have actually changed
- **State Persistence**: Upload states saved as `owner_repo_timeline.json`
- **Conflict Prevention**: Eliminates timeline upload conflicts

### Remote Timeline Discovery

```go
func (nm *NetworkManager) ListRemoteTimelines(portalURL string) ([]RemoteRef, error)
```

- **Automatic Discovery**: Finds all branches/timelines on remote repository
- **Multi-Platform**: Works with GitHub, GitLab, and native Ivaldi repos
- **Metadata Retrieval**: Returns timeline names and commit SHAs
- **Validation Support**: Verifies timeline existence before operations

### Bulk Synchronization

```go
func (sm *SyncManager) SyncAllTimelines(portalURL string) (*SyncAllResult, error)
func (sm *SyncManager) SyncSelectedTimelines(portalURL string, timelineNames []string) (*SyncAllResult, error)
```

- **All Timelines**: Sync every remote timeline automatically
- **Selective Sync**: Choose specific timelines to synchronize
- **Error Resilience**: Continue on individual timeline failures
- **Detailed Reporting**: Complete status for each timeline

## Sync Process Flow

### 1. Preparation Phase
```
saveLocalChanges() → saveSelectedFiles()
```
- Save uncommitted work to temporary storage
- Optimized scanning (only common file types)
- Skip binaries and large files (>1MB)

### 2. Fetch Phase
```
FetchFromPortal() → fetchFromGitHub()
```
- Discover remote timeline commit SHA
- Download commit metadata
- Concurrent file downloads with progress

### 3. Integration Phase
```
Create temporary timeline → Store fetched seals → Update timeline head
```
- Create `temp-remote-{timestamp}` timeline
- Store remote changes in object storage
- Update timeline pointers

### 4. Merge Phase
```
Fuse remote timeline → Resolve conflicts → Update working directory
```
- Determine merge strategy (fast-forward, automatic, manual)
- Execute fuse operation
- Extract merged tree to working directory

### 5. Restoration Phase
```
restoreLocalChanges() → Cleanup temporary timeline
```
- Restore saved local changes
- Three-way merge if needed
- Clean up temporary resources

## Performance Optimizations

### 1. Concurrent Downloads
- **8x Faster**: Parallel downloads vs sequential
- **Worker Pool**: Efficient resource utilization
- **Progress Tracking**: Real-time feedback

### 2. Smart File Filtering
```go
searchPaths := []string{
    "*.go", "*.js", "*.ts", "*.py", "*.java",
    "src/*.go", "lib/*.go", "cmd/*.go",
}
```
- Skip binary files
- Ignore files >1MB
- Focus on source code files

### 3. Incremental Uploads
- Only upload changed files
- Per-timeline state tracking
- Content-based change detection

### 4. Batch Operations
- Upload multiple files in single API call
- Create Git tree objects in batch
- Minimize API round trips

## Error Handling

### Timeline Validation
```go
func (nm *NetworkManager) validateTimelineName(timeline string) error
```
- Check for invalid Git branch characters
- Enforce length limits (255 chars)
- Prevent problematic names

### Resilient Sync
- Individual timeline failures logged
- Continue processing remaining timelines
- Detailed error reporting per timeline

### Local Change Safety
- Automatic preservation before sync
- Restoration after sync completion
- Conflict detection and reporting

## API Reference

### Sync Operations

```go
// Single timeline sync
Sync(portalURL string, opts SyncOptions) (*SyncResult, error)

// Sync all remote timelines
SyncAllTimelines(portalURL string) (*SyncAllResult, error)

// Sync selected timelines
SyncSelectedTimelines(portalURL string, timelineNames []string) (*SyncAllResult, error)
```

### Network Operations

```go
// Discover remote timelines
ListRemoteTimelines(portalURL string) ([]RemoteRef, error)

// Fetch from remote
FetchFromPortal(portalURL, timeline string) (*FetchResult, error)

// Upload to remote
UploadToPortal(portalURL, timeline string, seals []*objects.Seal) error
```

### Timeline Management

```go
// Create timeline
Create(name, description string) error

// Switch timeline
Switch(name string) error

// Update timeline head
UpdateHead(timeline string, hash objects.Hash) error
```

## Configuration

### Worker Count
Default: 8 workers for concurrent downloads
Adjustable via `getWorkerCount()` method

### File Size Limits
- Skip files >1MB during local change preservation
- No limit on remote file downloads

### Timeout Settings
- HTTP client timeout: 30 seconds
- Can be adjusted in NetworkManager initialization

## Best Practices

1. **Use Bulk Sync for Multiple Timelines**
   - More efficient than individual syncs
   - Better error handling and reporting

2. **Validate Timeline Names**
   - Ensure Git compatibility before creation
   - Use validation function for user input

3. **Monitor Progress**
   - Use progress callbacks for long operations
   - Provide user feedback during sync

4. **Handle Errors Gracefully**
   - Check SyncResult for partial failures
   - Log detailed error information

5. **Optimize for Large Repositories**
   - Use concurrent downloads
   - Enable smart file filtering
   - Consider selective timeline sync

## Future Enhancements

- [ ] Configurable worker count
- [ ] Resumable downloads
- [ ] Delta sync for large files
- [ ] Conflict resolution UI
- [ ] Sync scheduling/automation
- [ ] Bandwidth throttling
- [ ] Offline sync queue

## Troubleshooting

### Slow Sync Performance
- Check network connectivity
- Verify GitHub API rate limits
- Consider selective timeline sync

### Upload Conflicts
- Each timeline has independent state
- Check `.ivaldi/upload_state/` directory
- Clear state files if corrupted

### Missing Timelines
- Use `ListRemoteTimelines()` to discover
- Verify timeline names are Git-compatible
- Check authentication credentials

### Large Repository Issues
- Use selective sync for specific timelines
- Enable file filtering optimizations
- Consider increasing timeout values