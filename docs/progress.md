# Progress Module (`progress.rs`)

Progress bars and spinners for Ivaldi VCS, using `indicatif`.

## Components
- `file_bar(total, action)` — bar for download/upload file counts
- `commit_bar(total)` — bar for commit processing
- `byte_bar(total, action)` — bar with byte counts and speed
- `spinner(message)` — indeterminate spinner for waiting operations

## Visual Examples
```
⠋ Downloading [█████████████░░░░░░░░░░░░░░░░░] 45/100 (2s)
⠋ Uploading [██████████████████░░░░░░░░░░░░] 67/100 (1s)
⠋ Processing commits [████████████████░░░░░░░░░░░░░░] 8/10
⠋ Waiting for authentication...
```
