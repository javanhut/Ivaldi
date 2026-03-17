# Logging Module (`logging.rs`)

Structured leveled logging for Ivaldi VCS.

## Levels
- **Error**: always shown (via `logging::error()`)
- **Warn**: shown with `-v` (via `logging::warn()`)
- **Info**: shown with `-v` (via `logging::info()`)
- **Debug**: shown with `-vv` (via `logging::debug()`)

## Quiet Mode
`-q` suppresses everything except errors.

## Usage
```bash
ivaldi -v status      # shows info messages
ivaldi -vv download   # shows debug messages
ivaldi -q seal "msg"  # errors only
```
