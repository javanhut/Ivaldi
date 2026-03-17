# Sync Module (`sync.rs`)

Download, upload, scout, and harvest operations for Ivaldi VCS.

## Overview

Bridges Ivaldi's BLAKE3-based internal storage with GitHub's SHA1-based Git objects. SHA1 is used ONLY for API communication — never in the internal pipeline.

## Commands

### Download (Clone)
```bash
ivaldi download owner/repo [directory]
```
1. Gets repo info and default branch
2. Fetches recursive tree
3. Downloads all blob files via raw.githubusercontent.com
4. Stores in CAS with BLAKE3 hashing
5. Creates SHA1↔BLAKE3 mappings
6. Writes files to working directory
7. Creates initial commit in Ivaldi

### Upload (Push)
```bash
ivaldi upload [branch] [--force]
```
1. Reads head commit tree
2. Creates blobs on GitHub (base64 upload)
3. Creates Git tree from blob SHAs
4. Creates Git commit pointing to tree
5. Updates branch reference (or creates new branch)

### Scout
```bash
ivaldi scout
```
Lists remote branches — metadata only, no data downloaded.

### Harvest
```bash
ivaldi harvest branch-a branch-b
```
Downloads specific branches into CAS and creates local timelines.

## Force Push Safety
```bash
ivaldi upload --force
# Prompts: "Type 'force push' to confirm:"
```
