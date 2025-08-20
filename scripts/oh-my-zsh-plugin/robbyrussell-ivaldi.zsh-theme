# RobbyRussell theme with Ivaldi timeline support
# Based on the original robbyrussell theme but adds Ivaldi timeline info

local ret_status="%(?:%{$fg_bold[green]%}➜ :%{$fg_bold[red]%}➜ )"
PROMPT='${ret_status} %{$fg[cyan]%}%c%{$reset_color%} $(git_prompt_info)$(ivaldi_prompt_info)'

ZSH_THEME_GIT_PROMPT_PREFIX="%{$fg_bold[blue]%}git:(%{$fg[red]%}"
ZSH_THEME_GIT_PROMPT_SUFFIX="%{$reset_color%} "
ZSH_THEME_GIT_PROMPT_DIRTY="%{$fg[blue]%}) %{$fg[yellow]%}✗"
ZSH_THEME_GIT_PROMPT_CLEAN="%{$fg[blue]%})"

ZSH_THEME_IVALDI_PROMPT_PREFIX="%{$fg_bold[magenta]%}ivaldi:(%{$fg[yellow]%}"
ZSH_THEME_IVALDI_PROMPT_SUFFIX="%{$fg[magenta]%})%{$reset_color%} "