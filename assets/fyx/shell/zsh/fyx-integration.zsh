# Fyx shell integration for zsh (plan 5.17). Sourced by Fyx's .zshrc after the user's own.
# Marks prompts and commands with OSC 633 (A prompt, B prompt end, E command line with Fyx's
# nonce, C executed, D finished, P;Cwd). Only a shell Fyx started has the nonce; it is unset
# here, so programs the shell runs never see it.

if [[ -o interactive && -n "$FYX_SHELL_NONCE" && -z "$__fyx_loaded" ]]; then
  typeset -g __fyx_loaded=1
  typeset -g __fyx_nonce="$FYX_SHELL_NONCE"
  unset FYX_SHELL_NONCE
  typeset -g __fyx_in_command=0
  typeset -g __fyx_user_ps1=""
  typeset -g __fyx_wrapped_ps1=""

  # `\` as `\\`; `;` and control characters as `\xAB`.
  __fyx_escape() {
    local text="$1" out="" char code
    local -i i
    for (( i = 1; i <= ${#text}; i++ )); do
      char="${text[i]}"
      if [[ "$char" == '\' ]]; then
        out+='\\'
      elif [[ "$char" == ';' ]]; then
        out+='\x3b'
      elif [[ "$char" == [[:cntrl:]] ]]; then
        printf -v code '\\x%02x' "'$char"
        out+="$code"
      else
        out+="$char"
      fi
    done
    print -rn -- "$out"
  }

  __fyx_precmd() {
    local code=$?
    if (( __fyx_in_command )); then
      printf '\e]633;D;%s\a' "$code"
      __fyx_in_command=0
    fi
    printf '\e]633;P;Cwd=%s\a' "$(__fyx_escape "$PWD")"
    # Themes may set PS1 in their own precmd: wrap what they left, once.
    if [[ "$PS1" != "$__fyx_wrapped_ps1" ]]; then
      __fyx_user_ps1="$PS1"
    fi
    __fyx_wrapped_ps1=$'%{\e]633;A\a%}'"$__fyx_user_ps1"$'%{\e]633;B\a%}'
    PS1="$__fyx_wrapped_ps1"
  }

  __fyx_preexec() {
    printf '\e]633;E;%s;%s\a\e]633;C\a' "$(__fyx_escape "$1")" "$__fyx_nonce"
    __fyx_in_command=1
  }

  autoload -Uz add-zsh-hook
  add-zsh-hook precmd __fyx_precmd
  add-zsh-hook preexec __fyx_preexec
fi
