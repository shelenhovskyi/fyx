# Fyx shell integration for bash (plan 5.17), started as `bash --init-file <this file>`.
# Sources the user's own startup files first, then marks prompts and commands with OSC 633
# (A prompt, B prompt end, E command line with Fyx's nonce, C executed, D finished, P;Cwd). Only
# a shell Fyx started has the nonce; it is unset here, so programs never see it.

if [[ -n "$FYX_BASH_LOGIN" ]]; then
  unset FYX_BASH_LOGIN
  [[ -r /etc/profile ]] && . /etc/profile
  for __fyx_file in ~/.bash_profile ~/.bash_login ~/.profile; do
    if [[ -r "$__fyx_file" ]]; then
      . "$__fyx_file"
      break
    fi
  done
  unset __fyx_file
else
  [[ -r /etc/bash.bashrc ]] && . /etc/bash.bashrc
  [[ -r ~/.bashrc ]] && . ~/.bashrc
fi

if [[ $- == *i* && -n "$FYX_SHELL_NONCE" && -z "$__fyx_loaded" ]]; then
  __fyx_loaded=1
  __fyx_nonce="$FYX_SHELL_NONCE"
  unset FYX_SHELL_NONCE
  __fyx_in_command=0
  # Until the first prompt: the rest of this file must not count as a command.
  __fyx_prompting=1
  __fyx_status=0
  __fyx_user_ps1=""
  __fyx_wrapped_ps1=""

  # `\` as `\\`; `;` and control characters as `\xAB`.
  __fyx_escape() {
    local text="$1" out="" char code i
    for (( i = 0; i < ${#text}; i++ )); do
      char="${text:i:1}"
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
    builtin printf '%s' "$out"
  }

  __fyx_command_line() {
    local line
    line="$(HISTTIMEFORMAT='' builtin history 1)"
    if [[ "$line" =~ ^[[:space:]]*[0-9]+[*]?[[:space:]]+(.*)$ ]]; then
      line="${BASH_REMATCH[1]}"
    fi
    builtin printf '%s' "$line"
  }

  __fyx_preexec() {
    (( __fyx_in_command || __fyx_prompting )) && return
    __fyx_in_command=1
    builtin printf '\e]633;E;%s;%s\a\e]633;C\a' \
      "$(__fyx_escape "$(__fyx_command_line)")" "$__fyx_nonce"
  }

  __fyx_prompt_start() {
    __fyx_status=$?
    __fyx_prompting=1
  }

  __fyx_set_status() {
    return "$1"
  }

  __fyx_precmd() {
    if (( __fyx_in_command )); then
      builtin printf '\e]633;D;%s\a' "$__fyx_status"
      __fyx_in_command=0
    fi
    builtin printf '\e]633;P;Cwd=%s\a' "$(__fyx_escape "$PWD")"
    if [[ "$PS1" != "$__fyx_wrapped_ps1" ]]; then
      __fyx_user_ps1="$PS1"
    fi
    __fyx_wrapped_ps1='\[\e]633;A\a\]'"$__fyx_user_ps1"'\[\e]633;B\a\]'
    PS1="$__fyx_wrapped_ps1"
    __fyx_prompting=0
    __fyx_set_status "$__fyx_status"
  }

  if [[ -n "${bash_preexec_imported:-}" ]]; then
    # bash-preexec owns the DEBUG trap and PROMPT_COMMAND.
    precmd_functions=(__fyx_prompt_start "${precmd_functions[@]}" __fyx_precmd)
    preexec_functions+=(__fyx_preexec)
  else
    __fyx_debug_trap() {
      [[ -n "${COMP_LINE:-}" ]] && return
      [[ "$BASH_COMMAND" == __fyx_* ]] && return
      __fyx_preexec
    }
    if [[ -z "$(trap -p DEBUG)" ]]; then
      trap '__fyx_debug_trap' DEBUG
    fi
    if [[ "$(declare -p PROMPT_COMMAND 2>/dev/null)" == "declare -a"* ]]; then
      PROMPT_COMMAND=(__fyx_prompt_start '__fyx_set_status "$__fyx_status"' "${PROMPT_COMMAND[@]}" __fyx_precmd)
    else
      PROMPT_COMMAND='__fyx_prompt_start; __fyx_set_status "$__fyx_status"'"${PROMPT_COMMAND:+; $PROMPT_COMMAND}"'; __fyx_precmd'
    fi
  fi
fi
