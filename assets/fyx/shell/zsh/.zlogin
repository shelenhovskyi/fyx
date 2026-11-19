# Fyx shell integration: the user's .zlogin, then the user's ZDOTDIR for good (see .zshenv).
ZDOTDIR="$FYX_USER_ZDOTDIR"
if [[ -f "$ZDOTDIR/.zlogin" ]]; then
  . "$ZDOTDIR/.zlogin"
fi
if [[ -n "$FYX_ZDOTDIR_WAS_SET" ]]; then
  export ZDOTDIR="$FYX_USER_ZDOTDIR"
else
  unset ZDOTDIR
fi
unset FYX_USER_ZDOTDIR FYX_ZDOTDIR_WAS_SET
