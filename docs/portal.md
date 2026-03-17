# Portal Module (`portal.rs`)

Remote repository connection management for Ivaldi VCS.

## Overview

Portals represent connections to remote repositories (GitHub, GitLab, self-hosted). Format: `owner/repo`.

## Usage

```rust
use ivaldi::portal::{Portal, PortalManager, Platform};

let mgr = PortalManager::new(&ivaldi_dir);

// Add portal
let portal = Portal::parse("javanhut/IvaldiVCS").unwrap();
mgr.add(&portal)?;

// GitLab with custom URL
let gl = Portal::parse("team/project").unwrap()
    .with_platform(Platform::GitLab)
    .with_base_url("https://gitlab.internal.com");
mgr.add(&gl)?;

// List, get default, get specific
let portals = mgr.list()?;
let default = mgr.get_default()?;
let specific = mgr.get("owner/repo")?;

// Remove
mgr.remove("owner/repo")?;
```

## Storage

`.ivaldi/portals` — one portal per line:
```
github javanhut/IvaldiVCS
gitlab team/project https://gitlab.internal.com
```
