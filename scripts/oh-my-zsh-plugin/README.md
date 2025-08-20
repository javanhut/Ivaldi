# Ivaldi Plugin for Oh My Zsh

This plugin adds Ivaldi timeline information to your Oh My Zsh prompt, similar to how Git branch information is displayed.

## What You Get

### Timeline Information
Your prompt will show both Git and Ivaldi information:
```bash
➜  myproject git:(main) ivaldi:(feature-timeline) 
```

### Useful Aliases
The plugin provides many convenient aliases:
- `iva` → `ivaldi`
- `igather` → `ivaldi gather`
- `iseal` → `ivaldi seal`
- `istatus` → `ivaldi status`
- `itimeline` → `ivaldi timeline`
- `iswitch` → `ivaldi timeline switch`
- `icreate` → `ivaldi timeline create`
- `imirror` → `ivaldi mirror`
- `idownload` → `ivaldi download`

Plus Git-style shortcuts:
- `iadd` → `ivaldi gather`
- `icommit` → `ivaldi seal`
- `ibranch` → `ivaldi timeline`
- `icheckout` → `ivaldi timeline switch`

## Installation

### Option 1: Manual Installation (Recommended)

1. **Create the plugin directory:**
   ```bash
   mkdir -p $ZSH_CUSTOM/plugins/ivaldi
   ```

2. **Copy the plugin file:**
   ```bash
   cp ivaldi.plugin.zsh $ZSH_CUSTOM/plugins/ivaldi/
   ```

3. **Add the plugin to your ~/.zshrc:**
   ```bash
   plugins=(git ivaldi)
   ```

4. **Reload your shell:**
   ```bash
   source ~/.zshrc
   ```

### Option 2: Use the Modified Theme

If you want timeline information directly in your prompt (like the example above):

1. **Copy the theme:**
   ```bash
   cp robbyrussell-ivaldi.zsh-theme $ZSH_CUSTOM/themes/
   ```

2. **Update your ~/.zshrc:**
   ```bash
   ZSH_THEME="robbyrussell-ivaldi"
   ```

3. **Add the plugin:**
   ```bash
   plugins=(git ivaldi)
   ```

4. **Reload your shell:**
   ```bash
   source ~/.zshrc
   ```

### Option 3: Add to Existing Theme

If you want to keep your current theme but add Ivaldi info:

1. **Install the plugin** (steps 1-4 from Option 1)

2. **Add timeline info to your existing theme:**
   
   Find your current theme file in `$ZSH/themes/` or `$ZSH_CUSTOM/themes/` and add `$(ivaldi_prompt_info)` to the PROMPT line.
   
   For example, if using the default robbyrussell theme, change:
   ```bash
   PROMPT='${ret_status} %{$fg[cyan]%}%c%{$reset_color%} $(git_prompt_info)'
   ```
   to:
   ```bash
   PROMPT='${ret_status} %{$fg[cyan]%}%c%{$reset_color%} $(git_prompt_info)$(ivaldi_prompt_info)'
   ```

## Customization

### Changing Timeline Display Format

You can customize how the timeline information appears by setting these variables in your ~/.zshrc:

```bash
# Change the prefix and suffix
ZSH_THEME_IVALDI_PROMPT_PREFIX="[timeline: "
ZSH_THEME_IVALDI_PROMPT_SUFFIX="] "

# Change colors (put this after the Oh My Zsh source line)
ZSH_THEME_IVALDI_PROMPT_PREFIX="%{$fg_bold[green]%}timeline:(%{$fg[cyan]%}"
ZSH_THEME_IVALDI_PROMPT_SUFFIX="%{$fg[green]%})%{$reset_color%} "
```

### Adding Timeline to Right Side (RPROMPT)

If you prefer timeline info on the right side:

```bash
# Add this to your theme or ~/.zshrc
RPROMPT='$(ivaldi_prompt_info)'
```

## Usage Examples

Once installed, your prompt will automatically show timeline information when you're in an Ivaldi repository:

```bash
# Outside Ivaldi repo
➜  ~ 

# In Git repo only
➜  myproject git:(main) 

# In Ivaldi repo (no Git)
➜  myproject ivaldi:(main) 

# In repo with both Git and Ivaldi
➜  myproject git:(main) ivaldi:(feature-timeline) 

# Use the convenient aliases
➜  myproject git:(main) ivaldi:(main) igather .
➜  myproject git:(main) ivaldi:(main) iseal "Add new feature"
➜  myproject git:(main) ivaldi:(main) iswitch feature-timeline
➜  myproject git:(main) ivaldi:(feature-timeline) 
```

## Troubleshooting

### Timeline shows as "unknown"
- Make sure you're in an Ivaldi repository
- Check that `.ivaldi/timelines/config.json` exists
- Try running `ivaldi status` to verify the repository

### Plugin not loading
- Verify the plugin is in the correct directory: `$ZSH_CUSTOM/plugins/ivaldi/ivaldi.plugin.zsh`
- Make sure `ivaldi` is in your plugins list in ~/.zshrc
- Restart your terminal or run `source ~/.zshrc`

### Slow prompt
- Install `jq` for faster JSON parsing: `brew install jq` (macOS) or `sudo apt install jq` (Linux)
- The plugin is optimized to be fast, but very large repositories might see slight delays

## Advanced Configuration

### Conditional Display
Only show timeline when it's not "main":

```bash
function ivaldi_prompt_info_conditional() {
    local timeline=$(ivaldi_timeline_info)
    if [[ -n "$timeline" && "$timeline" != "main" ]]; then
        echo "$ZSH_THEME_IVALDI_PROMPT_PREFIX$timeline$ZSH_THEME_IVALDI_PROMPT_SUFFIX"
    fi
}

# Use in your PROMPT
PROMPT='${ret_status} %{$fg[cyan]%}%c%{$reset_color%} $(git_prompt_info)$(ivaldi_prompt_info_conditional)'
```

### Show Timeline Status
For more detailed information (requires customization):

```bash
function ivaldi_detailed_info() {
    local timeline=$(ivaldi_timeline_info)
    if [[ -n "$timeline" ]]; then
        # You could add file counts, dirty status, etc.
        echo "$ZSH_THEME_IVALDI_PROMPT_PREFIX$timeline$ZSH_THEME_IVALDI_PROMPT_SUFFIX"
    fi
}
```