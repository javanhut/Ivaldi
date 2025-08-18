# Ivaldi Config Command

The `ivaldi config` command provides interactive setup and management of GitHub credentials for seamless integration.

## Overview

Ivaldi requires GitHub Personal Access Tokens to interact with GitHub repositories. The config command provides a secure, interactive way to set up and manage these credentials.

## Usage

### Interactive Setup
```bash
ivaldi config
```

This will prompt you for:
1. **GitHub Username**: Your GitHub username
2. **GitHub Token**: Your Personal Access Token

### Example Session
```bash
$ ivaldi config
→ Enter GitHub username: your-username
→ Enter GitHub token: ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxx
→ Validating GitHub credentials...
→ ✅ GitHub credentials configured successfully
→ ✅ Token validated with GitHub API
```

### Reset Credentials
```bash
ivaldi config --reset
```

This will remove stored credentials, requiring you to run `ivaldi config` again.

## GitHub Token Setup

### Creating a Personal Access Token

1. Go to GitHub Settings → Developer settings → Personal access tokens
2. Click "Generate new token (classic)"
3. Select these scopes:
   - `repo` (Full control of private repositories)
   - `workflow` (Update GitHub Action workflows)
4. Copy the generated token (starts with `ghp_`)

### Required Permissions

The token needs these permissions for full functionality:
- **repo**: Read/write access to repositories
- **contents**: Create/update repository contents
- **metadata**: Read repository metadata

## Security

### Storage Location
Credentials are stored in `.ivaldi/config.json` with restricted permissions (0600 - owner read/write only).

### Validation
Every token is validated against GitHub's API before being stored:
```bash
→ Validating GitHub credentials...
→ Testing API access to github.com...
→ ✅ Token validated successfully
```

### Best Practices
- Use tokens with minimal required permissions
- Set expiration dates on tokens
- Rotate tokens regularly
- Never share tokens or commit them to repositories

## Configuration File

The config file is stored at `.ivaldi/config.json`:
```json
{
  "github_username": "your-username",
  "github_token": "ghp_xxxxxxxxxxxxxxxxxxxxxxxxxxxx"
}
```

## Troubleshooting

### Invalid Token Error
```bash
→ ❌ Invalid GitHub token or insufficient permissions
```
**Solution**: Generate a new token with correct permissions.

### Network Error
```bash
→ ❌ Failed to validate token: network error
```
**Solution**: Check internet connection and GitHub API status.

### Permission Denied
```bash
→ ❌ Permission denied writing config file
```
**Solution**: Ensure you have write permissions in the repository directory.

## Integration with Other Commands

Once configured, credentials are automatically used by:
- `ivaldi sync` - For GitHub synchronization
- `ivaldi portal add` - For adding GitHub portals
- `ivaldi squash --force-push` - For force pushing to GitHub

## Command Options

| Option | Description | Example |
|--------|-------------|---------|
| `--reset` | Remove stored credentials | `ivaldi config --reset` |
| `--help` | Show command help | `ivaldi config --help` |

## Examples

### First-time Setup
```bash
# Clone repository
git clone https://github.com/user/repo.git
cd repo

# Initialize Ivaldi
ivaldi forge

# Configure GitHub access
ivaldi config
→ Enter GitHub username: myusername
→ Enter GitHub token: ghp_abc123...
→ ✅ GitHub credentials configured successfully

# Add portal and sync
ivaldi portal add origin https://github.com/user/repo.git
ivaldi sync origin
```

### Updating Credentials
```bash
# Remove old credentials
ivaldi config --reset

# Set new credentials
ivaldi config
→ Enter GitHub username: newusername
→ Enter GitHub token: ghp_xyz789...
→ ✅ GitHub credentials configured successfully
```

## Related Commands

- [`ivaldi sync`](SYNC_COMMAND.md) - Synchronize with GitHub
- [`ivaldi portal`](PORTAL_COMMAND.md) - Manage remote portals
- [`ivaldi squash`](SQUASH_COMMAND.md) - Squash commits