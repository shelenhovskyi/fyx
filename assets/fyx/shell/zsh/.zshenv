# Fyx shell integration: Fyx points ZDOTDIR here and keeps the user's in FYX_USER_ZDOTDIR.
# Each file sources the user's own with their ZDOTDIR in place, then puts Fyx's back.
__fyx_zdotdir="$ZDOTDIR"
ZDOTDIR="${FYX_USER_ZDOTDIR:-$HOME}"
if [[ -f "$ZDOTDIR/.zshenv" ]]; then
  . "$ZDOTDIR/.zshenv"
fi
# The user's .zshenv may move ZDOTDIR; the later files come from there.
FYX_USER_ZDOTDIR="$ZDOTDIR"
ZDOTDIR="$__fyx_zdotdir"
unset __fyx_zdotdir
