# Color Module (`color.rs`)

ANSI colored terminal output for Ivaldi VCS.

## Features
- Uses ANSI escape codes directly (no extra dependency)
- Respects `NO_COLOR` environment variable
- Auto-detects non-terminal output (pipes)
- Semantic color functions: `seal_name()`, `hash()`, `timeline()`, `author()`, `status_label()`

## Usage
Colors are applied automatically in CLI output. Disable with `NO_COLOR=1` or by piping.
