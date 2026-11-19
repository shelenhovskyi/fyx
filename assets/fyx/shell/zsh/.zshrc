# Fyx shell integration: the user's .zshrc, then Fyx's marks (see .zshenv).
__fyx_zdotdir="$ZDOTDIR"
ZDOTDIR="$FYX_USER_ZDOTDIR"
if [[ -f "$ZDOTDIR/.zshrc" ]]; then
  . "$ZDOTDIR/.zshrc"
fi
. "$__fyx_zdotdir/fyx-integration.zsh"
if [[ -o login ]]; then
  # .zlogin comes next, from Fyx's dir; it restores the user's ZDOTDIR.
  ZDOTDIR="$__fyx_zdotdir"
else
  if [[ -n "$FYX_ZDOTDIR_WAS_SET" ]]; then
    export ZDOTDIR="$FYX_USER_ZDOTDIR"
  else
    unset ZDOTDIR
  fi
  unset FYX_USER_ZDOTDIR FYX_ZDOTDIR_WAS_SET
fi
unset __fyx_zdotdir
