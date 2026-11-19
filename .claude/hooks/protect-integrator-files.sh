#!/usr/bin/env bash
# PreToolUse hook (Edit, Write, MultiEdit, NotebookEdit): denies edits of the integrator's files:
# the root Cargo.toml, any Cargo.lock and crates/ui_ext/wit/. A crate's own Cargo.toml is allowed.
# FYX_INTEGRATOR=1 in the session's env (.claude/settings.local.json "env") allows them, for W0
# and for a stream W0 hands "integrator lines" to.
# Never fails a tool call on its own error: any error exits 0 (= no decision). Pure bash.
trap 'exit 0' ERR
[[ "${FYX_INTEGRATOR:-}" == 1 ]] && exit 0
input=""
IFS= read -r -d '' input || true
[[ "$input" =~ \"(file_path|notebook_path)\"[[:space:]]*:[[:space:]]*\"(([^\"\\]|\\.)*)\" ]] || exit 0
file="${BASH_REMATCH[2]}"
file="${file//\\\\//}"
root="${CLAUDE_PROJECT_DIR:-}"
if [[ -z "$root" && "$input" =~ \"cwd\"[[:space:]]*:[[:space:]]*\"(([^\"\\]|\\.)*)\" ]]; then
    root="${BASH_REMATCH[1]}"
fi
root="${root//\\\\//}"
root="${root//\\//}"
root="${root%/}"
[[ -n "$root" ]] || exit 0
# Git Bash spells D:/x as /d/x.
if [[ "$root" =~ ^/([A-Za-z])(/.*)?$ ]]; then root="${BASH_REMATCH[1]}:${BASH_REMATCH[2]}"; fi
if [[ "$file" =~ ^/([A-Za-z])(/.*)?$ ]]; then file="${BASH_REMATCH[1]}:${BASH_REMATCH[2]}"; fi

# Windows paths compare case-insensitively (D:/Projects vs d:/projects).
shopt -s nocasematch
case "$file" in
    "$root"/*) relative="${file:${#root}+1}" ;;
    /* | [A-Za-z]:*) exit 0 ;;
    *) relative="$file" ;;
esac
relative="${relative#./}"
shopt -u nocasematch

case "$relative" in
    Cargo.toml | Cargo.lock | */Cargo.lock | crates/ui_ext/wit/*)
        printf '%s\n' "{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"$relative is integrator-only (W0, .claude/CLAUDE.md): list the change for the integrator in your progress file instead\"}}"
        ;;
esac
exit 0
