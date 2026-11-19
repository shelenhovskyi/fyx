# Fyx shell integration: the user's .zprofile (see .zshenv).
__fyx_zdotdir="$ZDOTDIR"
ZDOTDIR="$FYX_USER_ZDOTDIR"
if [[ -f "$ZDOTDIR/.zprofile" ]]; then
  . "$ZDOTDIR/.zprofile"
fi
ZDOTDIR="$__fyx_zdotdir"
unset __fyx_zdotdir
